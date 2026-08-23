use async_trait::async_trait;
use tokio::sync::mpsc;
use whisperspree_lib::{
    asr::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer},
    audio::{
        capture::{capture_queue, CaptureProcessor},
        vad::VadDecision,
    },
    error::Error,
    ipc::events::WordTiming,
    pipeline::{
        session_pump::{CapturePoller, FrameGate},
        session_task::{LiveCaptureSession, SessionTask, SessionTaskEvent},
    },
};

struct Gate(Vec<VadDecision>);
impl FrameGate for Gate {
    fn classify(&mut self, _: &[f32]) -> Result<VadDecision, Error> {
        Ok(self.0.remove(0))
    }
}

struct SilenceGate;
impl FrameGate for SilenceGate {
    fn classify(&mut self, _: &[f32]) -> Result<VadDecision, Error> {
        Ok(VadDecision::Suppress)
    }
    fn is_silence_only(&self) -> bool {
        true
    }
}

struct Rec {
    feeds: usize,
    aborted: bool,
}

struct SegmentOnFinalize {
    events: Option<mpsc::Sender<AsrEvent>>,
}

#[async_trait]
impl SpeechRecognizer for SegmentOnFinalize {
    async fn start(&mut self, _: AsrConfig, tx: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        self.events = Some(tx);
        Ok(())
    }

    fn feed(&mut self, _: &[f32]) -> Result<(), Error> {
        Ok(())
    }

    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        self.events
            .take()
            .expect("start stores the ASR event sender")
            .send(AsrEvent::Segment {
                text: "last".into(),
                words: vec![WordTiming {
                    w: "last".into(),
                    s: 0,
                    e: 10,
                }],
            })
            .await
            .expect("session pump retains the ASR receiver during finalize");
        Ok(FinalTranscript {
            text: "last".into(),
            words: vec![WordTiming {
                w: "last".into(),
                s: 0,
                e: 10,
            }],
            language: None,
        })
    }

    async fn abort(&mut self) {}
}
#[async_trait]
impl SpeechRecognizer for Rec {
    async fn start(&mut self, _: AsrConfig, _: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        Ok(())
    }
    fn feed(&mut self, _: &[f32]) -> Result<(), Error> {
        self.feeds += 1;
        Ok(())
    }
    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        Ok(FinalTranscript {
            text: "ok".into(),
            words: vec![WordTiming {
                w: "ok".into(),
                s: 0,
                e: 10,
            }],
            language: None,
        })
    }
    async fn abort(&mut self) {
        self.aborted = true;
    }
}

#[tokio::test]
async fn fr_1_1_session_task_forwards_final_and_bounds_release_tail() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(
        Rec {
            feeds: 0,
            aborted: false,
        },
        Gate(vec![VadDecision::SpeechStarted { buffered_frames: 0 }]),
        tx,
    );
    task.start(AsrConfig::local("small")).await.unwrap();
    task.push_samples(&vec![1.0; 480]).unwrap();
    let final_result = task.stop(&vec![0.5; 9_000]).await.unwrap();
    assert_eq!(final_result.text, "ok");
    assert!(matches!(rx.recv().await, Some(SessionTaskEvent::Final(_))));
}

#[tokio::test]
async fn fr_1_3_vad_speech_is_reported_before_short_release() {
    let (tx, mut rx) = mpsc::channel(8);
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_for_task = std::sync::Arc::clone(&observed);
    let mut task = SessionTask::new(
        Rec {
            feeds: 0,
            aborted: false,
        },
        Gate(vec![VadDecision::SpeechStarted { buffered_frames: 0 }]),
        tx,
    );
    task.set_speech_observer(std::sync::Arc::new(move || {
        observed_for_task.store(true, std::sync::atomic::Ordering::Release);
    }));
    task.start(AsrConfig::local("small")).await.unwrap();
    task.push_samples(&vec![1.0; 480]).unwrap();
    assert!(observed.load(std::sync::atomic::Ordering::Acquire));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn fr_1_1_session_task_cancel_emits_cancelled_without_final() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(
        Rec {
            feeds: 0,
            aborted: false,
        },
        Gate(vec![]),
        tx,
    );
    task.start(AsrConfig::local("small")).await.unwrap();
    task.cancel().await.unwrap();
    assert!(matches!(rx.recv().await, Some(SessionTaskEvent::Cancelled)));
}

#[tokio::test]
async fn fr_1_1_silence_only_session_emits_distinct_notice_outcome() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(
        Rec {
            feeds: 0,
            aborted: false,
        },
        SilenceGate,
        tx,
    );
    task.start(AsrConfig::local("small")).await.unwrap();
    let result = task.stop(&[]).await.unwrap();
    assert!(result.text.is_empty());
    assert!(matches!(
        rx.recv().await,
        Some(SessionTaskEvent::SilenceOnly)
    ));
}

#[tokio::test]
async fn fr_1_1_finalize_forwards_last_asr_segment_before_final_event() {
    let (tx, mut rx) = mpsc::channel(8);
    let mut task = SessionTask::new(SegmentOnFinalize { events: None }, Gate(vec![]), tx);
    task.start(AsrConfig::local("small")).await.unwrap();
    task.stop(&[]).await.unwrap();
    assert!(matches!(
        rx.recv().await,
        Some(SessionTaskEvent::Asr(AsrEvent::Segment { text, .. })) if text == "last"
    ));
    assert!(matches!(rx.recv().await, Some(SessionTaskEvent::Final(_))));
}

#[tokio::test]
async fn fr_1_1_live_capture_session_drains_capture_into_vad_and_asr() {
    let (mut producer, consumer) = capture_queue(2_000);
    let capture = CapturePoller::new(CaptureProcessor::new(consumer, 16_000), 4_000);
    let (tx, mut rx) = mpsc::channel(8);
    let mut session = LiveCaptureSession::new(
        capture,
        SessionTask::new(
            Rec {
                feeds: 0,
                aborted: false,
            },
            Gate(vec![
                VadDecision::SpeechStarted { buffered_frames: 0 },
                VadDecision::Feed,
            ]),
            tx,
        ),
    );
    session.start(AsrConfig::local("small")).await.unwrap();
    producer.push_interleaved(&vec![0.5_f32; 960], 1);
    session.poll(std::time::Instant::now()).unwrap();
    let result = session.stop().await.unwrap();
    assert_eq!(result.text, "ok");
    assert!(matches!(rx.recv().await, Some(SessionTaskEvent::Final(_))));
}
