//! T2.2 Slice F — the runtime injection phase (SM-3/SM-5, AC-1.4, P-5).
//!
//! Drives a real `CoordinatorRuntime` through Finalizing→PostProcessing and
//! proves that `run_injection_phase` performs the coordinator transition,
//! emits Injecting → outcome → Idle in order, and carries the P-5 history
//! suppression flag from secure targets.

use std::sync::{Arc, Mutex};

use whisperspree_lib::error::Error;
use whisperspree_lib::inject::{InjectContext, InjectMethod, Injector};
use whisperspree_lib::pipeline::session::SessionEventSink;
use whisperspree_lib::pipeline::SessionTaskEvent;
use whisperspree_lib::pipeline::{
    run_injection_phase, AppContext, ContextDetector, CoordinatorRuntime, Microphone, SessionState,
};

#[derive(Default)]
struct TestMicrophone;

impl Microphone for TestMicrophone {
    fn open(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

fn app_context(bundle_id: &str, secure: bool) -> AppContext {
    AppContext {
        bundle_id: bundle_id.into(),
        title: "Window".into(),
        secure_input: secure,
    }
}

#[derive(Default)]
struct RecordingInjector {
    result: Mutex<Option<Result<InjectMethod, Error>>>,
}

impl Injector for RecordingInjector {
    fn inject(&self, _text: &str, _ctx: &InjectContext) -> Result<InjectMethod, Error> {
        match self.result.lock().unwrap().take() {
            Some(result) => result,
            None => Ok(InjectMethod::Paste),
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    states: Mutex<Vec<SessionState>>,
    outcomes: Mutex<Vec<(String, bool)>>,
}

impl SessionEventSink for RecordingSink {
    fn emit_task(&self, _session_id: &str, _event: SessionTaskEvent) -> Result<(), Error> {
        Ok(())
    }
    fn emit_state(
        &self,
        _session_id: &str,
        state: SessionState,
        _engine: &str,
    ) -> Result<(), Error> {
        self.states.lock().unwrap().push(state);
        Ok(())
    }
    fn emit_injection_outcome(
        &self,
        _session_id: &str,
        method: &str,
        persist_history: bool,
    ) -> Result<(), Error> {
        self.outcomes
            .lock()
            .unwrap()
            .push((method.to_string(), persist_history));
        Ok(())
    }
}

/// Drives the shared coordinator to PostProcessing: the first snapshot is the
/// start context (style), the second is the injection-time target.
fn post_processing_runtime(
    start: AppContext,
    target: AppContext,
) -> CoordinatorRuntime<TestMicrophone, TwoShot> {
    let runtime = CoordinatorRuntime::new(TestMicrophone, TwoShot(vec![start, target]));
    runtime.start_control().expect("session must start");
    runtime.stop_control().expect("release must transition");
    runtime
        .finalized_control()
        .expect("finalized must transition");
    runtime
}

struct TwoShot(Vec<AppContext>);

impl ContextDetector for TwoShot {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        if self.0.is_empty() {
            return Ok(app_context("unknown", false));
        }
        Ok(self.0.remove(0))
    }
}

#[test]
fn fr_1_4_injection_phase_emits_injecting_outcome_then_idle_in_order() {
    let runtime = post_processing_runtime(
        app_context("com.example.mail", false),
        app_context("com.example.editor", false),
    );
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Ok(InjectMethod::Paste));
    let sink = Arc::new(RecordingSink::default());

    run_injection_phase(
        &runtime,
        injector.as_ref(),
        Some(&(sink.clone() as Arc<dyn SessionEventSink>)),
        "session-1",
        Some("hello world".into()),
    )
    .expect("injection phase must succeed");

    assert_eq!(
        *sink.states.lock().unwrap(),
        vec![SessionState::Injecting, SessionState::Idle,]
    );
    assert_eq!(
        *sink.outcomes.lock().unwrap(),
        vec![("paste".to_string(), true)]
    );
    assert_eq!(runtime.state().unwrap(), SessionState::Idle);
}

#[test]
fn sm_5_secure_target_reports_clipboard_only_and_suppresses_history_flag() {
    let runtime = post_processing_runtime(
        app_context("com.example.mail", false),
        app_context("com.example.passwords", true),
    );
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Ok(InjectMethod::ClipboardOnly));
    let sink = Arc::new(RecordingSink::default());

    run_injection_phase(
        &runtime,
        injector.as_ref(),
        Some(&(sink.clone() as Arc<dyn SessionEventSink>)),
        "session-1",
        Some("secret".into()),
    )
    .expect("secure path is not an error");

    assert_eq!(
        *sink.outcomes.lock().unwrap(),
        vec![("clipboard_only".to_string(), false)]
    );
}

#[test]
fn ec_1_4_injector_failure_returns_to_idle_without_an_outcome_event() {
    let runtime = post_processing_runtime(
        app_context("com.example.mail", false),
        app_context("com.example.editor", false),
    );
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Err(Error::InjFail("all strategies failed".into())));
    let sink = Arc::new(RecordingSink::default());

    run_injection_phase(
        &runtime,
        injector.as_ref(),
        Some(&(sink.clone() as Arc<dyn SessionEventSink>)),
        "session-1",
        Some("text".into()),
    )
    .expect("phase must not wedge the machine");

    assert!(sink.outcomes.lock().unwrap().is_empty());
    assert_eq!(runtime.state().unwrap(), SessionState::Idle);
}

#[test]
fn fr_1_4_missing_transcript_still_completes_the_session_machine() {
    let runtime = post_processing_runtime(
        app_context("com.example.mail", false),
        app_context("com.example.editor", false),
    );
    let sink = Arc::new(RecordingSink::default());

    run_injection_phase(
        &runtime,
        &RecordingInjector::default(),
        Some(&(sink.clone() as Arc<dyn SessionEventSink>)),
        "session-1",
        None,
    )
    .expect("missing transcript must not wedge the machine");

    assert_eq!(runtime.state().unwrap(), SessionState::Idle);
}
