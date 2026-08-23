//! Production CPAL microphone ownership for the session boundary.

use crate::{audio, error::Error, pipeline::Microphone};

/// Owns the CPAL stream and its single consumer for exactly one session. The
/// callback remains in `audio::capture`; this adapter only performs lifecycle
/// work on the coordinator thread.
pub struct CpalMicrophone {
    requested_device_id: Option<String>,
    queue_capacity_frames: usize,
    stream: Option<audio::CaptureStream>,
    consumer: Option<audio::CaptureConsumer>,
}

impl CpalMicrophone {
    pub fn new(requested_device_id: Option<String>, queue_capacity_frames: usize) -> Self {
        Self {
            requested_device_id,
            queue_capacity_frames,
            stream: None,
            consumer: None,
        }
    }

    /// Apply the persisted device choice before the next session opens the
    /// stream. A live stream is never retargeted underneath an active task.
    pub fn set_requested_device_id(&mut self, requested_device_id: Option<String>) {
        if self.stream.is_none() {
            self.requested_device_id = requested_device_id;
        }
    }

    /// Take the non-real-time consumer after `open`. The session task owns the
    /// returned value while this adapter retains the CPAL stream lifetime.
    pub fn take_consumer(&mut self) -> Option<audio::CaptureConsumer> {
        self.consumer.take()
    }

    /// Convert the owned consumer into the non-real-time capture processor
    /// used by the session task. This method is only valid after `open`.
    pub fn take_processor(&mut self) -> Option<audio::CaptureProcessor> {
        let sample_rate = self.stream.as_ref()?.sample_rate_hz();
        self.consumer
            .take()
            .map(|consumer| audio::CaptureProcessor::new(consumer, sample_rate))
    }

    pub fn has_stream_error(&self) -> bool {
        self.stream
            .as_ref()
            .is_some_and(audio::CaptureStream::has_stream_error)
    }
}

impl Microphone for CpalMicrophone {
    fn open(&mut self) -> Result<(), Error> {
        if self.stream.is_some() {
            return Ok(());
        }
        let (stream, consumer) = audio::open_input_stream(
            self.requested_device_id.as_deref(),
            self.queue_capacity_frames,
        )
        .map_err(Error::MicDev)?;
        self.stream = Some(stream);
        self.consumer = Some(consumer);
        Ok(())
    }

    fn close(&mut self) -> Result<(), Error> {
        self.consumer = None;
        self.stream = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_microphone_close_without_open_is_idempotent() {
        let mut microphone = CpalMicrophone::new(None, 16_000);
        microphone.close().expect("close before open is a no-op");
        assert!(microphone.take_consumer().is_none());
    }

    #[test]
    fn fr_1_1_persisted_device_selection_is_applied_before_open() {
        let mut microphone = CpalMicrophone::new(None, 16_000);
        microphone.set_requested_device_id(Some("usb-mic".into()));
        assert_eq!(microphone.requested_device_id.as_deref(), Some("usb-mic"));
    }
}
