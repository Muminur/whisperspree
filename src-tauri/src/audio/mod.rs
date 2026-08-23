//! Audio capture primitives (`cpal` → resample → mono f32) for milestone 1.
//!
//! T1.1 is intentionally narrow: capture device enumeration, raw sample
//! normalization to mono f32, deterministic resampling helpers, ring-buffer
//! behavior, and level measurement utilities that power `audio:level`.
//! Real audio streaming remains RT-safe in `capture.rs` utilities and is extended
//! in later milestones when the session engine owns the stream lifecycle.

pub mod capture;
pub mod level;
pub mod resample;
pub mod ring_buffer;
pub mod vad;

pub use capture::{
    capture_queue, list_input_devices, open_input_stream, select_input_device_id, AudioInputDevice,
    CaptureConsumer, CaptureProcessor, CaptureProducer, CaptureStream,
};

pub const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;
pub const TARGET_CHANNELS: usize = 1;
