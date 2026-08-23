//! WebRTC VAD framing and speech-gating rules for dictation audio.
//!
//! The audio callback only delivers resampled samples.  The session task owns
//! this segmenter and calls it once per 30 ms frame, keeping non-speech out of
//! its ASR input until speech has been confirmed.

use webrtc_vad::{SampleRate, Vad, VadMode};

use super::TARGET_SAMPLE_RATE_HZ;

/// The only frame duration accepted by the dictation VAD pipeline.
pub const VAD_FRAME_MS: u32 = 30;
/// Number of 16 kHz samples in one 30 ms VAD frame.
pub const VAD_FRAME_SAMPLES: usize =
    (TARGET_SAMPLE_RATE_HZ as usize * VAD_FRAME_MS as usize) / 1_000;
/// Three consecutive 30 ms speech frames are required to start speech.
pub const SPEECH_START_FRAMES: u32 = 3;
/// A detected utterance ends after at least 700 ms without speech.
pub const ENDPOINT_SILENCE_MS: u32 = 700;
/// Frames are discrete, so 700 ms requires 24 30 ms frames (720 ms).
pub const ENDPOINT_SILENCE_FRAMES: u32 = ENDPOINT_SILENCE_MS.div_ceil(VAD_FRAME_MS);
/// Sessions with less speech than this are treated as silence-only.
pub const MIN_SPEECH_MS: u32 = 600;
pub const MIN_SPEECH_FRAMES: u32 = MIN_SPEECH_MS.div_ceil(VAD_FRAME_MS);

/// The effect of one VAD frame on the session's ASR input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// The frame is non-speech, or speech has not yet passed the start gate.
    Suppress,
    /// Speech has started.  The caller must flush this many buffered frames to ASR.
    SpeechStarted { buffered_frames: u32 },
    /// The current speech frame belongs in the ASR input.
    Feed,
    /// The current run of non-speech reached the endpoint threshold.
    Endpoint,
}

/// Errors returned when a frame cannot be evaluated by WebRTC VAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadError {
    InvalidFrameLength { actual_samples: usize },
    DetectorRejectedFrame,
}

/// WebRTC VAD plus the FR-1.1.c start and endpoint state machine.
pub struct VadSegmenter {
    detector: Vad,
    speech_run: u32,
    silence_run: u32,
    total_speech_frames: u32,
    in_speech: bool,
}

impl VadSegmenter {
    /// Creates a 16 kHz WebRTC VAD in the required aggressiveness-2 mode.
    pub fn new() -> Self {
        Self {
            detector: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive),
            speech_run: 0,
            silence_run: 0,
            total_speech_frames: 0,
            in_speech: false,
        }
    }

    /// Classifies one 30 ms mono f32 frame and applies the speech gate.
    pub fn process_frame(&mut self, frame: &[f32]) -> Result<VadDecision, VadError> {
        if frame.len() != VAD_FRAME_SAMPLES {
            return Err(VadError::InvalidFrameLength {
                actual_samples: frame.len(),
            });
        }

        let pcm: Vec<i16> = frame.iter().map(|&sample| f32_to_pcm16(sample)).collect();
        let is_speech = self
            .detector
            .is_voice_segment(&pcm)
            .map_err(|()| VadError::DetectorRejectedFrame)?;

        Ok(self.process_speech_decision(is_speech))
    }

    /// Applies a WebRTC speech decision to the start/endpoint gate.
    ///
    /// This is public so deterministic audio pipelines can apply recorded VAD
    /// decisions without replacing the production WebRTC detector.
    pub fn process_speech_decision(&mut self, is_speech: bool) -> VadDecision {
        if !self.in_speech {
            if is_speech {
                self.speech_run += 1;
                if self.speech_run == SPEECH_START_FRAMES {
                    self.in_speech = true;
                    self.total_speech_frames += SPEECH_START_FRAMES;
                    self.silence_run = 0;
                    return VadDecision::SpeechStarted {
                        buffered_frames: SPEECH_START_FRAMES,
                    };
                }
            } else {
                self.speech_run = 0;
            }

            return VadDecision::Suppress;
        }

        if is_speech {
            self.silence_run = 0;
            self.total_speech_frames += 1;
            return VadDecision::Feed;
        }

        self.silence_run += 1;
        if self.silence_run >= ENDPOINT_SILENCE_FRAMES {
            self.in_speech = false;
            self.speech_run = 0;
            self.silence_run = 0;
            VadDecision::Endpoint
        } else {
            VadDecision::Suppress
        }
    }

    /// Returns whether the session had less than the 600 ms speech minimum.
    pub fn is_silence_only(&self) -> bool {
        self.total_speech_frames < MIN_SPEECH_FRAMES
    }

    /// Clears gating and speech-duration state for the next dictation session.
    pub fn reset(&mut self) {
        self.detector.reset();
        self.speech_run = 0;
        self.silence_run = 0;
        self.total_speech_frames = 0;
        self.in_speech = false;
    }
}

