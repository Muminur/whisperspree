//! T1.1 ring-buffer tests for capture→decoder handoff.
//!
//! This is a bounded ring over one-channel `f32` samples. The milestone's
//! constraints currently require deterministic overwrite behavior and deterministic
//! frame draining; RT-thread ownership comes in later milestones.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct AudioRingBuffer {
    samples: VecDeque<f32>,
    cap_frames: usize,
}

impl AudioRingBuffer {
    pub fn new(cap_frames: usize) -> Self {
        assert!(cap_frames > 0, "ring buffer capacity must be non-zero");
        Self {
            samples: VecDeque::with_capacity(cap_frames),
            cap_frames,
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn cap_frames(&self) -> usize {
        self.cap_frames
    }

    pub fn is_full(&self) -> bool {
        self.samples.len() == self.cap_frames
    }

    pub fn push_frame(&mut self, sample: f32) {
        self.push_frames(std::slice::from_ref(&sample));
    }

    pub fn push_frames(&mut self, batch: &[f32]) {
        for &sample in batch {
            if self.samples.len() == self.cap_frames {
                self.samples.pop_front();
            }

            self.samples.push_back(sample);
        }
    }

    pub fn pop_frames(&mut self, want: usize) -> Vec<f32> {
        let actual = want.min(self.samples.len());
        let mut out = Vec::with_capacity(actual);

        for _ in 0..actual {
            out.push(self.samples.pop_front().expect("ring-buffer underflow"));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_ring_buffer_is_bounded_and_overwrites_oldest() {
        let mut ring = AudioRingBuffer::new(4);
        ring.push_frames(&[0.0, 1.0, 2.0, 3.0]);
        ring.push_frames(&[4.0, 5.0]);

        assert_eq!(ring.len(), 4);
        assert_eq!(ring.pop_frames(4), vec![2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn fr_1_1_ring_buffer_preserves_fifo_on_partial_drain() {
        let mut ring = AudioRingBuffer::new(5);
        ring.push_frames(&[10.0, 11.0, 12.0]);
        assert_eq!(ring.pop_frames(2), vec![10.0, 11.0]);
        ring.push_frames(&[13.0, 14.0, 15.0]);

        assert_eq!(ring.pop_frames(4), vec![12.0, 13.0, 14.0, 15.0]);
    }

    #[test]
    #[should_panic(expected = "ring buffer capacity must be non-zero")]
    fn fr_1_1_ring_buffer_rejects_zero_capacity() {
        let _ = AudioRingBuffer::new(0);
    }
}
