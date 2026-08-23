//! Frame-level RMS/peak helpers for FR-1.1.b/c + HUD `audio:level` pacing.

use std::time::{Duration, Instant};

pub const LEVEL_EVENT_RATE_HZ: u64 = 20;
pub const LEVEL_EVENT_INTERVAL: Duration = Duration::from_millis(1000 / LEVEL_EVENT_RATE_HZ);

#[derive(Debug, Clone, Copy)]
pub struct AudioLevel {
    pub rms: f32,
    pub peak: f32,
}

pub fn calculate_level(samples: &[f32]) -> AudioLevel {
    if samples.is_empty() {
        return AudioLevel {
            rms: 0.0,
            peak: 0.0,
        };
    }

    let mut sum_sq = 0.0f32;
    let mut peak = 0.0f32;

    for sample in samples {
        let abs = sample.abs();
        if abs > peak {
            peak = abs;
        }

        sum_sq += sample * sample;
    }

    AudioLevel {
        rms: (sum_sq / samples.len() as f32).sqrt(),
        peak,
    }
}

pub fn should_emit_level(now: Instant, last_emitted_at: Option<Instant>) -> bool {
    if let Some(last) = last_emitted_at {
        now.duration_since(last) >= LEVEL_EVENT_INTERVAL
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_level_from_known_samples_is_deterministic() {
        let level = calculate_level(&[1.0, -1.0, 0.0, 1.0, -1.0]);

        assert_eq!(level.peak, 1.0);
        assert!(
            (level.rms - 0.894_427_2).abs() < 0.000_01,
            "unexpected RMS: {}",
            level.rms
        );
    }

    #[test]
    fn fr_1_1_level_emits_at_or_below_20hz_when_throttled() {
        let start = Instant::now();
        assert!(should_emit_level(start, None));
        assert!(!should_emit_level(
            start + LEVEL_EVENT_INTERVAL / 2,
            Some(start)
        ));
        assert!(should_emit_level(start + LEVEL_EVENT_INTERVAL, Some(start)));
    }
}
