//! Session-task composition for capture, VAD, ASR, and typed output events.
//!
//! This layer deliberately owns the asynchronous recognizer pump.  Platform
//! threads only hand it already-resampled PCM; they never call a recognizer or
//! mutate coordinator state from the audio callback.

use super::session_pump::{CapturePoller, FrameGate, SessionPump, TAIL_SAMPLES};
use crate::asr::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer};
use crate::audio::level::AudioLevel;
use crate::error::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub enum SessionTaskEvent {
    Asr(AsrEvent),
    Final(FinalTranscript),
    Cancelled,
    SilenceOnly,
    Failed(Error),
}

/// One live utterance.  The owner must call `start` exactly once, then may
/// feed arbitrary capture batches until `stop` or `cancel` is selected.
pub struct SessionTask<R, G> {
    pump: SessionPump<R, G>,
    events: mpsc::Sender<SessionTaskEvent>,
    started: bool,
    forwarder: Option<tokio::task::JoinHandle<()>>,
    flush: Option<mpsc::Sender<oneshot::Sender<()>>>,
    speech_reported: bool,
    speech_observer: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}

/// Full non-real-time capture composition. It drains the CPAL consumer through
/// the stateful resampler/ring, forwards the resulting 16 kHz batches into the
/// VAD/ASR pump, and retains only the bounded release tail.
pub struct LiveCaptureSession<R, G> {
    capture: CapturePoller,
    task: SessionTask<R, G>,
    tail: Vec<f32>,
    releasing: bool,
}

impl<R: SpeechRecognizer, G: FrameGate> LiveCaptureSession<R, G> {
    pub fn new(capture: CapturePoller, task: SessionTask<R, G>) -> Self {
        Self {
            capture,
            task,
            tail: Vec::new(),
            releasing: false,
        }
    }

    pub async fn start(&mut self, config: AsrConfig) -> Result<(), Error> {
        self.task.start(config).await
    }

    pub fn poll(&mut self, now: std::time::Instant) -> Result<(), Error> {
        self.poll_with_level(now).map(|_| ())
    }

    pub fn poll_with_level(
        &mut self,
        now: std::time::Instant,
    ) -> Result<Option<AudioLevel>, Error> {
        let (samples, level) = self.capture.poll_with_level(now);
        if let Some(samples) = samples {
            if self.releasing {
                self.tail.extend_from_slice(&samples);
                if self.tail.len() > TAIL_SAMPLES {
                    let drop_count = self.tail.len() - TAIL_SAMPLES;
                    self.tail.drain(..drop_count);
                }
            } else {
                self.task.push_samples(&samples)?;
            }
        }
        Ok(level)
    }

    /// EC-1.1 seam: mid-session recognizer replacement (cloud drop -> local).
    pub fn recognizer_mut(&mut self) -> &mut R {
        self.task.recognizer_mut()
    }

    /// EC-1.1: replay retained gated audio into the replacement recognizer.
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<(), Error> {
        self.task.push_samples(samples)
    }

    /// EC-1.1: discard the dead recognizer without publishing anything.
    pub async fn abort(&mut self) {
        let _ = self.task.cancel().await;
    }

    /// EC-1.1: gated audio already delivered to the current recognizer.
    pub fn utterance_samples(&self) -> &[f32] {
        self.task.utterance_samples()
    }

    pub fn begin_release(&mut self) {
        self.releasing = true;
        self.tail.clear();
    }

    pub async fn service(&mut self) -> Result<(), Error> {
        self.task.service().await
    }

    pub async fn stop(&mut self) -> Result<FinalTranscript, Error> {
        let tail = std::mem::take(&mut self.tail);
        let result = self.task.stop(&tail).await;
        self.tail.clear();
        result
    }

    pub async fn cancel(&mut self) -> Result<(), Error> {
        self.tail.clear();
        self.task.cancel().await
    }

    pub async fn fail(&mut self, error: Error) {
        self.tail.clear();
        self.task.fail(error).await;
    }
}

impl<R: SpeechRecognizer, G: FrameGate> SessionTask<R, G> {
    pub fn new(recognizer: R, gate: G, events: mpsc::Sender<SessionTaskEvent>) -> Self {
        Self {
            pump: SessionPump::new(recognizer, gate),
            events,
            started: false,
            forwarder: None,
            flush: None,
            speech_reported: false,
            speech_observer: None,
        }
    }