impl Default for VadSegmenter {
    fn default() -> Self {
        Self::new()
    }
}

fn f32_to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segmenter() -> VadSegmenter {
        VadSegmenter::new()
    }

    fn start_speech(vad: &mut VadSegmenter) {
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(
            vad.process_speech_decision(true),
            VadDecision::SpeechStarted {
                buffered_frames: SPEECH_START_FRAMES
            }
        );
    }

    #[test]
    fn fr_1_1_c_requires_three_consecutive_speech_frames_before_starting() {
        let mut vad = segmenter();

        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(
            vad.process_speech_decision(true),
            VadDecision::SpeechStarted {
                buffered_frames: SPEECH_START_FRAMES
            }
        );
    }

    #[test]
    fn fr_1_1_c_suppresses_non_speech_and_feeds_confirmed_speech() {
        let mut vad = segmenter();

        assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
        start_speech(&mut vad);
        assert_eq!(vad.process_speech_decision(true), VadDecision::Feed);
        assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
    }

    #[test]
    fn fr_1_1_c_ends_only_after_700ms_of_non_speech() {
        let mut vad = segmenter();
        start_speech(&mut vad);

        for _ in 0..ENDPOINT_SILENCE_FRAMES - 1 {
            assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
        }
        assert_eq!(vad.process_speech_decision(false), VadDecision::Endpoint);
    }

    #[test]
    fn fr_1_1_c_speech_before_endpoint_resets_the_silence_timer() {
        let mut vad = segmenter();
        start_speech(&mut vad);

        for _ in 0..ENDPOINT_SILENCE_FRAMES - 1 {
            assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
        }
        assert_eq!(vad.process_speech_decision(true), VadDecision::Feed);
        for _ in 0..ENDPOINT_SILENCE_FRAMES - 1 {
            assert_eq!(vad.process_speech_decision(false), VadDecision::Suppress);
        }
        assert_eq!(vad.process_speech_decision(false), VadDecision::Endpoint);
    }

    #[test]
    fn fr_1_1_c_under_600ms_of_speech_is_silence_only() {
        let mut vad = segmenter();
        start_speech(&mut vad);

        for _ in SPEECH_START_FRAMES..MIN_SPEECH_FRAMES - 1 {
            assert_eq!(vad.process_speech_decision(true), VadDecision::Feed);
        }
        assert!(vad.is_silence_only());
    }

    #[test]
    fn fr_1_1_c_600ms_of_speech_is_not_silence_only() {
        let mut vad = segmenter();
        start_speech(&mut vad);

        for _ in SPEECH_START_FRAMES..MIN_SPEECH_FRAMES {
            assert_eq!(vad.process_speech_decision(true), VadDecision::Feed);
        }
        assert!(!vad.is_silence_only());
    }

    #[test]
    fn fr_1_1_c_rejects_non_30ms_frames() {
        let mut vad = segmenter();
        let frame = vec![0.0; VAD_FRAME_SAMPLES - 1];

        assert_eq!(
            vad.process_frame(&frame),
            Err(VadError::InvalidFrameLength {
                actual_samples: VAD_FRAME_SAMPLES - 1
            })
        );
    }

    #[test]
    fn fr_1_1_c_uses_a_30ms_16khz_frame_with_the_webrtc_detector() {
        let mut vad = segmenter();
        let silent_frame = vec![0.0; VAD_FRAME_SAMPLES];

        assert_eq!(vad.process_frame(&silent_frame), Ok(VadDecision::Suppress));
    }

    #[test]
    fn fr_1_1_c_reset_clears_start_gate_and_speech_duration() {
        let mut vad = segmenter();
        start_speech(&mut vad);
        assert!(vad.is_silence_only());

        vad.reset();

        assert!(vad.is_silence_only());
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
        assert_eq!(vad.process_speech_decision(true), VadDecision::Suppress);
    }
}
