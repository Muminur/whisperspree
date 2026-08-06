//! T0.5 — Event helpers and payloads for §9.2.
//!
//! Rust-side emitters are intentionally tiny and typed. Behavior stays TODO-stub
//! for now because §9.2 event consumers are implemented in later milestones.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

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
    pub state: String,
    pub engine: Option<String>,
    pub style_id: Option<String>,
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
    pub words: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptFinalPayload {
    pub session_id: String,
    pub raw_text: String,
    pub words: Vec<serde_json::Value>,
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
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

pub fn emit_session_state(app: &AppHandle, payload: SessionStatePayload) {
    app.emit(EVENT_SESSION_STATE, payload)
        .expect("emitting session:state must not fail in this skeleton");
}

pub fn emit_transcript_partial(app: &AppHandle, payload: TranscriptPartialPayload) {
    app.emit(EVENT_TRANSCRIPT_PARTIAL, payload)
        .expect("emitting transcript:partial must not fail in this skeleton");
}

pub fn emit_transcript_segment(app: &AppHandle, payload: TranscriptSegmentPayload) {
    app.emit(EVENT_TRANSCRIPT_SEGMENT, payload)
        .expect("emitting transcript:segment must not fail in this skeleton");
}

pub fn emit_transcript_final(app: &AppHandle, payload: TranscriptFinalPayload) {
    app.emit(EVENT_TRANSCRIPT_FINAL, payload)
        .expect("emitting transcript:final must not fail in this skeleton");
}

pub fn emit_postprocess_done(app: &AppHandle, payload: PostprocessDonePayload) {
    app.emit(EVENT_POSTPROCESS_DONE, payload)
        .expect("emitting postprocess:done must not fail in this skeleton");
}

pub fn emit_inject_done(app: &AppHandle, payload: InjectDonePayload) {
    app.emit(EVENT_INJECT_DONE, payload)
        .expect("emitting inject:done must not fail in this skeleton");
}

pub fn emit_audio_level(app: &AppHandle, payload: AudioLevelPayload) {
    app.emit(EVENT_AUDIO_LEVEL, payload)
        .expect("emitting audio:level must not fail in this skeleton");
}

pub fn emit_language_detected(app: &AppHandle, payload: LanguageDetectedPayload) {
    app.emit(EVENT_LANGUAGE_DETECTED, payload)
        .expect("emitting language:detected must not fail in this skeleton");
}

pub fn emit_model_download_progress(app: &AppHandle, payload: ModelDownloadProgressPayload) {
    app.emit(EVENT_MODEL_DOWNLOAD_PROGRESS, payload)
        .expect("emitting model:download:progress must not fail in this skeleton");
}

pub fn emit_app_error(app: &AppHandle, payload: AppErrorPayload) {
    app.emit(EVENT_APP_ERROR, payload)
        .expect("emitting app:error must not fail in this skeleton");
}
