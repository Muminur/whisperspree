//! Global hotkey boundary (T2.1).
//!
//! The rdev callback never touches session state: it forwards small [`KeyEvent`]
//! values to a manager loop. [`HotkeyManager`] decides whether a hotkey becomes a
//! session action and sends that action down the supplied `mpsc` channel. The
//! Tokio session owner is deliberately the consumer of that channel (§5.4).

use crate::pipeline::DictationRuntime;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Ignore a new session start for this long after the preceding one ended
/// (FR-1.3).
pub const DEBOUNCE: Duration = Duration::from_millis(250);
/// A speech-less PTT hold shorter than this is an accidental tap (FR-1.3).
pub const ACCIDENTAL_TAP: Duration = Duration::from_millis(200);

/// Small, platform-neutral subset of keys accepted by the v1 hotkey manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    AltRight,
    Escape,
    Space,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    /// Explicitly unsupported by FR-1.3.
    Fn,
}

/// A raw key edge received from the platform listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
}

impl KeyEvent {
    pub const fn down(key: Key) -> Self {
        Self { key, pressed: true }
    }

    pub const fn up(key: Key) -> Self {
        Self {
            key,
            pressed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyMode {
    PushToTalk,
    Toggle,
}

/// Validated configuration used by the input boundary. T2.6 owns Tauri tray
/// controls; it can use [`HotkeyManager::handle_toggle`] for the same action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyConfig {
    pub mode: HotkeyMode,
    pub push_to_talk_key: Key,
    pub toggle_combo: String,
    pub esc_cancels: bool,
}

impl HotkeyConfig {
    /// Convert the persisted §8.3 representation into the validated runtime
    /// policy. This is the single translation point used by live settings
    /// rebinding, so invalid values cannot partially replace an active binding.
    pub fn from_wire(
        mode: &str,
        push_to_talk_key: &str,
        toggle_combo: &str,
        esc_cancels: bool,
    ) -> Result<Self, HotkeyError> {
        let mode = match mode {
            "push_to_talk" => HotkeyMode::PushToTalk,
            "toggle" => HotkeyMode::Toggle,
            _ => return Err(HotkeyError::EmptyToggleCombo),
        };
        let push_to_talk_key = parse_key(push_to_talk_key)?;
        let config = Self {
            mode,
            push_to_talk_key,
            toggle_combo: toggle_combo.to_owned(),
            esc_cancels,
        };
        validate_config(&config)?;
        Ok(config)
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            mode: HotkeyMode::PushToTalk,
            push_to_talk_key: Key::AltRight,
            toggle_combo: "Ctrl+Alt+Space".to_string(),
            esc_cancels: true,
        }
    }
}

/// Input Monitoring is the capability that gates OS-level hotkey registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
}

/// Messages for the session task. The session task maps these directly to its
/// `hotkey_down`, `release`, and `escape` transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Start,
    Stop,
    Cancel,
    /// A short, speech-less PTT press; no error/HUD/history side effects.
    CancelSilent,
}

/// Commands delivered by the settings/permission/session owners to the live
/// hotkey manager thread. Keeping these on the manager's channel makes
/// rebinding and permission changes take effect without replacing the rdev
/// listener or touching policy from the raw callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyControl {
    Rebind(HotkeyConfig),
    SetPermission(PermissionState),
    SessionBecameIdle(Instant),
    Toggle(Instant),
    SpeechObserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    FnKeyUnsupported,
    EmptyToggleCombo,
    ManagerStopped,
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FnKeyUnsupported => f.write_str("the fn key is not supported as a hotkey"),
            Self::EmptyToggleCombo => f.write_str("toggle shortcut must not be empty"),
            Self::ManagerStopped => f.write_str("hotkey manager is stopped"),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// Stateful policy object. It is intended to be owned by exactly one manager
/// thread; no lock is needed in the rdev callback or the session task.
pub struct HotkeyManager {
    config: HotkeyConfig,
    permission: PermissionState,
    sender: Sender<HotkeyAction>,
    phase: SessionPhase,
    ptt_started_at: Option<Instant>,
    speech_observed: bool,
    speech_signal: Arc<AtomicBool>,
    active_ptt_key: Option<Key>,
    last_session_end: Option<Instant>,
}

/// Handle retained by settings, onboarding, and the session owner to update
/// the live manager without restarting the raw rdev listener.
#[derive(Clone)]
pub struct HotkeyController {
    controls: Sender<HotkeyControlMessage>,
    speech_signal: Arc<AtomicBool>,
}

