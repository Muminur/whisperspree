//! Session-task audio/VAD/ASR pump (FR-1.1, §5.3–§5.4).

use crate::{
    asr::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer},
    audio::{
        capture::CaptureProcessor,
        level::AudioLevel,
        ring_buffer::AudioRingBuffer,
        vad::{VadDecision, VadSegmenter, VAD_FRAME_SAMPLES},
        TARGET_SAMPLE_RATE_HZ,
    },
    error::Error,
};
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// Maximum release tail accepted by SM-4.
pub const TAIL_SAMPLES: usize = TARGET_SAMPLE_RATE_HZ as usize * 300 / 1_000;

/// Non-real-time bridge from the CPAL SPSC queue into the session pump. It
/// owns the stateful resampler and bounded decoder ring; callers invoke it on
/// the Tokio session task, never from the audio callback.
pub struct CapturePoller {
    processor: CaptureProcessor,
    ring: AudioRingBuffer,
}

impl CapturePoller {
    pub fn new(processor: CaptureProcessor, ring_capacity_samples: usize) -> Self {
        Self {
            processor,
            ring: AudioRingBuffer::new(ring_capacity_samples),
        }
    }

    pub fn poll(&mut self, now: std::time::Instant) -> Option<Vec<f32>> {
        self.poll_with_level(now).0
    }

    pub fn poll_with_level(
        &mut self,
        now: std::time::Instant,
    ) -> (Option<Vec<f32>>, Option<AudioLevel>) {
        let level = self.processor.poll(now, &mut self.ring);
        if self.ring.is_empty() {
            (None, level)
        } else {
            (Some(self.ring.pop_frames(self.ring.len())), level)
        }
    }
}

/// A small testable boundary around the VAD classifier. Production uses the
/// WebRTC segmenter; fixture tests can script decisions without depending on
/// host audio classification.
pub trait FrameGate {
    fn classify(&mut self, frame: &[f32]) -> Result<VadDecision, Error>;
    fn is_silence_only(&self) -> bool {
        false
    }
}

impl FrameGate for VadSegmenter {
    fn classify(&mut self, frame: &[f32]) -> Result<VadDecision, Error> {
        self.process_frame(frame)
            .map_err(|error| Error::AsrLoad(format!("VAD rejected audio frame: {error:?}")))
    }

    fn is_silence_only(&self) -> bool {
        VadSegmenter::is_silence_only(self)
    }
}

/// One session-task owner. It never runs on the CPAL callback: callers drain
/// capture samples on the Tokio session task and pass them here in batches.
pub struct SessionPump<R, G> {
    recognizer: R,
    gate: G,
    pending_samples: Vec<f32>,
    pre_speech: VecDeque<Vec<f32>>,
    in_speech: bool,
    speech_observed: bool,
    started: bool,
    /// EC-1.1: every gated frame actually fed to the recognizer, capped to the
    /// newest 30 s so a mid-session cloud->local swap can replay the utterance
    /// without unbounded memory.
    utterance: Vec<f32>,
}

/// 30 s of 16 kHz mono — the EC-1.1 replay ceiling.
const MAX_UTTERANCE_SAMPLES: usize = 16_000 * 30;

impl<R: SpeechRecognizer, G: FrameGate> SessionPump<R, G> {
    pub fn new(recognizer: R, gate: G) -> Self {
        Self {
            recognizer,
            gate,
            pending_samples: Vec::new(),
            pre_speech: VecDeque::with_capacity(3),
            in_speech: false,
            speech_observed: false,
            started: false,
            utterance: Vec::new(),
        }
    }

    pub async fn start(
        &mut self,
        config: AsrConfig,
        events: mpsc::Sender<AsrEvent>,
    ) -> Result<(), Error> {
        self.recognizer.start(config, events).await?;
        self.pending_samples.clear();
        self.pre_speech.clear();
        self.in_speech = false;
        self.speech_observed = false;
        self.started = true;
        Ok(())
    }

    /// Append 16 kHz mono samples and process every complete 30 ms frame.
    /// Partial frames remain owned by the session task for the next poll.
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<(), Error> {
        if !self.started {
            return Err(Error::IllegalTransition(
                "audio before recognizer start".into(),
            ));
        }
        self.pending_samples.extend_from_slice(samples);
        while self.pending_samples.len() >= VAD_FRAME_SAMPLES {
            let frame: Vec<f32> = self.pending_samples.drain(..VAD_FRAME_SAMPLES).collect();
            self.process_frame(frame)?;
        }
        Ok(())
    }

