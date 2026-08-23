//! T1.1 capture-facing helpers.
//!
//! The CPAL callback is deliberately limited to an allocation-free, lock-free
//! hand-off. Resampling and `AudioRingBuffer` ownership stay on the session
//! task, where the existing helpers may allocate safely.

use std::{
    cell::UnsafeCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleFormat, Stream, StreamConfig,
};
use serde::{Deserialize, Serialize};

use super::{
    level::{calculate_level, should_emit_level, AudioLevel},
    resample::StreamingResampler,
    ring_buffer::AudioRingBuffer,
};

/// Publicly exposed input-device shape (`list_input_devices` RPC payload).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioInputDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Single-producer/single-consumer queue used between CPAL's real-time thread
/// and the session task. The producer drops new frames rather than blocking
/// when the consumer falls behind.
///
/// Exactly one producer may call [`Self::push_interleaved`] and exactly one
/// consumer may call [`Self::drain_into`]. Both operations are allocation-free.
struct CaptureQueue {
    samples: Box<[UnsafeCell<f32>]>,
    capacity: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicUsize,
}

// SAFETY: entries are only written by the designated producer before `head` is
// published (Release), and only read by the designated consumer after observing
// `head` (Acquire). The consumer publishes `tail` after its reads (Release).
unsafe impl Sync for CaptureQueue {}

impl CaptureQueue {
    fn new(capacity_frames: usize) -> Self {
        assert!(
            capacity_frames > 0,
            "capture queue capacity must be non-zero"
        );
        let samples = std::iter::repeat_with(|| UnsafeCell::new(0.0))
            .take(capacity_frames)
            .collect();

        Self {
            samples,
            capacity: capacity_frames,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        }
    }

    /// Converts interleaved input to mono and hands it to the session task.
    /// It does not allocate, lock, log, or invoke user code.
    fn push_interleaved<T>(&self, input: &[T], channels: u16)
    where
        T: Sample,
        f32: cpal::FromSample<T>,
    {
        let channels = usize::from(channels.max(1));
        for frame in input.chunks_exact(channels) {
            let mut sum = 0.0;
            for sample in frame {
                sum += sample.to_sample::<f32>();
            }
            self.push_mono(sum / channels as f32);
        }
    }