impl HotkeyController {
    pub fn send(&self, control: HotkeyControl) -> Result<(), HotkeyError> {
        self.controls
            .send(HotkeyControlMessage::Control(control))
            .map_err(|_| HotkeyError::ManagerStopped)
    }

    /// Synchronous VAD feedback used by the sub-200 ms accidental-tap
    /// decision. This atomic avoids racing the rdev manager channel at key-up.
    pub fn mark_speech_observed(&self) {
        self.speech_signal.store(true, Ordering::Release);
    }
}

/// The hotkey boundary must remain armed only after the session task confirms
/// that it is truly idle. In particular, key release enters `AwaitingIdle`, not
/// `Idle`: Finalizing may still own microphone/audio resources (SM-1/SM-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionPhase {
    Idle,
    Active,
    AwaitingIdle,
}

impl HotkeyManager {
    pub fn new(
        config: HotkeyConfig,
        permission: PermissionState,
        sender: Sender<HotkeyAction>,
    ) -> Self {
        Self::new_with_signal(config, permission, sender, Arc::new(AtomicBool::new(false)))
    }

    fn new_with_signal(
        config: HotkeyConfig,
        permission: PermissionState,
        sender: Sender<HotkeyAction>,
        speech_signal: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            permission,
            sender,
            phase: SessionPhase::Idle,
            ptt_started_at: None,
            speech_observed: false,
            speech_signal,
            active_ptt_key: None,
            last_session_end: None,
        }
    }

    /// Update the binding immediately after settings persistence. Validation is
    /// performed before replacing the existing config, so a rejected setting
    /// leaves the previous live binding intact (FR-1.3).
    pub fn rebind(&mut self, config: HotkeyConfig) -> Result<(), HotkeyError> {
        validate_config(&config)?;
        self.config = config;
        Ok(())
    }

    /// Update permission status as onboarding detects a grant/revocation. When
    /// unavailable, hotkeys simply emit nothing; callers can expose their
    /// persistent degraded warning without starting a session.
    pub fn set_permission(&mut self, permission: PermissionState) {
        self.permission = permission;
        if permission != PermissionState::Granted && self.phase == SessionPhase::Active {
            self.emit(HotkeyAction::Cancel);
            self.finish();
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.permission != PermissionState::Granted
    }

    pub fn config(&self) -> &HotkeyConfig {
        &self.config
    }

    /// Apply a control-plane update on the manager owner thread. Invalid
    /// rebinding leaves the active configuration untouched; the caller can
    /// surface the returned validation error through the settings IPC path.
    pub fn apply_control(&mut self, control: HotkeyControl) -> Result<(), HotkeyError> {
        match control {
            HotkeyControl::Rebind(config) => self.rebind(config),
            HotkeyControl::SetPermission(permission) => {
                self.set_permission(permission);
                Ok(())
            }
            HotkeyControl::SessionBecameIdle(now) => {
                self.session_became_idle(now);
                Ok(())
            }
            HotkeyControl::Toggle(now) => {
                self.handle_toggle(now);
                Ok(())
            }
            HotkeyControl::SpeechObserved => {
                self.speech_observed = true;
                Ok(())
            }
        }
    }

    /// Notify the hotkey boundary after the session coordinator has reached
    /// `Idle`. This is deliberately not inferred from a physical key release:
    /// the release only starts Finalizing, whereas FR-1.3's debounce is measured
    /// from the end of the previous session.
    pub fn session_became_idle(&mut self, now: Instant) {
        self.phase = SessionPhase::Idle;
        self.last_session_end = Some(now);
    }

    /// Handle a raw PTT/Escape edge. This method is deterministic given `now`,
    /// keeping the debounce and accidental-tap rules testable without sleeping.
    pub fn handle(&mut self, event: KeyEvent, now: Instant) {
        if self.is_degraded() {
            return;
        }

        if event.pressed && event.key == Key::Escape {
            if self.config.esc_cancels && self.phase == SessionPhase::Active {
                self.emit(HotkeyAction::Cancel);
                self.finish();
            }
            return;
        }

        if self.config.mode != HotkeyMode::PushToTalk {
            return;
        }

        if event.pressed && event.key == self.config.push_to_talk_key {
            if self.phase == SessionPhase::Idle && !self.in_debounce(now) {
                self.phase = SessionPhase::Active;
                self.ptt_started_at = Some(now);
                self.speech_signal.store(false, Ordering::Release);
                self.active_ptt_key = Some(event.key);
                self.emit(HotkeyAction::Start);
            }
        } else if !event.pressed
            && self.phase == SessionPhase::Active
            && self.active_ptt_key == Some(event.key)
        {
            let accidental = !self.speech_observed
                && !self.speech_signal.load(Ordering::Acquire)
                && self
                    .ptt_started_at
                    .is_some_and(|started| now.saturating_duration_since(started) < ACCIDENTAL_TAP);
            self.emit(if accidental {
                HotkeyAction::CancelSilent
            } else {
                HotkeyAction::Stop
            });
            self.finish();
        }
    }

    /// Receive an activation from the OS global-shortcut plugin (or from the
    /// tray) in toggle mode. The plugin owns OS registration; this manager owns
    /// the session/debounce semantics shared by all sources.
    pub fn handle_toggle(&mut self, now: Instant) {
        if self.is_degraded() || self.config.mode != HotkeyMode::Toggle {
            return;
        }
        if self.phase == SessionPhase::Active {
            self.emit(HotkeyAction::Stop);
            self.finish();
        } else if self.phase == SessionPhase::Idle && !self.in_debounce(now) {
            self.phase = SessionPhase::Active;
            self.emit(HotkeyAction::Start);
        }
    }

    fn in_debounce(&self, now: Instant) -> bool {
        self.last_session_end
            .is_some_and(|ended| now.saturating_duration_since(ended) < DEBOUNCE)
    }

    fn emit(&self, action: HotkeyAction) {
        // A closed receiver means the app/session task is shutting down. Input
        // callbacks must not panic or block while shutdown is in progress.
        if let Err(error) = self.sender.send(action) {
            tracing::warn!(
                code = "HK-PERM",
                ?error,
                "hotkey action receiver unavailable"
            );
        }
    }

    fn finish(&mut self) {
        self.phase = SessionPhase::AwaitingIdle;
        self.ptt_started_at = None;
        self.speech_observed = false;
        self.speech_signal.store(false, Ordering::Release);
        self.active_ptt_key = None;
        // Do not set `last_session_end` here. A key release/cancel only signals
        // the session task; `session_became_idle` establishes the real boundary.
    }
}

