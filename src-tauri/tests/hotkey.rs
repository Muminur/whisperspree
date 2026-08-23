use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use whisperspree_lib::hotkey::{
    HotkeyAction, HotkeyConfig, HotkeyControl, HotkeyManager, HotkeyMode, Key, KeyEvent,
    PermissionState, ACCIDENTAL_TAP, DEBOUNCE,
};
use whisperspree_lib::pipeline::DictationRuntime;

fn manager() -> (HotkeyManager, mpsc::Receiver<HotkeyAction>) {
    let (sender, receiver) = mpsc::channel();
    (
        HotkeyManager::new(HotkeyConfig::default(), PermissionState::Granted, sender),
        receiver,
    )
}

struct RecordingRuntime {
    actions: Mutex<Vec<&'static str>>,
}

impl DictationRuntime for RecordingRuntime {
    fn start_dictation(&self) -> Result<(), whisperspree_lib::error::Error> {
        self.actions.lock().unwrap().push("start");
        Ok(())
    }

    fn stop_dictation(&self) -> Result<(), whisperspree_lib::error::Error> {
        self.actions.lock().unwrap().push("stop");
        Ok(())
    }

    fn cancel_dictation(&self) -> Result<(), whisperspree_lib::error::Error> {
        self.actions.lock().unwrap().push("cancel");
        Ok(())
    }

    fn cancel_silent_dictation(&self) -> Result<(), whisperspree_lib::error::Error> {
        Ok(())
    }
}

#[test]
fn fr_1_3_action_bridge_routes_hotkey_actions_to_shared_runtime() {
    let (actions, receiver) = mpsc::channel();
    let runtime = Arc::new(RecordingRuntime {
        actions: Mutex::new(Vec::new()),
    });
    let worker = whisperspree_lib::hotkey::spawn_action_bridge(runtime.clone(), receiver);
    actions.send(HotkeyAction::Start).unwrap();
    actions.send(HotkeyAction::Stop).unwrap();
    actions.send(HotkeyAction::Cancel).unwrap();
    drop(actions);
    worker.join().unwrap();
    assert_eq!(
        *runtime.actions.lock().unwrap(),
        vec!["start", "stop", "cancel"]
    );
}

#[test]
fn fr_1_3_accidental_tap_cancel_is_silent_through_action_bridge() {
    let (actions, receiver) = mpsc::channel();
    let runtime = Arc::new(RecordingRuntime {
        actions: Mutex::new(Vec::new()),
    });
    let worker = whisperspree_lib::hotkey::spawn_action_bridge(runtime.clone(), receiver);
    actions.send(HotkeyAction::CancelSilent).unwrap();
    drop(actions);
    worker.join().unwrap();
    assert!(runtime.actions.lock().unwrap().is_empty());
}

#[test]
fn fr_1_3_persisted_hotkey_settings_convert_to_validated_live_config() {
    let config = HotkeyConfig::from_wire("toggle", "F8", "Ctrl+F8", false).unwrap();
    assert_eq!(config.mode, HotkeyMode::Toggle);
    assert_eq!(config.push_to_talk_key, Key::F8);
    assert!(HotkeyConfig::from_wire("push_to_talk", "Fn", "Ctrl+Space", true).is_err());
}

#[test]
fn fr_1_3_short_release_after_speech_is_stop_not_silent_cancel() {
    let (mut manager, receiver) = manager();
    let start = Instant::now();
    manager.handle(KeyEvent::down(Key::AltRight), start);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager
        .apply_control(HotkeyControl::SpeechObserved)
        .unwrap();
    manager.handle(
        KeyEvent::up(Key::AltRight),
        start + Duration::from_millis(100),
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);
}

#[test]
fn fr_1_3_permission_revocation_cancels_active_session_and_releases_phase() {
    let (mut manager, receiver) = manager();
    let start = Instant::now();
    manager.handle(KeyEvent::down(Key::AltRight), start);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.set_permission(PermissionState::Denied);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Cancel);
    manager.set_permission(PermissionState::Granted);
    manager.session_became_idle(start + Duration::from_millis(500));
    manager.handle(
        KeyEvent::down(Key::AltRight),
        start + Duration::from_millis(800),
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
}

#[test]
fn debounce_ignores_ptt_down_for_250ms_after_session_end() {
    let (mut manager, receiver) = manager();
    let began = Instant::now();

    manager.handle(KeyEvent::down(Key::AltRight), began);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::up(Key::AltRight), began + ACCIDENTAL_TAP);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);
    manager.session_became_idle(began + ACCIDENTAL_TAP);

    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + ACCIDENTAL_TAP + DEBOUNCE - Duration::from_millis(1),
    );
    assert!(receiver.try_recv().is_err());

    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + ACCIDENTAL_TAP + DEBOUNCE,
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
}

#[test]
fn debounce_begins_when_the_session_reports_idle_not_at_raw_key_release() {
    let (mut manager, receiver) = manager();
    let began = Instant::now();

    manager.handle(KeyEvent::down(Key::AltRight), began);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::up(Key::AltRight), began + ACCIDENTAL_TAP);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);

    // The key has been up for much longer than the debounce period, but the
    // session is still Finalizing. It must not be possible to start a second
    // session before the coordinator has actually reached Idle.
    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + Duration::from_secs(1),
    );
    assert!(receiver.try_recv().is_err());

    manager.session_became_idle(began + Duration::from_secs(1));
    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + Duration::from_secs(1) + DEBOUNCE - Duration::from_millis(1),
    );
    assert!(receiver.try_recv().is_err());
    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + Duration::from_secs(1) + DEBOUNCE,
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
}