    pub async fn service(&mut self) -> Result<(), Error> {
        if !self.started {
            return Err(Error::IllegalTransition(
                "service before recognizer start".into(),
            ));
        }
        self.recognizer.service().await
    }

    fn process_frame(&mut self, frame: Vec<f32>) -> Result<(), Error> {
        let decision = self.gate.classify(&frame)?;
        match decision {
            VadDecision::Suppress if !self.in_speech => {
                if self.pre_speech.len() == 3 {
                    self.pre_speech.pop_front();
                }
                self.pre_speech.push_back(frame);
            }
            VadDecision::SpeechStarted { buffered_frames } => {
                self.in_speech = true;
                self.speech_observed = true;
                let buffered_frames = buffered_frames.saturating_sub(1) as usize;
                let skip = self.pre_speech.len().saturating_sub(buffered_frames);
                let priming: Vec<Vec<f32>> = self.pre_speech.drain(..).skip(skip).collect();
                for buffered in priming {
                    self.retain_utterance(&buffered);
                    self.recognizer.feed(&buffered)?;
                }
                self.retain_utterance(&frame);
                self.recognizer.feed(&frame)?;
            }
            VadDecision::Feed => {
                self.in_speech = true;
                self.speech_observed = true;
                self.retain_utterance(&frame);
                self.recognizer.feed(&frame)?;
            }
            VadDecision::Endpoint => {
                self.in_speech = false;
                self.pre_speech.clear();
                self.recognizer
                    .endpoint_silence(crate::audio::vad::ENDPOINT_SILENCE_MS)?;
            }
            VadDecision::Suppress => {}
        }
        Ok(())
    }

    /// EC-1.1 replay buffer: the gated frames the current recognizer received.
    pub fn utterance_samples(&self) -> &[f32] {
        &self.utterance
    }

    /// EC-1.1 seam: replace the recognizer mid-session (cloud drop -> local).
    pub fn recognizer_mut(&mut self) -> &mut R {
        &mut self.recognizer
    }

    fn retain_utterance(&mut self, frame: &[f32]) {
        self.utterance.extend_from_slice(frame);
        let excess = self.utterance.len().saturating_sub(MAX_UTTERANCE_SAMPLES);
        if excess > 0 {
            self.utterance.drain(..excess);
        }
    }

    pub fn speech_observed(&self) -> bool {
        self.speech_observed
    }

    /// Feed up to 300 ms of release tail directly to the recognizer, then
    /// drain its final transcript. Tail audio is intentionally exempt from
    /// VAD gating per SM-4.
    pub async fn finalize_with_tail(
        &mut self,
        tail_samples: &[f32],
    ) -> Result<FinalTranscript, Error> {
        if !self.started {
            return Err(Error::IllegalTransition(
                "finalize before recognizer start".into(),
            ));
        }
        if self.gate.is_silence_only() {
            self.recognizer.abort().await;
            self.started = false;
            self.pending_samples.clear();
            self.pre_speech.clear();
            self.in_speech = false;
            return Ok(FinalTranscript {
                text: String::new(),
                words: Vec::new(),
                language: None,
            });
        }
        let accepted = tail_samples.len().min(TAIL_SAMPLES);
        for chunk in tail_samples[..accepted].chunks(VAD_FRAME_SAMPLES) {
            if let Err(error) = self.recognizer.feed(chunk) {
                self.recognizer.abort().await;
                self.started = false;
                self.pending_samples.clear();
                self.pre_speech.clear();
                self.in_speech = false;
                return Err(error);
            }
        }
        let result = self.recognizer.finalize().await;
        self.started = false;
        self.pending_samples.clear();
        self.pre_speech.clear();
        self.in_speech = false;
        result
    }

    pub fn is_silence_only(&self) -> bool {
        self.gate.is_silence_only()
    }

    pub async fn abort(&mut self) {
        self.recognizer.abort().await;
        self.started = false;
        self.pending_samples.clear();
        self.pre_speech.clear();
        self.in_speech = false;
    }
}

/// Convenience constructor for the production WebRTC VAD gate.
pub fn local_vad_gate() -> VadSegmenter {
    VadSegmenter::new()
}