fn validate_config(config: &HotkeyConfig) -> Result<(), HotkeyError> {
    if config.push_to_talk_key == Key::Fn {
        return Err(HotkeyError::FnKeyUnsupported);
    }
    if config.toggle_combo.trim().is_empty() {
        return Err(HotkeyError::EmptyToggleCombo);
    }
    Ok(())
}

fn parse_key(key: &str) -> Result<Key, HotkeyError> {
    let key = match key {
        "AltRight" => Key::AltRight,
        "Escape" => Key::Escape,
        "Space" => Key::Space,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "Fn" => Key::Fn,
        _ => return Err(HotkeyError::FnKeyUnsupported),
    };
    Ok(key)
}

/// Spawn the rdev source and a separate manager thread. The callback only sends
/// a compact event across `mpsc`; all policy and session-action dispatch happen
/// off the raw listener callback, satisfying §5.4's "never does work inline"
/// constraint.
pub fn spawn_rdev_listener(
    config: HotkeyConfig,
    permission: PermissionState,
    action_sender: Sender<HotkeyAction>,
) -> thread::JoinHandle<()> {
    let (_, manager) = spawn_rdev_listener_with_control(config, permission, action_sender);
    manager
}

/// Consume policy actions off the listener/manager channels and dispatch them
/// through the same runtime boundary used by IPC and the tray. The worker is
/// deliberately separate from rdev and never runs session work in a raw input
/// callback.
pub fn spawn_action_bridge(
    runtime: Arc<dyn DictationRuntime>,
    actions: Receiver<HotkeyAction>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("whisperspree-hotkey-actions".into())
        .spawn(move || {
            while let Ok(action) = actions.recv() {
                let result = match action {
                    HotkeyAction::Start => runtime.start_dictation(),
                    HotkeyAction::Stop => runtime.stop_dictation(),
                    HotkeyAction::Cancel => runtime.cancel_dictation(),
                    HotkeyAction::CancelSilent => runtime.cancel_silent_dictation(),
                };
                if let Err(error) = result {
                    tracing::warn!(code = %error.code(), %error, ?action, "hotkey action dispatch failed");
                }
            }
        })
        .expect("hotkey action bridge thread must start")
}