    pub async fn start(&mut self, config: AsrConfig) -> Result<(), Error> {
        if self.started {
            return Err(Error::IllegalTransition("session already started".into()));
        }
        let (asr_tx, mut asr_rx) = mpsc::channel(32);
        self.pump.start(config, asr_tx).await?;
        self.started = true;
        self.speech_reported = false;
        let events = self.events.clone();
        let (flush_tx, mut flush_rx) = mpsc::channel::<oneshot::Sender<()>>(1);
        self.flush = Some(flush_tx);
        self.forwarder = Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = asr_rx.recv() => {
                        let Some(event) = event else { break };
                        if events.send(SessionTaskEvent::Asr(event)).await.is_err() {
                            break;
                        }
                    }
                    ack = flush_rx.recv() => {
                        let Some(ack) = ack else { break };
                        while let Ok(event) = asr_rx.try_recv() {
                            if events.send(SessionTaskEvent::Asr(event)).await.is_err() {
                                break;
                            }
                        }
                        let _ = ack.send(());
                        break;
                    }
                }
            }
        }));
        Ok(())
    }

    async fn flush_forwarder(&mut self) {
        if let Some(flush) = self.flush.take() {
            let (ack_tx, ack_rx) = oneshot::channel();
            if flush.send(ack_tx).await.is_ok() {
                let _ = ack_rx.await;
            }
        }
        if let Some(forwarder) = self.forwarder.take() {
            let _ = forwarder.await;
        }
    }

    /// EC-1.1 seam: mid-session recognizer replacement (cloud drop -> local).
    pub fn recognizer_mut(&mut self) -> &mut R {
        self.pump.recognizer_mut()
    }

    /// EC-1.1: gated audio already delivered to the current recognizer.
    pub fn utterance_samples(&self) -> &[f32] {
        self.pump.utterance_samples()
    }

    pub fn push_samples(&mut self, samples: &[f32]) -> Result<(), Error> {
        if !self.started {
            return Err(Error::IllegalTransition(
                "audio before session start".into(),
            ));
        }
        self.pump.push_samples(samples)?;
        if self.pump.speech_observed() && !self.speech_reported {
            self.speech_reported = true;
            if let Some(observer) = &self.speech_observer {
                observer();
            }
        }
        Ok(())
    }

    pub fn set_speech_observer(&mut self, observer: std::sync::Arc<dyn Fn() + Send + Sync>) {
        self.speech_observer = Some(observer);
    }

    pub async fn service(&mut self) -> Result<(), Error> {
        if !self.started {
            return Err(Error::IllegalTransition(
                "service before session start".into(),
            ));
        }
        self.pump.service().await
    }

    pub async fn stop(&mut self, tail: &[f32]) -> Result<FinalTranscript, Error> {
        if !self.started {
            return Err(Error::IllegalTransition("stop before session start".into()));
        }
        let silence_only = self.pump.is_silence_only();
        let result = self
            .pump
            .finalize_with_tail(&tail[..tail.len().min(TAIL_SAMPLES)])
            .await;
        self.started = false;
        self.speech_reported = false;
        self.flush_forwarder().await;
        match result {
            Ok(transcript) => {
                if !silence_only {
                    self.events
                        .send(SessionTaskEvent::Final(transcript.clone()))
                        .await
                        .map_err(|_| Error::DbIo("session event receiver dropped".into()))?;
                } else {
                    self.events
                        .send(SessionTaskEvent::SilenceOnly)
                        .await
                        .map_err(|_| Error::DbIo("session event receiver dropped".into()))?;
                }
                Ok(transcript)
            }
            Err(error) => {
                let _ = self
                    .events
                    .send(SessionTaskEvent::Failed(error.clone()))
                    .await;
                Err(error)
            }
        }
    }

    pub async fn cancel(&mut self) -> Result<(), Error> {
        if self.started {
            self.pump.abort().await;
            self.started = false;
        }
        self.speech_reported = false;
        self.flush_forwarder().await;
        self.events
            .send(SessionTaskEvent::Cancelled)
            .await
            .map_err(|_| Error::DbIo("session event receiver dropped".into()))
    }

    /// Abort a live session after a capture/VAD/ASR polling failure and retain
    /// the failure as the terminal task event. This path deliberately avoids
    /// emitting `Cancelled`; the owner publishes the error lifecycle instead.
    pub async fn fail(&mut self, error: Error) {
        if self.started {
            self.pump.abort().await;
            self.started = false;
        }
        self.speech_reported = false;
        self.flush_forwarder().await;
        let _ = self.events.send(SessionTaskEvent::Failed(error)).await;
    }
}