    fn push_mono(&self, sample: f32) {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.capacity {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let index = head % self.capacity;
        // SAFETY: the SPSC contract means this producer has exclusive access
        // to this slot until it publishes the new head below.
        unsafe { *self.samples[index].get() = sample };
        self.head.store(head.wrapping_add(1), Ordering::Release);
    }

    /// Moves all currently available native-rate mono samples into a buffer
    /// owned by the non-real-time session task.
    fn drain_into(&self, output: &mut Vec<f32>) {
        let mut tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        while tail != head {
            let index = tail % self.capacity;
            // SAFETY: acquiring `head` observes the producer's write, and the
            // producer cannot reuse this slot until `tail` is published below.
            output.push(unsafe { *self.samples[index].get() });
            tail = tail.wrapping_add(1);
        }
        self.tail.store(tail, Ordering::Release);
    }

    fn dropped_frames(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The sole handle that may write to a capture queue. It is moved into CPAL's
/// callback and intentionally has no `Clone` implementation.
pub struct CaptureProducer {
    queue: Arc<CaptureQueue>,
}

impl CaptureProducer {
    /// Converts interleaved samples to mono and enqueues them without
    /// allocation, locking, logging, or calling user code.
    pub fn push_interleaved<T>(&mut self, input: &[T], channels: u16)
    where
        T: Sample,
        f32: cpal::FromSample<T>,
    {
        self.queue.push_interleaved(input, channels);
    }
}

/// The sole handle that may drain capture frames. It is owned by the
/// non-real-time session task and deliberately has no `Clone` implementation.
pub struct CaptureConsumer {
    queue: Arc<CaptureQueue>,
}

impl CaptureConsumer {
    pub fn drain_into(&mut self, output: &mut Vec<f32>) {
        self.queue.drain_into(output);
    }

    pub fn dropped_frames(&self) -> usize {
        self.queue.dropped_frames()
    }
}

/// Creates the one producer and one consumer required by the capture queue.
/// Neither endpoint can be cloned, structurally preserving the SPSC invariant.
pub fn capture_queue(capacity_frames: usize) -> (CaptureProducer, CaptureConsumer) {
    let queue = Arc::new(CaptureQueue::new(capacity_frames));
    (
        CaptureProducer {
            queue: Arc::clone(&queue),
        },
        CaptureConsumer { queue },
    )
}

/// Non-real-time capture processing for the future session task. It drains the
/// callback queue, performs stateful 16 kHz resampling, appends to the decoder
/// ring, and returns an at-most-20-Hz level measurement.
pub struct CaptureProcessor {
    consumer: CaptureConsumer,
    resampler: StreamingResampler,
    native_frames: Vec<f32>,
    resampled_frames: Vec<f32>,
    last_level_at: Option<std::time::Instant>,
}

impl CaptureProcessor {
    pub fn new(consumer: CaptureConsumer, input_rate_hz: u32) -> Self {
        Self {
            consumer,
            resampler: StreamingResampler::new(input_rate_hz),
            native_frames: Vec::new(),
            resampled_frames: Vec::new(),
            last_level_at: None,
        }
    }

    pub fn poll(
        &mut self,
        now: std::time::Instant,
        decoder_ring: &mut AudioRingBuffer,
    ) -> Option<AudioLevel> {
        self.native_frames.clear();
        self.consumer.drain_into(&mut self.native_frames);
        if self.native_frames.is_empty() {
            return None;
        }

        self.resampled_frames.clear();
        self.resampler
            .resample_into(&self.native_frames, &mut self.resampled_frames);
        if self.resampled_frames.is_empty() {
            return None;
        }

        decoder_ring.push_frames(&self.resampled_frames);
        if should_emit_level(now, self.last_level_at) {
            self.last_level_at = Some(now);
            Some(calculate_level(&self.resampled_frames))
        } else {
            None
        }
    }
}

/// An active CPAL input stream. Calling [`Self::stop`] or dropping this handle
/// stops capture. Its paired [`CaptureConsumer`] is owned by the session task.
pub struct CaptureStream {
    _stream: Stream,
    sample_rate_hz: u32,
    stream_error: Arc<AtomicBool>,
}

impl CaptureStream {
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Returns whether CPAL reported a stream error. Session ownership maps
    /// this to the PRD's `MIC-DEV` transition in T1.5.
    pub fn has_stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Acquire)
    }

    /// Explicit lifecycle endpoint; dropping the returned handle closes CPAL's
    /// stream immediately.
    pub fn stop(self) {}
}

fn dedupe_names(names: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut seen = HashMap::<String, usize>::new();
    let mut out = Vec::new();

    for name in names {
        let idx = seen.entry(name.clone()).or_insert(0);
        let id_suffix = if *idx == 0 {
            String::new()
        } else {
            format!("#{idx}")
        };
        *idx += 1;
        out.push((format!("{name}{id_suffix}"), *idx));
    }

    out
}

fn canonicalize_device_names(
    names: Vec<String>,
    default_name: Option<&str>,
) -> Vec<AudioInputDevice> {
    let deduped = dedupe_names(names.into_iter());

    let mut devices: Vec<AudioInputDevice> = deduped
        .into_iter()
        .map(|(id, occurrence)| {
            let name = id
                .split_once('#')
                .map(|(base, _)| base.to_string())
                .unwrap_or_else(|| id.clone());

            let default = default_name == Some(name.as_str()) && occurrence == 1;

            AudioInputDevice { id, name, default }
        })
        .collect();

    devices.sort_by(|a, b| {
        use std::cmp::Ordering;

        match (a.default, b.default) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });

    devices
}

pub fn to_mono_f32<T>(input: &[T], channels: u16) -> Vec<f32>
where
    T: Sample,
    f32: cpal::FromSample<T>,
{
    if channels <= 1 {
        return input.iter().copied().map(Sample::to_sample).collect();
    }

    let channels = channels as usize;
    let frame_count = input.len() / channels;
    let mut out = Vec::with_capacity(frame_count);

    for frame in 0..frame_count {
        let base = frame * channels;
        let mut sum = 0.0f32;

        for ch in 0..channels {
            sum += input[base + ch].to_sample::<f32>();
        }

        out.push(sum / channels as f32);
    }

    out
}

/// Enumerate input devices from `cpal`, returning display-safe IDs and the
/// default-device flag.
pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, String> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());

    let names: Vec<String> = host
        .input_devices()
        .map_err(|error| error.to_string())?
        .filter_map(|device| device.name().ok())
        .collect();

    Ok(canonicalize_device_names(names, default_name.as_deref()))
}

