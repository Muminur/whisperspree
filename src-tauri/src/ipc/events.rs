//! T0.5 — Event helpers and payloads for §9.2.
//!
//! Rust-side emitters are intentionally tiny and typed. Behavior stays TODO-stub
//! for now because §9.2 event consumers are implemented in later milestones.

use crate::{
    asr::AsrEvent,
    error::{ApiError, Error},
    pipeline::session_task::SessionTaskEvent,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

pub const EVENT_SESSION_STATE: &str = "session:state";
pub const EVENT_TRANSCRIPT_PARTIAL: &str = "transcript:partial";
pub const EVENT_TRANSCRIPT_SEGMENT: &str = "transcript:segment";
pub const EVENT_TRANSCRIPT_FINAL: &str = "transcript:final";
pub const EVENT_POSTPROCESS_DONE: &str = "postprocess:done";
pub const EVENT_INJECT_DONE: &str = "inject:done";
pub const EVENT_AUDIO_LEVEL: &str = "audio:level";
pub const EVENT_LANGUAGE_DETECTED: &str = "language:detected";
pub const EVENT_MODEL_DOWNLOAD_PROGRESS: &str = "model:download:progress";
pub const EVENT_APP_ERROR: &str = "app:error";

pub const IPC_EVENTS: [&str; 10] = [
    "session:state",
    "transcript:partial",
    "transcript:segment",
    "transcript:final",
    "postprocess:done",
    "inject:done",
    "audio:level",
    "language:detected",
    "model:download:progress",
    "app:error",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatePayload {
    pub session_id: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Arming,
    Listening,
    Finalizing,
    PostProcessing,
    Injecting,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub w: String,
    pub s: u32,
    pub e: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPartialPayload {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegmentPayload {
    pub session_id: String,
    pub text: String,
    pub words: Vec<WordTiming>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptFinalPayload {
    pub session_id: String,
    pub raw_text: String,
    pub words: Vec<WordTiming>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostprocessDonePayload {
    pub session_id: String,
    pub text: String,
    pub persona_id: String,
    pub fallback_used: bool,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectDonePayload {
    pub session_id: String,
    pub method: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevelPayload {
    pub rms: f32,
    pub peak: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageDetectedPayload {
    pub code: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgressPayload {
    pub id: String,
    pub received: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppErrorPayload {
    code: String,
    message: String,
    recoverable: bool,
}

impl AppErrorPayload {
    pub fn from_error(error: &Error) -> Self {
        let api = ApiError::from(error);
        Self {
            code: api.code,
            message: api.message,
            recoverable: error.recoverable(),
        }
    }
}

pub trait EventSink {
    type Error;

    fn emit<P: Serialize + Clone>(&self, event: &str, payload: P) -> Result<(), Self::Error>;
}

impl<R: Runtime> EventSink for AppHandle<R> {
    type Error = tauri::Error;

    fn emit<P: Serialize + Clone>(&self, event: &str, payload: P) -> Result<(), Self::Error> {
        Emitter::emit(self, event, payload)
    }
}

pub fn emit_session_state<S: EventSink>(
    sink: &S,
    payload: SessionStatePayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_SESSION_STATE, payload)
}

pub fn emit_transcript_partial<S: EventSink>(
    sink: &S,
    payload: TranscriptPartialPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_TRANSCRIPT_PARTIAL, payload)
}

pub fn emit_transcript_segment<S: EventSink>(
    sink: &S,
    payload: TranscriptSegmentPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_TRANSCRIPT_SEGMENT, payload)
}

pub fn emit_transcript_final<S: EventSink>(
    sink: &S,
    payload: TranscriptFinalPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_TRANSCRIPT_FINAL, payload)
}

pub fn emit_postprocess_done<S: EventSink>(
    sink: &S,
    payload: PostprocessDonePayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_POSTPROCESS_DONE, payload)
}

pub fn emit_inject_done<S: EventSink>(
    sink: &S,
    payload: InjectDonePayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_INJECT_DONE, payload)
}

pub fn emit_audio_level<S: EventSink>(
    sink: &S,
    payload: AudioLevelPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_AUDIO_LEVEL, payload)
}

pub fn emit_language_detected<S: EventSink>(
    sink: &S,
    payload: LanguageDetectedPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_LANGUAGE_DETECTED, payload)
}

pub fn emit_model_download_progress<S: EventSink>(
    sink: &S,
    payload: ModelDownloadProgressPayload,
) -> Result<(), S::Error> {
    sink.emit(EVENT_MODEL_DOWNLOAD_PROGRESS, payload)
}

pub fn emit_app_error<S: EventSink>(sink: &S, payload: AppErrorPayload) -> Result<(), S::Error> {
    sink.emit(EVENT_APP_ERROR, payload)
}

/// Map one recognizer event to its typed §9.2 event. The session task owns the
/// session id; this adapter owns only wire-shape conversion and propagation of
/// emitter failures.
pub fn emit_asr_event<S: EventSink>(
    sink: &S,
    session_id: &str,
    event: AsrEvent,
) -> Result<(), S::Error> {
    match event {
        AsrEvent::Partial { text } => emit_transcript_partial(
            sink,
            TranscriptPartialPayload {
                session_id: session_id.into(),
                text,
            },
        ),
        AsrEvent::Segment { text, words } => emit_transcript_segment(
            sink,
            TranscriptSegmentPayload {
                session_id: session_id.into(),
                text,
                words,
            },
        ),
        AsrEvent::LanguageDetected { code, confidence } => {
            emit_language_detected(sink, LanguageDetectedPayload { code, confidence })
        }
        AsrEvent::Error { error } => emit_app_error(sink, AppErrorPayload::from_error(&error)),
    }
}

/// Surface a completed session-task output through the typed §9.2 helpers.
///
/// This adapter is deliberately side-effect-free beyond the supplied event
/// sink: the macOS runtime may call it from its session thread, while tests use
/// a deterministic recording sink. Coordinator state transitions remain owned
/// by the coordinator; `Cancelled` is the only terminal session-task outcome
/// that has a direct §9.2 state representation here.
pub fn emit_session_task_event<S: EventSink>(
    sink: &S,
    session_id: &str,
    event: SessionTaskEvent,
) -> Result<(), S::Error> {
    match event {
        SessionTaskEvent::Asr(event) => emit_asr_event(sink, session_id, event),
        SessionTaskEvent::Final(transcript) => emit_transcript_final(
            sink,
            TranscriptFinalPayload {
                session_id: session_id.into(),
                raw_text: transcript.text,
                words: transcript.words,
                language: transcript.language,
            },
        ),
        SessionTaskEvent::Cancelled => emit_session_state(
            sink,
            SessionStatePayload {
                session_id: session_id.into(),
                state: SessionState::Cancelled,
                engine: None,
                style_id: None,
            },
        ),
        SessionTaskEvent::Failed(error) => {
            emit_app_error(sink, AppErrorPayload::from_error(&error))
        }
    }
}
