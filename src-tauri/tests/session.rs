use whisperspree_lib::{
    error::Error,
    pipeline::{
        AppContext, ContextDetector, CoordinatorEvent, CoordinatorRuntime, DictationRuntime,
        InjectionMethod, Microphone, SessionCoordinator, SessionEvent, SessionState,
        SessionTransition,
    },
};

#[derive(Default)]
struct TestMicrophone {
    opens: usize,
    closes: usize,
    fail_close: bool,
}

impl Microphone for TestMicrophone {
    fn open(&mut self) -> Result<(), Error> {
        self.opens += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<(), Error> {
        self.closes += 1;
        if self.fail_close {
            return Err(Error::MicDev("close failed".into()));
        }
        Ok(())
    }
}

struct TestContextDetector(Vec<AppContext>);

struct FailingContextDetector;

impl ContextDetector for FailingContextDetector {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        Err(Error::AxPerm("frontmost snapshot unavailable".into()))
    }
}

struct StartThenFailContextDetector {
    start: AppContext,
}

impl ContextDetector for StartThenFailContextDetector {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        if self.start.bundle_id.is_empty() {
            Err(Error::AxPerm("frontmost snapshot unavailable".into()))
        } else {
            let start = std::mem::replace(
                &mut self.start,
                AppContext {
                    bundle_id: String::new(),
                    title: String::new(),
                    secure_input: false,
                },
            );
            Ok(start)
        }
    }
}

impl ContextDetector for TestContextDetector {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        if self.0.len() > 1 {
            Ok(self.0.remove(0))
        } else {
            Ok(self
                .0
                .first()
                .cloned()
                .ok_or_else(|| Error::DbIo("missing test context".into()))?)
        }
    }
}

fn context(bundle_id: &str, title: &str, secure_input: bool) -> AppContext {
    AppContext {
        bundle_id: bundle_id.into(),
        title: title.into(),
        secure_input,
    }
}

#[test]
fn fr_1_1_context_probe_failure_degrades_to_unknown_context_and_starts_session() {
    let runtime = CoordinatorRuntime::new(TestMicrophone::default(), FailingContextDetector);

    let session_id = runtime
        .start_control_with_id()
        .expect("context metadata is advisory");
    assert_eq!(session_id, "session-1");
    assert_eq!(runtime.state().unwrap(), SessionState::Listening);
}

#[test]
fn sm_5_injection_context_failure_falls_back_to_clipboard_only() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        StartThenFailContextDetector {
            start: context("com.example.editor", "Draft", false),
        },
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.release().unwrap();
    coordinator.finalized().unwrap();
    coordinator.take_events();

    assert_eq!(coordinator.processed().unwrap(), SessionState::Injecting);
    assert!(matches!(
        coordinator.take_events().as_slice(),
        [
            CoordinatorEvent::State(SessionState::Injecting),
            CoordinatorEvent::InjectionReady {
                method: InjectionMethod::ClipboardOnly,
                target: AppContext { bundle_id, .. },
                ..
            }
        ] if bundle_id == "unknown"
    ));
}

#[test]
fn sm_1_duplicate_hotkey_is_ignored_and_escape_cancels_listening() {
    let mut s = SessionTransition::new();
    assert_eq!(
        s.apply(SessionEvent::HotkeyDown).unwrap(),
        SessionState::Listening
    );
    assert_eq!(
        s.apply(SessionEvent::HotkeyDown).unwrap(),
        SessionState::Listening
    );
    assert_eq!(
        s.apply(SessionEvent::Escape).unwrap(),
        SessionState::Cancelled
    );
}

#[test]
fn illegal_transition_does_not_mutate_state() {
    let mut s = SessionTransition::new();
    assert!(s.apply(SessionEvent::InjectDone).is_err());
    assert_eq!(s.state(), SessionState::Idle);
}

#[test]
fn fr_1_1_ipc_and_hotkeys_share_one_runtime_state_machine_for_start_stop_cancel() {
    let runtime = CoordinatorRuntime::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );

    runtime.start_dictation().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Listening);
    assert_eq!(
        runtime.take_events().unwrap(),
        vec![CoordinatorEvent::State(SessionState::Listening)]
    );

    runtime.stop_dictation().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Finalizing);
    assert_eq!(
        runtime.take_events().unwrap(),
        vec![CoordinatorEvent::State(SessionState::Finalizing)]
    );
    assert!(
        runtime.cancel_dictation().is_err(),
        "cancel is only valid while listening"
    );
}

#[test]
fn fr_1_1_runtime_control_can_reset_terminal_states_before_restart() {
    let runtime = CoordinatorRuntime::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );

    runtime.start_dictation().unwrap();
    runtime.cancel_dictation().unwrap();
    runtime.reset_control().unwrap();
    runtime.start_dictation().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Listening);
}

#[test]
fn fr_1_1_coordinator_returns_authoritative_session_id() {
    let runtime = CoordinatorRuntime::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    assert_eq!(runtime.start_control_with_id().unwrap(), "session-1");
    assert_eq!(runtime.state().unwrap(), SessionState::Listening);
}

#[test]
fn sm_2_runtime_failure_enters_error_and_can_reset_to_idle() {
    let runtime = CoordinatorRuntime::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );

    runtime.start_dictation().unwrap();
    runtime.failed_control().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Error);
    runtime.reset_control().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Idle);
}

