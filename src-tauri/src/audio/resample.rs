//! T1.1 resampling helpers.
//!
//! This module uses [`rubato`] for conversion to 16 kHz.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

const TARGET_RATE_HZ: usize = 16_000;
const SILENT_GUARD: f64 = 1.1;
const SINC_LEN: usize = 256;
const F_CUTOFF: f32 = 0.95;
const OVERSAMPLING_FACTOR: usize = 128;
const MIN_CHUNK_SIZE: usize = 256;

/// Stateful Rubato converter for the session-side capture processor. It keeps
/// one `SincFixedIn` filter alive across queue drains, so independently sized
/// CPAL callbacks cannot reset the filter or introduce boundary artifacts.
pub struct StreamingResampler {
    input_rate_hz: u32,
    resampler: Option<SincFixedIn<f32>>,
    pending: Vec<f32>,
    output_scratch: Vec<f32>,
}

impl StreamingResampler {
    pub fn new(input_rate_hz: u32) -> Self {
        assert!(input_rate_hz > 0, "input sample rate must be non-zero");
        let resampler = (input_rate_hz != TARGET_RATE_HZ as u32).then(|| {
            SincFixedIn::<f32>::new(
                TARGET_RATE_HZ as f64 / input_rate_hz as f64,
                SILENT_GUARD,
                SincInterpolationParameters {
                    sinc_len: SINC_LEN,
                    f_cutoff: F_CUTOFF,
                    oversampling_factor: OVERSAMPLING_FACTOR,
                    interpolation: SincInterpolationType::Linear,
                    window: WindowFunction::BlackmanHarris2,
                },
                MIN_CHUNK_SIZE,
                1,
            )
            .expect("valid fixed-rate SincFixedIn configuration")
        });
        let output_capacity = resampler
            .as_ref()
            .map(Resampler::output_frames_next)
            .unwrap_or_default();
        Self {
            input_rate_hz,
            resampler,
            pending: Vec::with_capacity(MIN_CHUNK_SIZE),
            output_scratch: vec![0.0; output_capacity],
        }
    }

    /// Appends 16 kHz output to `output`. This runs on
    /// the session task, not CPAL's callback; the latter only copies samples.
    pub fn resample_into(&mut self, input: &[f32], output: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }

        if self.input_rate_hz == TARGET_RATE_HZ as u32 {
            output.extend_from_slice(input);
            return;
        }

        self.pending.extend_from_slice(input);
        let resampler = self
            .resampler
            .as_mut()
            .expect("non-16k streams have a Rubato resampler");
        let needed = resampler.input_frames_next();
        while self.pending.len() >= needed {
            let input = [self.pending[..needed].as_ref()];
            let mut output_buffers = [self.output_scratch.as_mut_slice()];
            let (_, written) = resampler
                .process_into_buffer(&input, &mut output_buffers, None)
                .expect("preallocated Rubato buffers match fixed configuration");
            output.extend_from_slice(&self.output_scratch[..written]);
            self.pending.drain(..needed);
        }
    }
}

/// Resample `samples` from `input_rate_hz` to 16 kHz mono using rubato.
pub fn resample_to_16k_hz(samples: &[f32], input_rate_hz: u32) -> Vec<f32> {
    if input_rate_hz == TARGET_RATE_HZ as u32 {
        return samples.to_vec();
    }

    if samples.is_empty() || input_rate_hz == 0 {
        return Vec::new();
    }

    let ratio = TARGET_RATE_HZ as f64 / input_rate_hz as f64;
    // Add one filter-length of zero padding so rubato's edge handling does not
    // shorten the usable signal by the sinc kernel's group delay.
    let chunk_size = samples.len().max(MIN_CHUNK_SIZE) + SINC_LEN;
    let mut padded = vec![0.0f32; chunk_size];
    padded[..samples.len()].copy_from_slice(samples);
    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        SILENT_GUARD,
        SincInterpolationParameters {
            sinc_len: SINC_LEN,
            f_cutoff: F_CUTOFF,
            oversampling_factor: OVERSAMPLING_FACTOR,
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::BlackmanHarris2,
        },
        chunk_size,
        1,
    )
    .unwrap_or_else(|err| panic!("rubato SincFixedIn construction failed: {err}"));

    let input = vec![padded.as_slice()];
    let output = resampler
        .process(&input, None)
        .unwrap_or_else(|err| panic!("rubato process failed: {err}"));

    if output.is_empty() {
        return Vec::new();
    }
    let mut out = output[0].clone();
    let expected_len =
        (samples.len() as f64 * (TARGET_RATE_HZ as f64) / input_rate_hz as f64).round() as usize;
    out.resize(expected_len, 0.0);
    out.truncate(expected_len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_440(samples: usize, sample_rate_hz: usize) -> Vec<f32> {
        (0..samples)
            .map(|n| {
                ((n as f64 * 440.0 * 2.0 * std::f64::consts::PI / sample_rate_hz as f64).sin())
                    as f32
            })
            .collect()
    }

    #[test]
    fn fr_1_1_resample_sine_from_32000_to_16000_is_length_reasonable() {
        let input = sine_440(32000, 32_000);
        let output = resample_to_16k_hz(&input, 32_000);

        let expected = ((input.len() as f64) * (TARGET_RATE_HZ as f64) / 32_000.0).round() as usize;
        let len_delta = output.len().abs_diff(expected);
        assert!(
            len_delta <= 1,
            "output length {} != {expected}",
            output.len()
        );

        let (sum, sum_abs) = output.iter().fold((0.0f64, 0.0f64), |acc, sample| {
            (acc.0 + *sample as f64, acc.1 + sample.abs() as f64)
        });

        assert!(sum.abs() < 0.5, "sine mean should remain near 0, got {sum}");
        assert!(
            sum_abs > output.len() as f64 * 0.05,
            "resampled sine must keep energy, got {sum_abs}"
        );
    }

    #[test]
    fn fr_1_1_resample_noop_when_already_16k() {
        let input = vec![0.0, 0.25, -0.25, 1.0, -1.0];
        assert_eq!(resample_to_16k_hz(&input, 16_000), input);
    }

    #[test]
    fn fr_1_1_streaming_resampler_preserves_rate_across_drains() {
        let mut resampler = StreamingResampler::new(32_000);
        let mut output = Vec::new();
        let input: Vec<f32> = (0..(MIN_CHUNK_SIZE * 2)).map(|n| n as f32).collect();
        resampler.resample_into(&input[..MIN_CHUNK_SIZE], &mut output);
        resampler.resample_into(&input[MIN_CHUNK_SIZE..], &mut output);

        assert!(
            output.len() > 180,
            "a persistent Rubato filter must emit past its first-chunk warmup, got {}",
            output.len()
        );
    }
}