#[test]
fn ignored_press_while_finalizing_does_not_later_emit_a_spurious_stop() {
    let (mut manager, receiver) = manager();
    let began = Instant::now();

    manager.handle(KeyEvent::down(Key::AltRight), began);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::up(Key::AltRight), began + ACCIDENTAL_TAP);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);

    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + Duration::from_secs(1),
    );
    manager.session_became_idle(began + Duration::from_secs(1));
    manager.handle(
        KeyEvent::up(Key::AltRight),
        began + Duration::from_secs(1) + DEBOUNCE,
    );
    assert!(receiver.try_recv().is_err());

    manager.handle(
        KeyEvent::down(Key::AltRight),
        began + Duration::from_secs(1) + DEBOUNCE,
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
}

#[test]
fn accidental_ptt_tap_cancels_silently_before_200ms() {
    let (mut manager, receiver) = manager();
    let began = Instant::now();

    manager.handle(KeyEvent::down(Key::AltRight), began);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(
        KeyEvent::up(Key::AltRight),
        began + ACCIDENTAL_TAP - Duration::from_millis(1),
    );

    assert_eq!(receiver.recv().unwrap(), HotkeyAction::CancelSilent);
}

#[test]
fn ptt_release_after_200ms_stops_the_session() {
    let (mut manager, receiver) = manager();
    let began = Instant::now();

    manager.handle(KeyEvent::down(Key::AltRight), began);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::up(Key::AltRight), began + ACCIDENTAL_TAP);

    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);
}

#[test]
fn escape_cancels_only_an_active_ptt_session_when_enabled() {
    let (mut manager, receiver) = manager();
    let now = Instant::now();

    manager.handle(KeyEvent::down(Key::Escape), now);
    assert!(receiver.try_recv().is_err());

    manager.handle(KeyEvent::down(Key::AltRight), now);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::down(Key::Escape), now + Duration::from_millis(1));
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Cancel);
}

#[test]
fn toggle_binding_starts_and_stops_without_ptt_events() {
    let (sender, receiver) = mpsc::channel();
    let config = HotkeyConfig {
        mode: HotkeyMode::Toggle,
        ..HotkeyConfig::default()
    };
    let mut manager = HotkeyManager::new(config, PermissionState::Granted, sender);
    let now = Instant::now();

    manager.handle_toggle(now);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle_toggle(now + Duration::from_secs(1));
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);
}

#[test]
fn rebinding_ptt_applies_live_and_releases_the_old_binding() {
    let (mut manager, receiver) = manager();
    let now = Instant::now();

    manager
        .rebind(HotkeyConfig {
            push_to_talk_key: Key::F9,
            ..HotkeyConfig::default()
        })
        .unwrap();
    manager.handle(KeyEvent::down(Key::AltRight), now);
    assert!(receiver.try_recv().is_err());
    manager.handle(KeyEvent::down(Key::F9), now);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
    manager.handle(KeyEvent::up(Key::F9), now + ACCIDENTAL_TAP);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Stop);
}

#[test]
fn input_monitoring_denied_degrades_without_emitting_actions() {
    let (sender, receiver) = mpsc::channel();
    let mut manager = HotkeyManager::new(HotkeyConfig::default(), PermissionState::Denied, sender);

    manager.handle(KeyEvent::down(Key::AltRight), Instant::now());
    assert!(receiver.try_recv().is_err());
    assert!(manager.is_degraded());
}

#[test]
fn fn_cannot_be_bound_as_ptt() {
    let (mut manager, _) = manager();
    let result = manager.rebind(HotkeyConfig {
        push_to_talk_key: Key::Fn,
        ..HotkeyConfig::default()
    });

    assert!(result.is_err());
}

#[test]
fn runtime_control_commands_rebind_permission_and_idle_boundary() {
    let (sender, receiver) = mpsc::channel();
    let mut manager = HotkeyManager::new(HotkeyConfig::default(), PermissionState::Granted, sender);
    let now = Instant::now();

    manager
        .apply_control(whisperspree_lib::hotkey::HotkeyControl::Rebind(
            HotkeyConfig {
                push_to_talk_key: Key::F8,
                ..HotkeyConfig::default()
            },
        ))
        .unwrap();
    manager.handle(KeyEvent::down(Key::AltRight), now);
    assert!(receiver.try_recv().is_err());
    manager.handle(KeyEvent::down(Key::F8), now);
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);

    manager
        .apply_control(whisperspree_lib::hotkey::HotkeyControl::SetPermission(
            PermissionState::Denied,
        ))
        .unwrap();
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Cancel);
    manager.handle(KeyEvent::up(Key::F8), now + ACCIDENTAL_TAP);
    assert!(receiver.try_recv().is_err());

    manager
        .apply_control(whisperspree_lib::hotkey::HotkeyControl::SetPermission(
            PermissionState::Granted,
        ))
        .unwrap();
    manager
        .apply_control(whisperspree_lib::hotkey::HotkeyControl::SessionBecameIdle(
            now + Duration::from_secs(1),
        ))
        .unwrap();
    manager.handle(
        KeyEvent::down(Key::F8),
        now + Duration::from_secs(1) + DEBOUNCE,
    );
    assert_eq!(receiver.recv().unwrap(), HotkeyAction::Start);
}
