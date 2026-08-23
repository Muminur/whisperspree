use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::mpsc;
use whisperspree_lib::{
    asr::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer},
    audio::vad::VadDecision,
    error::Error,
    pipeline::{
        session_pump::FrameGate,
        session_task::{SessionTask, SessionTaskEvent},
    },
};

struct Gate;
impl FrameGate for Gate {
    fn classify(&mut self, _: &[f32]) -> Result<VadDecision, Error> {
        Ok(VadDecision::Feed)
    }
}

struct Failing;
#[async_trait]
impl SpeechRecognizer for Failing {
    async fn start(&mut self, _: AsrConfig, _: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        Ok(())
    }
    fn feed(&mut self, _: &[f32]) -> Result<(), Error> {
        Ok(())
    }
    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        Err(Error::AsrLoad("finalize failed".into()))
    }
    async fn abort(&mut self) {}
}

struct FailingTail {
    aborted: Arc<AtomicBool>,
}

#[async_trait]
impl SpeechRecognizer for FailingTail {
    async fn start(&mut self, _: AsrConfig, _: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        Ok(())
    }
    fn feed(&mut self, _: &[f32]) -> Result<(), Error> {
        Err(Error::AsrLoad("tail feed failed".into()))
    }
    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        Ok(FinalTranscript {
            text: "unused".into(),
            words: vec![],
            language: None,
        })
    }
    async fn abort(&mut self) {
        self.aborted.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn ec_1_1_finalize_failure_marks_task_stopped_and_emits_failed() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(Failing, Gate, tx);
    task.start(AsrConfig::local("small")).await.unwrap();
    assert!(task.stop(&[]).await.is_err());
    assert!(matches!(
        rx.recv().await,
        Some(SessionTaskEvent::Failed(Error::AsrLoad(_)))
    ));
    assert!(task.push_samples(&[0.0; 480]).is_err());
}

#[tokio::test]
async fn ec_1_1_tail_feed_failure_aborts_and_stops_task() {
    let (tx, mut rx) = mpsc::channel(8);
    let aborted = Arc::new(AtomicBool::new(false));
    let mut task = SessionTask::new(
        FailingTail {
            aborted: aborted.clone(),
        },
        Gate,
        tx,
    );
    task.start(AsrConfig::local("small")).await.unwrap();
    assert!(task.stop(&[0.0; 480]).await.is_err());
    assert!(matches!(
        rx.recv().await,
        Some(SessionTaskEvent::Failed(Error::AsrLoad(_)))
    ));
    assert!(aborted.load(Ordering::SeqCst));
    assert!(task.push_samples(&[0.0; 480]).is_err());
}

#[tokio::test]
async fn ec_1_1_live_poll_failure_aborts_capture_and_emits_failed() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(
        FailingTail {
            aborted: Arc::new(AtomicBool::new(false)),
        },
        Gate,
        tx,
    );
    task.start(AsrConfig::local("small")).await.unwrap();
    task.fail(Error::AsrLoad("poll failed".into())).await;
    assert!(matches!(
        rx.recv().await,
        Some(SessionTaskEvent::Failed(Error::AsrLoad(message))) if message == "poll failed"
    ));
    assert!(task.push_samples(&[0.0; 480]).is_err());
}
