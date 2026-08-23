//! Speech-recognition support services.

use crate::{error::Error, ipc::events::WordTiming};
use async_trait::async_trait;
use tokio::sync::mpsc;

pub mod deepgram;
pub mod local_whisper;
#[cfg(target_os = "macos")]
mod macos;
pub mod mode;
pub mod model_manager;

/// Per-session ASR settings consumed by the §9.3 recognizer boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrConfig {
    pub local_model: String,
    pub language: Option<String>,
    pub translate_to_english: bool,
}

impl AsrConfig {
    pub fn local(model: impl Into<String>) -> Self {
        Self {
            local_model: model.into(),
            language: None,
            translate_to_english: false,
        }
    }
}

/// Events a recognizer sends to the session task (§9.3).
#[derive(Debug)]
pub enum AsrEvent {
    Partial {
        text: String,
    },
    Segment {
        text: String,
        words: Vec<WordTiming>,
    },
    LanguageDetected {
        code: String,
        confidence: f32,
    },
    Error {
        error: Error,
    },
}

/// The complete local or cloud recognition result for one utterance (§9.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalTranscript {
    pub text: String,
    pub words: Vec<WordTiming>,
    pub language: Option<String>,
}

/// The shared recognizer contract used by the session state machine (§9.3).
#[async_trait]
pub trait SpeechRecognizer: Send {
    async fn start(&mut self, cfg: AsrConfig, tx: mpsc::Sender<AsrEvent>) -> Result<(), Error>;
    fn feed(&mut self, pcm_16k_mono: &[f32]) -> Result<(), Error>;
    fn endpoint_silence(&mut self, _silence_ms: u32) -> Result<(), Error> {
        Ok(())
    }
    /// Service completed live decode work without waiting for finalization.
    /// Streaming engines may use this to reap a window and schedule its
    /// successor while capture continues; the default is a no-op.
    async fn service(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn finalize(&mut self) -> Result<FinalTranscript, Error>;
    async fn abort(&mut self);
}