/// Selects the requested input ID, or the enumerated default when no explicit
/// selection is configured. Kept pure so device preference behavior is tested
/// without requiring a microphone.
pub fn select_input_device_id(
    devices: &[AudioInputDevice],
    requested_id: Option<&str>,
) -> Result<String, String> {
    if let Some(id) = requested_id {
        return devices
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.id.clone())
            .ok_or_else(|| format!("input device '{id}' is unavailable"));
    }

    devices
        .iter()
        .find(|device| device.default)
        .or_else(|| devices.first())
        .map(|device| device.id.clone())
        .ok_or_else(|| "no input devices are available".to_string())
}

/// Opens and starts a real CPAL input stream. The callback only normalizes to
/// mono and copies into its [`CaptureProducer`]; it intentionally performs no
/// resampling, allocation, locking, or logging.
pub fn open_input_stream(
    requested_device_id: Option<&str>,
    queue_capacity_frames: usize,
) -> Result<(CaptureStream, CaptureConsumer), String> {
    let host = cpal::default_host();
    let device = resolve_input_device(&host, requested_device_id)?;
    let device_name = device.name().unwrap_or_else(|_| "input device".to_string());
    let supported = device
        .default_input_config()
        .map_err(|error| format!("could not open {device_name}: {error}"))?;
    let sample_rate_hz = supported.sample_rate().0;
    let config: StreamConfig = supported.config();
    let (producer, consumer) = capture_queue(queue_capacity_frames);
    let stream_error = Arc::new(AtomicBool::new(false));

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, producer, &stream_error),
        SampleFormat::I16 => build_stream::<i16>(&device, &config, producer, &stream_error),
        SampleFormat::U16 => build_stream::<u16>(&device, &config, producer, &stream_error),
        format => return Err(format!("unsupported input sample format: {format:?}")),
    }?;
    stream
        .play()
        .map_err(|error| format!("could not start {device_name}: {error}"))?;

    Ok((
        CaptureStream {
            _stream: stream,
            sample_rate_hz,
            stream_error,
        },
        consumer,
    ))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut producer: CaptureProducer,
    stream_error: &Arc<AtomicBool>,
) -> Result<Stream, String>
where
    T: Sample + cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let stream_error_callback = Arc::clone(stream_error);
    let channels = config.channels;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| producer.push_interleaved(data, channels),
            move |_| stream_error_callback.store(true, Ordering::Release),
            None,
        )
        .map_err(|error| error.to_string())
}