/// Spawn the rdev source plus a control-plane handle. Raw events and control
/// updates are serialized on the manager owner thread, preserving the
/// callback's no-work/no-lock boundary while allowing live rebinding.
pub fn spawn_rdev_listener_with_control(
    config: HotkeyConfig,
    permission: PermissionState,
    action_sender: Sender<HotkeyAction>,
) -> (HotkeyController, thread::JoinHandle<()>) {
    let (message_sender, message_receiver) = mpsc::channel();
    let speech_signal = Arc::new(AtomicBool::new(false));
    let controller = HotkeyController {
        controls: message_sender.clone(),
        speech_signal: Arc::clone(&speech_signal),
    };
    let manager = thread::spawn(move || {
        run_manager(
            config,
            permission,
            action_sender,
            message_receiver,
            speech_signal,
        )
    });

    let permission_sender = message_sender.clone();
    thread::spawn(move || {
        if let Err(error) = rdev::listen(move |event| {
            if let Some(event) = rdev_key_event(event) {
                if let Err(error) = message_sender.send(HotkeyControlMessage::Event(event)) {
                    tracing::warn!(
                        code = "HK-PERM",
                        ?error,
                        "hotkey manager unavailable for raw event"
                    );
                }
            }
        }) {
            tracing::warn!(
                code = "HK-PERM",
                ?error,
                "global hotkey listener registration failed"
            );
            if let Err(send_error) = permission_sender.send(HotkeyControlMessage::Control(
                HotkeyControl::SetPermission(PermissionState::Denied),
            )) {
                tracing::warn!(
                    code = "HK-PERM",
                    ?send_error,
                    "hotkey manager unavailable after listener failure"
                );
            }
        }
    });

    (controller, manager)
}

enum HotkeyControlMessage {
    Event(KeyEvent),
    Control(HotkeyControl),
}

fn run_manager(
    config: HotkeyConfig,
    permission: PermissionState,
    action_sender: Sender<HotkeyAction>,
    events: Receiver<HotkeyControlMessage>,
    speech_signal: Arc<AtomicBool>,
) {
    let mut manager =
        HotkeyManager::new_with_signal(config, permission, action_sender, speech_signal);
    while let Ok(message) = events.recv() {
        match message {
            HotkeyControlMessage::Event(event) => manager.handle(event, Instant::now()),
            HotkeyControlMessage::Control(control) => {
                if let Err(error) = manager.apply_control(control) {
                    tracing::warn!(code = "HK-PERM", ?error, "hotkey control update failed");
                }
            }
        }
    }
}

fn rdev_key_event(event: rdev::Event) -> Option<KeyEvent> {
    let (raw_key, pressed) = match event.event_type {
        rdev::EventType::KeyPress(key) => (key, true),
        rdev::EventType::KeyRelease(key) => (key, false),
        _ => return None,
    };
    let key = match raw_key {
        // rdev maps macOS's right Option virtual key to AltGr; the left key is
        // Alt and therefore deliberately does not activate the default PTT.
        rdev::Key::AltGr => Key::AltRight,
        rdev::Key::Escape => Key::Escape,
        rdev::Key::F1 => Key::F1,
        rdev::Key::F2 => Key::F2,
        rdev::Key::F3 => Key::F3,
        rdev::Key::F4 => Key::F4,
        rdev::Key::F5 => Key::F5,
        rdev::Key::F6 => Key::F6,
        rdev::Key::F7 => Key::F7,
        rdev::Key::F8 => Key::F8,
        rdev::Key::F9 => Key::F9,
        rdev::Key::F10 => Key::F10,
        rdev::Key::F11 => Key::F11,
        rdev::Key::F12 => Key::F12,
        _ => return None,
    };
    Some(KeyEvent { key, pressed })
}

#[cfg(test)]
mod debounce {
    use super::*;

    #[test]
    fn fr_1_3_debounce_begins_at_confirmed_idle() {
        let (tx, rx) = mpsc::channel();
        let mut manager = HotkeyManager::new(HotkeyConfig::default(), PermissionState::Granted, tx);
        let start = Instant::now();
        manager.handle(KeyEvent::down(Key::AltRight), start);
        manager.handle(KeyEvent::up(Key::AltRight), start + ACCIDENTAL_TAP);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![HotkeyAction::Start, HotkeyAction::Stop]
        );
        manager.session_became_idle(start + ACCIDENTAL_TAP);
        manager.handle(
            KeyEvent::down(Key::AltRight),
            start + ACCIDENTAL_TAP + DEBOUNCE,
        );
        assert_eq!(rx.recv().unwrap(), HotkeyAction::Start);
    }
}
