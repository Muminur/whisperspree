use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use whisperspree_lib::{
    asr::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer},
    audio::vad::{VadDecision, VAD_FRAME_SAMPLES},
    error::Error,
    pipeline::session_pump::{FrameGate, SessionPump},
};

#[derive(Clone)]
struct RecordingRecognizer {
    started: Arc<Mutex<bool>>,
    fed: Arc<Mutex<Vec<Vec<f32>>>>,
    finalized: Arc<Mutex<bool>>,
    aborted: Arc<Mutex<bool>>,
}

#[async_trait]
impl SpeechRecognizer for RecordingRecognizer {
    async fn start(&mut self, _cfg: AsrConfig, _tx: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        *self.started.lock().unwrap() = true;
        Ok(())
    }

    fn feed(&mut self, samples: &[f32]) -> Result<(), Error> {
        self.fed.lock().unwrap().push(samples.to_vec());
        Ok(())
    }

    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        *self.finalized.lock().unwrap() = true;
        Ok(FinalTranscript {
            text: "hello".into(),
            words: vec![],
            language: Some("en".into()),
        })
    }

    async fn abort(&mut self) {
        *self.aborted.lock().unwrap() = true;
    }
}

struct ScriptedGate {
    decisions: Vec<VadDecision>,
}

struct SilenceGate;

impl FrameGate for SilenceGate {
    fn classify(&mut self, _frame: &[f32]) -> Result<VadDecision, Error> {
        Ok(VadDecision::Suppress)
    }

    fn is_silence_only(&self) -> bool {
        true
    }
}

impl FrameGate for ScriptedGate {
    fn classify(&mut self, _frame: &[f32]) -> Result<VadDecision, Error> {
        self.decisions
            .is_empty()
            .then_some(VadDecision::Suppress)
            .or_else(|| Some(self.decisions.remove(0)))
            .ok_or_else(|| Error::AsrLoad("missing scripted VAD decision".into()))
    }
}

fn recognizer() -> RecordingRecognizer {
    RecordingRecognizer {
        started: Arc::new(Mutex::new(false)),
        fed: Arc::new(Mutex::new(Vec::new())),
        finalized: Arc::new(Mutex::new(false)),
        aborted: Arc::new(Mutex::new(false)),
    }
}

#[tokio::test]
async fn fr_1_1_session_pump_starts_recognizer_and_flushes_vad_start_gate() {
    let recognizer = recognizer();
    let fed = Arc::clone(&recognizer.fed);
    let started = Arc::clone(&recognizer.started);
    let mut pump = SessionPump::new(
        recognizer,
        ScriptedGate {
            decisions: vec![
                VadDecision::Suppress,
                VadDecision::Suppress,
                VadDecision::SpeechStarted { buffered_frames: 3 },
                VadDecision::Feed,
            ],
        },
    );
    let (tx, _rx) = mpsc::channel(4);
    pump.start(AsrConfig::local("tiny"), tx).await.unwrap();
    assert!(*started.lock().unwrap());

    pump.push_samples(&vec![0.1; VAD_FRAME_SAMPLES * 4])
        .unwrap();
    assert_eq!(fed.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn fr_1_1_start_gate_drops_stale_non_speech_frame_before_asr() {
    let recognizer = recognizer();
    let fed = Arc::clone(&recognizer.fed);
    let mut pump = SessionPump::new(
        recognizer,
        ScriptedGate {
            decisions: vec![
                VadDecision::Suppress,
                VadDecision::Suppress,
                VadDecision::Suppress,
                VadDecision::SpeechStarted { buffered_frames: 3 },
            ],
        },
    );
    let (tx, _rx) = mpsc::channel(4);
    pump.start(AsrConfig::local("tiny"), tx).await.unwrap();
    let mut samples = Vec::new();
    for value in [0.1, 0.2, 0.3, 0.4] {
        samples.extend(std::iter::repeat_n(value, VAD_FRAME_SAMPLES));
    }
    pump.push_samples(&samples).unwrap();

    let fed = fed.lock().unwrap();
    assert_eq!(fed.len(), 3);
    assert!(fed.iter().all(|frame| frame[0] >= 0.2));
}

#[tokio::test]
async fn fr_1_1_session_pump_release_retains_at_most_300ms_tail_then_finalizes() {
    let recognizer = recognizer();
    let fed = Arc::clone(&recognizer.fed);
    let finalized = Arc::clone(&recognizer.finalized);
    let mut pump = SessionPump::new(
        recognizer,
        ScriptedGate {
            decisions: vec![VadDecision::SpeechStarted { buffered_frames: 1 }],
        },
    );
    let (tx, _rx) = mpsc::channel(4);
    pump.start(AsrConfig::local("tiny"), tx).await.unwrap();
    pump.push_samples(&vec![0.1; VAD_FRAME_SAMPLES]).unwrap();
    let result = pump
        .finalize_with_tail(&vec![0.2; VAD_FRAME_SAMPLES * 20])
        .await
        .unwrap();

    assert_eq!(result.text, "hello");
    assert!(*finalized.lock().unwrap());
    let fed_samples: usize = fed.lock().unwrap().iter().map(Vec::len).sum();
    assert_eq!(fed_samples, VAD_FRAME_SAMPLES + 4_800);
}

#[tokio::test]
async fn fr_1_1_session_pump_abort_calls_recognizer_abort() {
    let recognizer = recognizer();
    let aborted = Arc::clone(&recognizer.aborted);
    let mut pump = SessionPump::new(recognizer, ScriptedGate { decisions: vec![] });
    let (tx, _rx) = mpsc::channel(4);
    pump.start(AsrConfig::local("tiny"), tx).await.unwrap();
    pump.abort().await;
    assert!(*aborted.lock().unwrap());
}

#[tokio::test]
async fn fr_1_1_silence_only_session_aborts_without_normal_transcript() {
    let recognizer = recognizer();
    let finalized = Arc::clone(&recognizer.finalized);
    let aborted = Arc::clone(&recognizer.aborted);
    let mut pump = SessionPump::new(recognizer, SilenceGate);
    let (tx, _rx) = mpsc::channel(4);
    pump.start(AsrConfig::local("tiny"), tx).await.unwrap();
    pump.push_samples(&vec![0.0; VAD_FRAME_SAMPLES]).unwrap();

    let result = pump.finalize_with_tail(&[]).await.unwrap();
    assert!(result.text.is_empty());
    assert!(!*finalized.lock().unwrap());
    assert!(*aborted.lock().unwrap());
}