fn resolve_input_device(
    host: &cpal::Host,
    requested_device_id: Option<&str>,
) -> Result<cpal::Device, String> {
    let Some(requested_id) = requested_device_id else {
        return host
            .default_input_device()
            .ok_or_else(|| "no default input device is available".to_string());
    };

    let mut seen = HashMap::<String, usize>::new();
    for device in host.input_devices().map_err(|error| error.to_string())? {
        let name = device.name().map_err(|error| error.to_string())?;
        let occurrence = seen.entry(name.clone()).or_insert(0);
        let id = if *occurrence == 0 {
            name
        } else {
            format!("{name}#{occurrence}")
        };
        *occurrence += 1;
        if id == requested_id {
            return Ok(device);
        }
    }

    Err(format!("input device '{requested_id}' is unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_capture_to_mono_f32_stereo_averages_channels() {
        let raw = [30000i16, 10000, -20000, -10000, 5000, 7000];
        let mono = to_mono_f32(&raw, 2);

        let expected0 = (raw[0].to_sample::<f32>() + raw[1].to_sample::<f32>()) / 2.0;
        let expected1 = (raw[2].to_sample::<f32>() + raw[3].to_sample::<f32>()) / 2.0;
        let expected2 = (raw[4].to_sample::<f32>() + raw[5].to_sample::<f32>()) / 2.0;

        assert_eq!(mono.len(), 3);
        assert!((mono[0] - expected0).abs() < 0.0001);
        assert!((mono[1] - expected1).abs() < 0.0001);
        assert!((mono[2] - expected2).abs() < 0.0001);
    }

    #[test]
    fn fr_1_1_list_input_devices_dedupes_duplicate_names_and_marks_default() {
        let names = vec![
            "Studio Mic".to_string(),
            "Studio Mic".to_string(),
            "Headset".to_string(),
        ];

        let devices = canonicalize_device_names(names, Some("Studio Mic"));
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].id, "Studio Mic");
        assert_eq!(devices[1].id, "Headset");
        assert_eq!(devices[2].id, "Studio Mic#1");

        assert!(devices[0].default);
        assert!(!devices[1].default);
        assert!(!devices[2].default);
    }

    #[test]
    fn fr_1_1_selected_device_id_prefers_requested_input() {
        let devices = vec![
            AudioInputDevice {
                id: "Built-in Microphone".to_string(),
                name: "Built-in Microphone".to_string(),
                default: true,
            },
            AudioInputDevice {
                id: "USB Microphone".to_string(),
                name: "USB Microphone".to_string(),
                default: false,
            },
        ];

        assert_eq!(
            select_input_device_id(&devices, Some("USB Microphone")).unwrap(),
            "USB Microphone"
        );
        assert_eq!(
            select_input_device_id(&devices, None).unwrap(),
            "Built-in Microphone"
        );
        assert!(select_input_device_id(&devices, Some("missing")).is_err());
    }

    #[test]
    fn fr_1_1_callback_queue_normalizes_stereo_without_allocation() {
        let (mut producer, mut consumer) = capture_queue(4);
        let input = [1.0f32, -1.0, 0.5, 0.0, -0.5, -0.5];

        producer.push_interleaved(&input, 2);

        let mut frames = Vec::with_capacity(3);
        consumer.drain_into(&mut frames);
        assert_eq!(frames, vec![0.0, 0.25, -0.5]);
        assert_eq!(consumer.dropped_frames(), 0);
    }

    #[test]
    fn fr_1_1_callback_queue_is_bounded_and_drops_new_frames_when_consumer_lags() {
        let (mut producer, mut consumer) = capture_queue(2);
        producer.push_interleaved(&[0.1f32, 0.2, 0.3], 1);

        let mut frames = Vec::with_capacity(2);
        consumer.drain_into(&mut frames);
        assert_eq!(frames, vec![0.1, 0.2]);
        assert_eq!(consumer.dropped_frames(), 1);
    }

    #[test]
    fn fr_1_1_capture_to_mono_f32_converts_unsigned_samples() {
        let mono = to_mono_f32(&[0u16, u16::MAX], 2);
        assert_eq!(mono.len(), 1);
        assert!(
            mono[0].abs() < 0.000_1,
            "unsigned stereo should average to zero"
        );
    }

    #[test]
    fn fr_1_1_capture_processor_resamples_into_ring_and_throttles_levels() {
        let (mut producer, consumer) = capture_queue(4_096);
        let input: Vec<f32> = (0..2_048)
            .map(|frame| (frame as f32 * std::f32::consts::TAU / 32.0).sin())
            .collect();
        producer.push_interleaved(&input, 1);

        let mut processor = CaptureProcessor::new(consumer, 32_000);
        let mut ring = crate::audio::ring_buffer::AudioRingBuffer::new(2_048);
        let start = std::time::Instant::now();
        let level = processor.poll(start, &mut ring).expect("level is emitted");
        assert!(level.rms > 0.1);
        assert!(level.peak > 0.5);
        assert!(
            (900..=1_000).contains(&ring.len()),
            "Rubato output should approach the 2:1 rate after filter warmup, got {}",
            ring.len()
        );
        assert!(processor.poll(start, &mut ring).is_none());
    }
}