#[test]
fn fr_1_1_runtime_stop_is_restartable_after_terminal_cleanup() {
    let runtime = CoordinatorRuntime::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    runtime.start_dictation().unwrap();
    runtime.stop_dictation().unwrap();
    runtime.reset_control().unwrap();
    runtime.start_dictation().unwrap();
    assert_eq!(runtime.state().unwrap(), SessionState::Listening);
}

#[test]
fn sm_2_microphone_opens_at_start_stays_open_for_tail_and_closes_at_idle() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );

    coordinator.hotkey_down("session-1").unwrap();
    assert_eq!(coordinator.microphone().opens, 1);
    assert!(coordinator.microphone_is_open());

    coordinator.release().unwrap();
    assert!(
        coordinator.microphone_is_open(),
        "the tail must be captured while finalizing"
    );
    coordinator.finalized().unwrap();
    coordinator.processed().unwrap();
    coordinator.inject_done().unwrap();

    assert_eq!(coordinator.microphone().closes, 1);
    assert!(!coordinator.microphone_is_open());
}

#[test]
fn sm_2_escape_closes_microphone_before_cancelled_state_event() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.take_events();

    coordinator.escape().unwrap();

    assert_eq!(coordinator.microphone().closes, 1);
    assert_eq!(
        coordinator.take_events(),
        vec![CoordinatorEvent::State(SessionState::Cancelled)]
    );
}

#[test]
fn cancel_reset_clears_session_and_allows_next_dictation() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.escape().unwrap();
    assert!(coordinator.session_context().is_some());
    coordinator.reset().unwrap();
    assert_eq!(coordinator.state(), SessionState::Idle);
    assert!(coordinator.session_context().is_none());
    coordinator.hotkey_down("session-2").unwrap();
    assert_eq!(coordinator.state(), SessionState::Listening);
}

#[test]
fn reset_is_monotonic_even_when_microphone_close_fails() {
    let microphone = TestMicrophone {
        fail_close: true,
        ..TestMicrophone::default()
    };
    let mut coordinator = SessionCoordinator::new(
        microphone,
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.escape().unwrap_err();
    assert_eq!(coordinator.state(), SessionState::Cancelled);
    coordinator.reset().unwrap_err();
    assert_eq!(coordinator.state(), SessionState::Idle);
    assert!(coordinator.session_context().is_none());
}

#[test]
fn sm_4_release_retains_at_most_300ms_of_16khz_tail_audio() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![context("com.example.editor", "Draft", false)]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.take_events();
    coordinator.release().unwrap();

    assert_eq!(coordinator.feed_tail(&vec![0.25; 5_000]).unwrap(), 4_800);
    assert_eq!(coordinator.tail_samples(), 4_800);
    assert_eq!(
        coordinator.take_events(),
        vec![
            CoordinatorEvent::State(SessionState::Finalizing),
            CoordinatorEvent::TailAudioAccepted { samples: 4_800 },
        ]
    );
}

#[test]
fn sm_5_injection_uses_current_target_start_style_and_secure_clipboard_fallback() {
    let start = context("com.example.mail", "Compose", false);
    let target = context("com.example.passwords", "Sign in", true);
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![start.clone(), target.clone()]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.release().unwrap();
    coordinator.finalized().unwrap();
    coordinator.take_events();

    coordinator.processed().unwrap();

    assert_eq!(
        coordinator.take_events(),
        vec![
            CoordinatorEvent::State(SessionState::Injecting),
            CoordinatorEvent::InjectionReady {
                target,
                style: start,
                method: InjectionMethod::ClipboardOnly,
                persist_history: false,
            },
        ]
    );
}

#[test]
fn sm_5_secure_outcome_suppresses_history_flag() {
    let start = context("com.example.mail", "Compose", false);
    let target = context("com.example.passwords", "Sign in", true);
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![start.clone(), target.clone()]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.release().unwrap();
    coordinator.finalized().unwrap();
    coordinator.take_events();

    coordinator.processed().unwrap();

    assert!(matches!(
        coordinator.take_events().as_slice(),
        [
            CoordinatorEvent::State(SessionState::Injecting),
            CoordinatorEvent::InjectionReady {
                method: InjectionMethod::ClipboardOnly,
                persist_history: false,
                ..
            }
        ]
    ));
}

#[test]
fn sm_5_normal_outcome_keeps_history_flag() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![
            context("com.example.editor", "Draft", false),
            context("com.example.browser", "Page", false),
        ]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.release().unwrap();
    coordinator.finalized().unwrap();
    coordinator.take_events();

    coordinator.processed().unwrap();

    assert!(matches!(
        coordinator.take_events().as_slice(),
        [
            CoordinatorEvent::State(SessionState::Injecting),
            CoordinatorEvent::InjectionReady {
                method: InjectionMethod::Normal,
                persist_history: true,
                ..
            }
        ]
    ));
}

#[test]
fn sm_3_state_event_precedes_destination_injection_event() {
    let mut coordinator = SessionCoordinator::new(
        TestMicrophone::default(),
        TestContextDetector(vec![
            context("com.example.editor", "Draft", false),
            context("com.example.browser", "Page", false),
        ]),
    );
    coordinator.hotkey_down("session-1").unwrap();
    coordinator.release().unwrap();
    coordinator.finalized().unwrap();
    coordinator.take_events();

    coordinator.processed().unwrap();

    assert!(matches!(
        coordinator.take_events().as_slice(),
        [
            CoordinatorEvent::State(SessionState::Injecting),
            CoordinatorEvent::InjectionReady { .. }
        ]
    ));
}
