//! T0.5 — event helpers must propagate emitter failures without panicking.
//!
//! The production event module owns an `EventSink` seam. This fake is a real
//! deterministic in-process boundary double: it captures serialized payloads
//! on success and returns the exact injected failure on demand.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use serde_json::{json, Value};
use whisperspree_lib::{
    asr::{AsrEvent, FinalTranscript},
    error::Error,
    ipc::events,
    pipeline::session_task::SessionTaskEvent,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeEmitError(&'static str);

impl fmt::Display for FakeEmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for FakeEmitError {}

#[derive(Clone, Default)]
struct RecordingSink {
    recorded: Arc<Mutex<Vec<(String, Value)>>>,
    failure: Option<FakeEmitError>,
}

impl RecordingSink {
    fn failing() -> Self {
        Self {
            recorded: Arc::default(),
            failure: Some(FakeEmitError("deterministic emitter failure")),
        }
    }

    fn recorded(&self) -> Vec<(String, Value)> {
        self.recorded.lock().unwrap().clone()
    }
}

impl events::EventSink for RecordingSink {
    type Error = FakeEmitError;

    fn emit<P: serde::Serialize + Clone>(
        &self,
        event: &str,
        payload: P,
    ) -> Result<(), Self::Error> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let payload = serde_json::to_value(payload)
            .expect("test fake records only the serializable event payload contract");
        self.recorded
            .lock()
            .unwrap()
            .push((event.to_owned(), payload));
        Ok(())
    }
}

fn word() -> events::WordTiming {
    events::WordTiming {
        w: "hello".into(),
        s: 120,
        e: 480,
    }
}

fn app_error() -> events::AppErrorPayload {
    events::AppErrorPayload::from_error(&Error::NetStream("connection lost".into()))
}

#[test]
fn fr_1_1_asr_events_map_to_exact_session_wire_events() {
    let sink = RecordingSink::default();
    events::emit_asr_event(
        &sink,
        "session-1",
        AsrEvent::Partial {
            text: "hello".into(),
        },
    )
    .unwrap();
    events::emit_asr_event(
        &sink,
        "session-1",
        AsrEvent::Segment {
            text: "hello world".into(),
            words: vec![word()],
        },
    )
    .unwrap();
    events::emit_asr_event(
        &sink,
        "session-1",
        AsrEvent::LanguageDetected {
            code: "en".into(),
            confidence: 0.9,
        },
    )
    .unwrap();
    let recorded = sink.recorded();
    assert_eq!(recorded[0].0, "transcript:partial");
    assert_eq!(recorded[0].1["sessionId"], "session-1");
    assert_eq!(recorded[1].0, "transcript:segment");
    assert_eq!(recorded[2].0, "language:detected");
}

#[test]
fn fr_1_1_session_task_events_surface_asr_final_cancel_and_failure_through_ipc_helpers() {
    let sink = RecordingSink::default();
    events::emit_session_task_event(
        &sink,
        "session-1",
        SessionTaskEvent::Asr(AsrEvent::Partial {
            text: "hello".into(),
        }),
    )
    .unwrap();
    events::emit_session_task_event(
        &sink,
        "session-1",
        SessionTaskEvent::Final(FinalTranscript {
            text: "hello world".into(),
            words: vec![word()],
            language: Some("en".into()),
        }),
    )
    .unwrap();
    events::emit_session_task_event(&sink, "session-1", SessionTaskEvent::Cancelled).unwrap();
    events::emit_session_task_event(
        &sink,
        "session-1",
        SessionTaskEvent::Failed(Error::AsrLoad("corrupt model".into())),
    )
    .unwrap();

    assert_eq!(
        sink.recorded(),
        vec![
            (
                "transcript:partial".into(),
                json!({ "sessionId": "session-1", "text": "hello" })
            ),
            (
                "transcript:final".into(),
                json!({ "sessionId": "session-1", "rawText": "hello world", "words": [{ "w": "hello", "s": 120, "e": 480 }], "language": "en" })
            ),
            (
                "session:state".into(),
                json!({ "sessionId": "session-1", "state": "cancelled" })
            ),
            (
                "app:error".into(),
                json!({ "code": "ASR-LOAD", "message": "corrupt model", "recoverable": false })
            ),
        ]
    );
}

#[test]
fn ec_1_1_session_task_event_bridge_propagates_event_sink_failures() {
    let error = events::emit_session_task_event(
        &RecordingSink::failing(),
        "session-1",
        SessionTaskEvent::Cancelled,
    )
    .expect_err("the session bridge must return emitter failures");
    assert_eq!(error, FakeEmitError("deterministic emitter failure"));
}

macro_rules! assert_emit_failure {
    ($helper:path, $payload:expr, $event:literal) => {{
        let error = $helper(&RecordingSink::failing(), $payload).expect_err(concat!(
            $event,
            " failures must be returned, never panicked"
        ));
        assert_eq!(error, FakeEmitError("deterministic emitter failure"));
    }};
}

#[test]
fn fr_0_5_event_helpers_emit_exact_event_and_payload_through_project_sink() {
    let sink = RecordingSink::default();
    events::emit_session_state(
        &sink,
        events::SessionStatePayload {
            session_id: "session-1".into(),
            state: events::SessionState::Listening,
            engine: Some("local".into()),
            style_id: None,
        },
    )
    .unwrap();
    events::emit_transcript_partial(
        &sink,
        events::TranscriptPartialPayload {
            session_id: "session-1".into(),
            text: "hello…".into(),
        },
    )
    .unwrap();
    events::emit_transcript_segment(
        &sink,
        events::TranscriptSegmentPayload {
            session_id: "session-1".into(),
            text: "hello".into(),
            words: vec![word()],
        },
    )
    .unwrap();
    events::emit_transcript_final(
        &sink,
        events::TranscriptFinalPayload {
            session_id: "session-1".into(),
            raw_text: "hello world".into(),
            words: vec![word()],
            language: Some("en".into()),
        },
    )
    .unwrap();
    events::emit_postprocess_done(
        &sink,
        events::PostprocessDonePayload {
            session_id: "session-1".into(),
            text: "Hello, world.".into(),
            persona_id: "professional".into(),
            fallback_used: false,
            latency_ms: 321,
        },
    )
    .unwrap();
    events::emit_inject_done(
        &sink,
        events::InjectDonePayload {
            session_id: "session-1".into(),
            method: "paste".into(),
        },
    )
    .unwrap();
    events::emit_audio_level(
        &sink,
        events::AudioLevelPayload {
            rms: 0.25,
            peak: 0.75,
        },
    )
    .unwrap();
    events::emit_language_detected(
        &sink,
        events::LanguageDetectedPayload {
            code: "en".into(),
            confidence: 0.5,
        },
    )
    .unwrap();
    events::emit_model_download_progress(
        &sink,
        events::ModelDownloadProgressPayload {
            id: "small".into(),
            received: 123,
            total: 456,
        },
    )
    .unwrap();
    events::emit_app_error(&sink, app_error()).unwrap();

    assert_eq!(
        sink.recorded(),
        vec![
            (
                "session:state".into(),
                json!({ "sessionId": "session-1", "state": "listening", "engine": "local" })
            ),
            (
                "transcript:partial".into(),
                json!({ "sessionId": "session-1", "text": "hello…" })
            ),
            (
                "transcript:segment".into(),
                json!({ "sessionId": "session-1", "text": "hello", "words": [{ "w": "hello", "s": 120, "e": 480 }] })
            ),
            (
                "transcript:final".into(),
                json!({ "sessionId": "session-1", "rawText": "hello world", "words": [{ "w": "hello", "s": 120, "e": 480 }], "language": "en" })
            ),
            (
                "postprocess:done".into(),
                json!({ "sessionId": "session-1", "text": "Hello, world.", "personaId": "professional", "fallbackUsed": false, "latencyMs": 321 })
            ),
            (
                "inject:done".into(),
                json!({ "sessionId": "session-1", "method": "paste" })
            ),
            ("audio:level".into(), json!({ "rms": 0.25, "peak": 0.75 })),
            (
                "language:detected".into(),
                json!({ "code": "en", "confidence": 0.5 })
            ),
            (
                "model:download:progress".into(),
                json!({ "id": "small", "received": 123, "total": 456 })
            ),
            (
                "app:error".into(),
                json!({ "code": "NET-STREAM", "message": "connection lost", "recoverable": true })
            ),
        ]
    );
}

#[test]
fn fr_0_5_every_event_helper_returns_sink_failure_without_panic() {
    assert_emit_failure!(
        events::emit_session_state,
        events::SessionStatePayload {
            session_id: "session-1".into(),
            state: events::SessionState::Idle,
            engine: None,
            style_id: None
        },
        "session:state"
    );
    assert_emit_failure!(
        events::emit_transcript_partial,
        events::TranscriptPartialPayload {
            session_id: "session-1".into(),
            text: "hello".into()
        },
        "transcript:partial"
    );
    assert_emit_failure!(
        events::emit_transcript_segment,
        events::TranscriptSegmentPayload {
            session_id: "session-1".into(),
            text: "hello".into(),
            words: vec![word()]
        },
        "transcript:segment"
    );
    assert_emit_failure!(
        events::emit_transcript_final,
        events::TranscriptFinalPayload {
            session_id: "session-1".into(),
            raw_text: "hello".into(),
            words: vec![word()],
            language: Some("en".into())
        },
        "transcript:final"
    );
    assert_emit_failure!(
        events::emit_postprocess_done,
        events::PostprocessDonePayload {
            session_id: "session-1".into(),
            text: "hello".into(),
            persona_id: "clean".into(),
            fallback_used: false,
            latency_ms: 1
        },
        "postprocess:done"
    );
    assert_emit_failure!(
        events::emit_inject_done,
        events::InjectDonePayload {
            session_id: "session-1".into(),
            method: "paste".into()
        },
        "inject:done"
    );
    assert_emit_failure!(
        events::emit_audio_level,
        events::AudioLevelPayload {
            rms: 0.1,
            peak: 0.2
        },
        "audio:level"
    );
    assert_emit_failure!(
        events::emit_language_detected,
        events::LanguageDetectedPayload {
            code: "en".into(),
            confidence: 0.5
        },
        "language:detected"
    );
    assert_emit_failure!(
        events::emit_model_download_progress,
        events::ModelDownloadProgressPayload {
            id: "small".into(),
            received: 1,
            total: 2
        },
        "model:download:progress"
    );
    assert_emit_failure!(events::emit_app_error, app_error(), "app:error");
}
