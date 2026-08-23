//! Session coordination state and transition invariants (T1.5).
//!
//! The coordinator runs on the session task.  The CPAL callback remains outside
//! this module: it only copies samples to the audio ring buffer (§5.4); callers
//! forward tail samples here from non-real-time work.

#[cfg(target_os = "macos")]
mod macos_runtime {
    use super::{CoordinatorRuntime, DictationRuntime, Microphone, SessionEventSink, SessionState};
    use crate::asr::{
        local_whisper::{EffectiveModelPersistence, LocalWhisperEngine, WhisperRsDecoder},
        model_manager::ModelManager,
        AsrConfig,
    };
    use crate::context::macos::MacFrontmostSnapshot;
    use crate::error::Error;
    use crate::pipeline::{
        microphone::CpalMicrophone,
        session_pump::{local_vad_gate, CapturePoller},
        session_task::{LiveCaptureSession, SessionTask, SessionTaskEvent},
    };
    use crate::store::settings::SettingsStore;
    use std::sync::mpsc::{RecvTimeoutError, Sender};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc as async_mpsc;

    struct SettingsModelPersistence {
        settings: Arc<std::sync::Mutex<SettingsStore>>,
    }

    impl EffectiveModelPersistence for SettingsModelPersistence {
        fn set_effective_local_model(&self, model: &str) -> Result<(), Error> {
            let store = self.settings.lock().map_err(|_| {
                Error::DbIo("settings lock failed while persisting ASR model".into())
            })?;
            store
                .update(serde_json::json!({"asr": {"effectiveLocalModel": model}}))
                .map(|_| ())
        }
    }

    struct NoopMicrophone;
    impl Microphone for NoopMicrophone {
        fn open(&mut self) -> Result<(), Error> {
            Ok(())
        }
        fn close(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    type LocalSession =
        LiveCaptureSession<LocalWhisperEngine<WhisperRsDecoder>, crate::audio::vad::VadSegmenter>;

    struct ActiveSession {
        session: LocalSession,
        events: async_mpsc::Receiver<SessionTaskEvent>,
        session_id: String,
    }

    fn collect_release_tail(session: &mut LocalSession) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_millis(300);
        loop {
            session.poll(Instant::now())?;
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    fn emit_start_failure(
        sink: Option<&Arc<dyn SessionEventSink>>,
        session_id: Option<&str>,
        error: &Error,
    ) {
        let Some(session_id) = session_id else { return };
        if let Some(sink) = sink {
            warn_runtime_error(
                sink.emit_state(session_id, SessionState::Error),
                "error state emission after start failure",
            );
            warn_runtime_error(
                sink.emit_task(session_id, SessionTaskEvent::Failed(error.clone())),
                "start failure event emission failed",
            );
            warn_runtime_error(
                sink.emit_state(session_id, SessionState::Idle),
                "idle state emission after start failure",
            );
        }
    }

    enum Command {
        Start(Sender<Result<(), Error>>),
        Stop(Sender<Result<(), Error>>),
        Cancel(Sender<Result<(), Error>>),
        CancelSilent(Sender<Result<(), Error>>),
        Toggle(Sender<Result<(), Error>>),
        SetEventSink(Arc<dyn SessionEventSink>),
    }

    pub struct MacSessionRuntime {
        commands: Sender<Command>,
    }

    impl MacSessionRuntime {
        pub fn spawn(
            settings: Arc<std::sync::Mutex<SettingsStore>>,
            injector: Arc<dyn crate::inject::Injector>,
        ) -> Self {
            let (commands, receiver) = mpsc::channel();
            thread::Builder::new()
                .name("whisperspree-session".into())
                .spawn(move || {
                    let coordinator = CoordinatorRuntime::new(NoopMicrophone, MacFrontmostSnapshot);
                    let mut microphone = CpalMicrophone::new(None, 16_000);
                    let executor = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("session runtime executor must start");
                    let models = ModelManager::new(crate::store::app_data_dir().join("models"))
                        .expect("embedded model manifest must be valid");
                    let mut active: Option<ActiveSession> = None;
                    let mut event_sink: Option<Arc<dyn SessionEventSink>> = None;
                    let injector: Arc<dyn crate::inject::Injector> = injector;
                    loop {
                        match receiver.recv_timeout(Duration::from_millis(20)) {
                            Ok(command) => match command {
                                Command::Start(reply) => {
                                    let was_active = active.is_some();
                                    let mut start_session_id = Some(coordinator.next_session_id());
                                    let result = (|| {
                                        if active.is_some() {
                                            return Err(Error::IllegalTransition(
                                                "session already active".into(),
                                            ));
                                        }
                                        let session_id = coordinator.start_control_with_id()?;
                                        start_session_id = Some(session_id.clone());
                                        let settings_snapshot = settings
                                            .lock()
                                            .map_err(|_| Error::DbIo("settings lock failed while starting session".into()))?
                                            .get()?;
                                        microphone.set_requested_device_id(settings_snapshot.audio.input_device_id.clone());
                                        microphone.open()?;
                                        let local_model = settings_snapshot
                                            .asr
                                            .effective_local_model
                                            .unwrap_or(settings_snapshot.asr.local_model);
                                        let path =
                                            models.resolve_installed(&local_model).map_err(|error| {
                                                Error::AsrNoModel(error.to_string())
                                            })?;
                                        let mut engine =
                                            LocalWhisperEngine::new(WhisperRsDecoder::load(path)?);
                                        engine.set_effective_model_persistence(Arc::new(
                                            SettingsModelPersistence { settings: Arc::clone(&settings) },
                                        ));
                                        let processor =
                                            microphone.take_processor().ok_or_else(|| {
                                                Error::MicDev(
                                                    "capture processor unavailable".into(),
                                                )
                                            })?;
                                        let (event_tx, event_rx) = async_mpsc::channel(32);
                                        let mut task =
                                            SessionTask::new(engine, local_vad_gate(), event_tx);
                                        if let Some(sink) = event_sink.as_ref().cloned() {
                                            task.set_speech_observer(Arc::new(move || {
                                                sink.speech_observed();
                                            }));
                                        }
                                        let mut session = LiveCaptureSession::new(
                                            CapturePoller::new(processor, 160_000),
                                            task,
                                        );
                                        executor
                                            .block_on(session.start(AsrConfig::local(local_model)))?;
                                        active = Some(ActiveSession {
                                            session,
                                            events: event_rx,
                                            session_id,
                                        });
                                        if let Some(sink) = event_sink.as_ref() {
                                            if let Err(error) = sink.emit_state(
                                                active.as_ref().expect("active session").session_id.as_str(),
                                                SessionState::Listening,
                                            ) {
                                                tracing::warn!(code = %error.code(), %error, "session state event sink failed");
                                            }
                                        }
                                        Ok(())
                                    })();
                                    if result.is_err() && !was_active {
                                        if let Err(error) = &result {
                                            emit_start_failure(event_sink.as_ref(), start_session_id.as_deref(), error);
                                        }
                                        warn_runtime_error(microphone.close(), "microphone close after start failure");
                                        warn_runtime_error(coordinator.reset_control(), "coordinator reset after start failure");
                                    }
                                    let _ = reply.send(result);
                                }
                                Command::Stop(reply) => {
                                    let result = if let Some(mut current) = active.take() {
                                        let session_id = current.session_id.clone();
                                        if let Some(sink) = event_sink.as_ref() {
                                            warn_runtime_error(sink.emit_state(&session_id, SessionState::Finalizing), "finalizing state emission failed");
                                        }
                                        current.session.begin_release();
                                        let tail_result = collect_release_tail(&mut current.session);
                                        let result =
                                            tail_result.and_then(|_| executor.block_on(current.session.stop()).map(|_| ()));
                                        warn_runtime_error(microphone.close(), "microphone close after stop");
                                        let result = if result.is_err() {
                                            warn_runtime_error(coordinator.failed_control(), "coordinator failed transition after stop");
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(&session_id, SessionState::Error), "error state emission failed");
                                            }
                                            let _ = drain_events(&mut current, event_sink.as_ref());
                                            let reset = coordinator.reset_control();
                                            warn_runtime_error(reset.clone(), "coordinator reset after stop failure");
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission failed");
                                            }
                                            result
                                            } else {
                                                let drained =
                                                    drain_events(&mut current, event_sink.as_ref());
                                                if drained.silence_only {
                                                    let reset = coordinator.reset_control();
                                                    warn_runtime_error(reset.clone(), "coordinator reset after silence-only stop");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission after silence-only stop");
                                                    }
                                                    reset
                                                } else {
                                                    warn_runtime_error(coordinator.stop_control(), "coordinator stop transition failed");
                                                    warn_runtime_error(coordinator.finalized_control(), "coordinator finalized transition failed");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::PostProcessing), "post-processing state emission failed");
                                                    }
                                                    let injection = super::run_injection_phase(
                                                        &coordinator,
                                                        injector.as_ref(),
                                                        event_sink.as_ref(),
                                                        &session_id,
                                                        drained.final_text,
                                                    );
                                                    warn_runtime_error(injection.clone(), "injection phase failed");
                                                    injection
                                                }
                                            };
                                        result
                                    } else {
                                        coordinator
                                            .stop_control()
                                            .and_then(|_| coordinator.reset_control())
                                    };
                                    let _ = reply.send(result);
                                }
                                Command::Cancel(reply) => {
                                    let result = if let Some(mut current) = active.take() {
                                        let session_id = current.session_id.clone();
                                        let result = executor.block_on(current.session.cancel());
                                        drain_events(&mut current, event_sink.as_ref());
                                        warn_runtime_error(microphone.close(), "microphone close after cancel");
                                        warn_runtime_error(coordinator.cancel_control(), "coordinator cancel transition failed");
                                        let reset = coordinator.reset_control();
                                        warn_runtime_error(reset.clone(), "coordinator reset after cancel");
                                        let result = result.and(reset);
                                        if result.is_ok() {
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission failed");
                                            }
                                        }
                                        result
                                    } else {
                                        coordinator
                                            .cancel_control()
                                            .and_then(|_| coordinator.reset_control())
                                    };
                                    let _ = reply.send(result);
                                }
                                Command::CancelSilent(reply) => {
                                    let result = if let Some(mut current) = active.take() {
                                        let session_id = current.session_id.clone();
                                        let result = executor.block_on(current.session.cancel());
                                        // Deliberately discard task cancellation events: an
                                        // accidental tap must not create visible cancellation
                                        // HUD/history output.
                                        while current.events.try_recv().is_ok() {}
                                        warn_runtime_error(microphone.close(), "microphone close after silent cancel");
                                        warn_runtime_error(coordinator.cancel_control(), "coordinator silent-cancel transition failed");
                                        let reset = coordinator.reset_control();
                                        warn_runtime_error(reset.clone(), "coordinator reset after silent cancel");
                                        let result = result.and(reset);
                                        if result.is_ok() {
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission after silent cancel failed");
                                            }
                                        }
                                        result
                                    } else {
                                        coordinator.reset_control()
                                    };
                                    let _ = reply.send(result);
                                }
                                Command::Toggle(reply) => {
                                    let result = if active.is_some() {
                                        if let Some(mut current) = active.take() {
                                            let session_id = current.session_id.clone();
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(
                                                    &session_id,
                                                    SessionState::Finalizing,
                                                ), "finalizing state emission failed");
                                            }
                                            current.session.begin_release();
                                            let result = collect_release_tail(&mut current.session)
                                                .and_then(|_| executor.block_on(current.session.stop()).map(|_| ()));
                                            warn_runtime_error(microphone.close(), "microphone close after toggle stop");
                                            let result = if result.is_err() {
                                                warn_runtime_error(coordinator.failed_control(), "coordinator failed transition after toggle stop");
                                                if let Some(sink) = event_sink.as_ref() {
                                                    warn_runtime_error(sink.emit_state(&session_id, SessionState::Error), "error state emission failed");
                                                }
                                                let _ = drain_events(&mut current, event_sink.as_ref());
                                                let reset = coordinator.reset_control();
                                                warn_runtime_error(reset.clone(), "coordinator reset after toggle stop failure");
                                                if let Some(sink) = event_sink.as_ref() {
                                                    warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission failed");
                                                }
                                                result
                                            } else {
                                                let drained =
                                                    drain_events(&mut current, event_sink.as_ref());
                                                if drained.silence_only {
                                                    let reset = coordinator.reset_control();
                                                    warn_runtime_error(reset.clone(), "coordinator reset after silence-only toggle stop");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission after silence-only toggle stop");
                                                    }
                                                    reset
                                                } else {
                                                    warn_runtime_error(coordinator.stop_control(), "coordinator stop transition failed");
                                                    warn_runtime_error(coordinator.finalized_control(), "coordinator finalized transition failed");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::PostProcessing), "post-processing state emission failed");
                                                    }
                                                    let injection = super::run_injection_phase(
                                                        &coordinator,
                                                        injector.as_ref(),
                                                        event_sink.as_ref(),
                                                        &session_id,
                                                        drained.final_text,
                                                    );
                                                    warn_runtime_error(injection.clone(), "injection phase failed after toggle stop");
                                                    injection
                                                }
                                            };
                                            result
                                        } else {
                                            unreachable!("active session checked above")
                                        }
                                    } else {
                                        let mut start_session_id = Some(coordinator.next_session_id());
                                        let result = (|| {
                                            if active.is_some() {
                                                return Err(Error::IllegalTransition(
                                                    "session already active".into(),
                                                ));
                                            }
                                            let session_id = coordinator.start_control_with_id()?;
                                            start_session_id = Some(session_id.clone());
                                            let settings_snapshot = settings
                                                .lock()
                                                .map_err(|_| Error::DbIo("settings lock failed while starting session".into()))?
                                                .get()?;
                                            microphone.set_requested_device_id(settings_snapshot.audio.input_device_id.clone());
                                            microphone.open()?;
                                            let local_model = settings_snapshot
                                                .asr
                                                .effective_local_model
                                                .unwrap_or(settings_snapshot.asr.local_model);
                                            let path = models.resolve_installed(&local_model).map_err(
                                                |error| Error::AsrNoModel(error.to_string()),
                                            )?;
                                            let mut engine = LocalWhisperEngine::new(
                                                WhisperRsDecoder::load(path)?,
                                            );
                                            engine.set_effective_model_persistence(Arc::new(
                                                SettingsModelPersistence { settings: Arc::clone(&settings) },
                                            ));
                                            let processor =
                                                microphone.take_processor().ok_or_else(|| {
                                                    Error::MicDev(
                                                        "capture processor unavailable".into(),
                                                    )
                                                })?;
                                            let (event_tx, event_rx) = async_mpsc::channel(32);
                                            let mut task = SessionTask::new(
                                                engine,
                                                local_vad_gate(),
                                                event_tx,
                                            );
                                            if let Some(sink) = event_sink.as_ref().cloned() {
                                                task.set_speech_observer(Arc::new(move || {
                                                    sink.speech_observed();
                                                }));
                                            }
                                            let mut session = LiveCaptureSession::new(
                                                CapturePoller::new(processor, 160_000),
                                                task,
                                            );
                                            executor.block_on(
                                                session.start(AsrConfig::local(local_model)),
                                            )?;
                                            active = Some(ActiveSession {
                                                session,
                                                events: event_rx,
                                                session_id,
                                            });
                                            if let Some(sink) = event_sink.as_ref() {
                                                if let Err(error) = sink.emit_state(
                                                    active.as_ref().expect("active session").session_id.as_str(),
                                                    SessionState::Listening,
                                                ) {
                                                    tracing::warn!(code = %error.code(), %error, "session state event sink failed");
                                                }
                                            }
                                            Ok(())
                                        })();
                                        if result.is_err() {
                                            if let Err(error) = &result {
                                                emit_start_failure(event_sink.as_ref(), start_session_id.as_deref(), error);
                                            }
                                            warn_runtime_error(
                                                microphone.close(),
                                                "microphone close after toggle start failure",
                                            );
                                            warn_runtime_error(
                                                coordinator.reset_control(),
                                                "coordinator reset after toggle start failure",
                                            );
                                        }
                                        result
                                    };
                                    let _ = reply.send(result);
                                }
                                Command::SetEventSink(sink) => event_sink = Some(sink),
                            },
                            Err(RecvTimeoutError::Timeout) => {
                                if let Some(mut current) = active.take() {
                                    match current.session.poll_with_level(Instant::now()) {
                                        Ok(level) => {
                                            if microphone.has_stream_error() {
                                                let error = Error::MicDev(
                                                    "input stream failed during dictation".into(),
                                                );
                                                let session_id = current.session_id.clone();
                                                tracing::warn!(code = %error.code(), %error, "capture stream failed");
                                                executor.block_on(current.session.fail(error));
                                                warn_runtime_error(microphone.close(), "microphone close after capture stream failure");
                                                warn_runtime_error(coordinator.failed_control(), "coordinator failure transition after capture stream failure");
                                                if let Some(sink) = event_sink.as_ref() {
                                                    warn_runtime_error(sink.emit_state(&session_id, SessionState::Error), "error state emission after capture stream failure");
                                                }
                                                drain_events(&mut current, event_sink.as_ref());
                                                warn_runtime_error(coordinator.reset_control(), "coordinator reset after capture stream failure");
                                                if let Some(sink) = event_sink.as_ref() {
                                                    warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission after capture stream failure");
                                                }
                                                continue;
                                            }
                                            if let Some(level) = level {
                                                if let Some(sink) = event_sink.as_ref() {
                                                    warn_runtime_error(
                                                        sink.emit_audio_level(&current.session_id, level.rms, level.peak),
                                                        "audio level event emission failed",
                                                    );
                                                }
                                            }
                                            match executor.block_on(current.session.service()) {
                                                Ok(()) => {
                                                    drain_events(&mut current, event_sink.as_ref());
                                                    active = Some(current);
                                                }
                                                Err(error) => {
                                                    let session_id = current.session_id.clone();
                                                    tracing::warn!(code = %error.code(), %error, "live session servicing failed");
                                                    executor.block_on(current.session.fail(error));
                                                    warn_runtime_error(microphone.close(), "microphone close after service failure");
                                                    warn_runtime_error(coordinator.failed_control(), "coordinator failure transition after service failure");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::Error), "error state emission after service failure");
                                                    }
                                                    drain_events(&mut current, event_sink.as_ref());
                                                    warn_runtime_error(coordinator.reset_control(), "coordinator reset after service failure");
                                                    if let Some(sink) = event_sink.as_ref() {
                                                        warn_runtime_error(sink.emit_state(&session_id, SessionState::Idle), "idle state emission after service failure");
                                                    }
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            let session_id = current.session_id.clone();
                                            tracing::warn!(
                                                code = %error.code(),
                                                %error,
                                                "live session polling failed"
                                            );
                                            executor.block_on(current.session.fail(error));
                                            warn_runtime_error(microphone.close(), "microphone close after poll failure");
                                            warn_runtime_error(coordinator.failed_control(), "coordinator failure transition failed");
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(
                                                    &session_id,
                                                    SessionState::Error,
                                                ), "error state emission failed");
                                            }
                                            drain_events(&mut current, event_sink.as_ref());
                                            warn_runtime_error(coordinator.reset_control(), "coordinator reset after poll failure");
                                            if let Some(sink) = event_sink.as_ref() {
                                                warn_runtime_error(sink.emit_state(
                                                    &session_id,
                                                    SessionState::Idle,
                                                ), "idle state emission failed");
                                            }
                                        }
                                    }
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                })
                .expect("session runtime thread must start");
            Self { commands }
        }

        fn call(&self, command: fn(Sender<Result<(), Error>>) -> Command) -> Result<(), Error> {
            let (reply, result) = mpsc::channel();
            self.commands
                .send(command(reply))
                .map_err(|_| Error::DbIo("session runtime thread stopped".into()))?;
            result
                .recv()
                .map_err(|_| Error::DbIo("session runtime reply dropped".into()))?
        }

        fn set_event_sink(&self, sink: Arc<dyn SessionEventSink>) -> Result<(), Error> {
            self.commands
                .send(Command::SetEventSink(sink))
                .map_err(|_| Error::DbIo("session runtime thread stopped".into()))
        }
    }

    fn drain_events(
        active: &mut ActiveSession,
        sink: Option<&Arc<dyn SessionEventSink>>,
    ) -> DrainOutcome {
        let mut outcome = DrainOutcome::default();
        while let Ok(event) = active.events.try_recv() {
            match &event {
                SessionTaskEvent::SilenceOnly => outcome.silence_only = true,
                SessionTaskEvent::Final(final_transcript) => {
                    outcome.final_text = Some(final_transcript.text.clone());
                }
                _ => {}
            }
            if let Some(sink) = sink {
                if let Err(error) = sink.emit_task(&active.session_id, event) {
                    tracing::warn!(code = %error.code(), %error, "session task event sink failed");
                }
            }
        }
        outcome
    }

    fn warn_runtime_error(result: Result<(), Error>, context: &'static str) {
        if let Err(error) = result {
            tracing::warn!(code = %error.code(), %error, context);
        }
    }

    /// What a drained session produced: the silence verdict and, when present,
    /// the final transcript text the injection phase will place for the user.
    #[derive(Default)]
    struct DrainOutcome {
        silence_only: bool,
        final_text: Option<String>,
    }

    impl DictationRuntime for MacSessionRuntime {
        fn start_dictation(&self) -> Result<(), Error> {
            self.call(Command::Start)
        }
        fn stop_dictation(&self) -> Result<(), Error> {
            self.call(Command::Stop)
        }
        fn cancel_dictation(&self) -> Result<(), Error> {
            self.call(Command::Cancel)
        }

        fn cancel_silent_dictation(&self) -> Result<(), Error> {
            self.call(Command::CancelSilent)
        }

        fn toggle_dictation(&self) -> Result<(), Error> {
            self.call(Command::Toggle)
        }

        fn set_event_sink(&self, sink: Arc<dyn SessionEventSink>) {
            if let Err(error) = self.set_event_sink(sink) {
                tracing::warn!(code = %error.code(), %error, "unable to install session event sink");
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_runtime::MacSessionRuntime;

use crate::pipeline::session_task::SessionTaskEvent;

/// Non-fatal emission logging for the phase runner (module-local variant of
/// the runtime loop's `warn_runtime_error`).
fn warn_emit(result: Result<(), Error>, context: &'static str) {
    if let Err(error) = result {
        tracing::warn!(code = %error.code(), %error, context);
    }
}

/// Runtime-facing bridge for typed §9.2 events. Implementations are installed
/// by the application shell after Tauri has created its AppHandle; the session
/// thread only invokes this non-real-time boundary while draining its queue.
pub trait SessionEventSink: Send + Sync {
    fn emit_task(&self, session_id: &str, event: SessionTaskEvent) -> Result<(), Error>;
    fn emit_state(&self, session_id: &str, state: SessionState) -> Result<(), Error>;
    fn emit_audio_level(&self, _session_id: &str, _rms: f32, _peak: f32) -> Result<(), Error> {
        Ok(())
    }
    /// Notify the hotkey policy only after the coordinator has published Idle.
    /// The default keeps deterministic test sinks source-compatible.
    fn session_became_idle(&self, _at: std::time::Instant) {}
    /// §9.2 `inject:done {sessionId, method}` plus the P-5 history-suppression
    /// flag for secure-field outcomes. `persist_history=false` is consumed by
    /// the T6.1 history writer; absent writers treat it as informational.
    fn emit_injection_outcome(
        &self,
        _session_id: &str,
        _method: &str,
        _persist_history: bool,
    ) -> Result<(), Error> {
        Ok(())
    }
    /// Forward evidence that speech was observed so accidental-tap policy does
    /// not classify a real utterance as silent.
    fn speech_observed(&self) {}
}

use crate::error::Error;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

/// Maximum audio retained after hotkey release (SM-4).
pub const TAIL_MS: u32 = 300;
const PCM_SAMPLE_RATE: usize = 16_000;
const TAIL_SAMPLES: usize = PCM_SAMPLE_RATE * TAIL_MS as usize / 1_000;

/// The frontmost application snapshot captured at dictation start and injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppContext {
    pub bundle_id: String,
    pub title: String,
    pub secure_input: bool,
}

/// OS boundary implemented by T2.3.
pub trait ContextDetector {
    fn snapshot(&mut self) -> Result<AppContext, Error>;
}

/// OS microphone boundary.  The capture implementation must keep its real-time
/// callback allocation- and lock-free (§5.4); these lifecycle calls occur on the
/// coordinator task, never in that callback.
pub trait Microphone {
    fn open(&mut self) -> Result<(), Error>;
    fn close(&mut self) -> Result<(), Error>;
}

/// Synchronous control boundary shared by hotkeys, tray actions, and §9.1 IPC.
/// Implementations serialize all calls onto the one session owner; they never
/// run inside CPAL's callback.
pub trait DictationRuntime: Send + Sync {
    fn start_dictation(&self) -> Result<(), Error>;
    fn stop_dictation(&self) -> Result<(), Error>;
    fn cancel_dictation(&self) -> Result<(), Error>;
    /// Silently discard an accidental sub-200 ms tap. Idle runtimes treat this
    /// as a no-op; active runtimes close the session without publishing a user
    /// visible cancellation or history row.
    fn cancel_silent_dictation(&self) -> Result<(), Error> {
        self.cancel_dictation()
    }
    fn toggle_dictation(&self) -> Result<(), Error> {
        self.start_dictation()
    }
    fn set_event_sink(&self, _sink: std::sync::Arc<dyn SessionEventSink>) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionMethod {
    Normal,
    ClipboardOnly,
}

/// Internal, ordered output for the eventual §9.2 Tauri event adapter and T2.2
/// injector. State always precedes a destination-state-specific event (SM-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorEvent {
    State(SessionState),
    TailAudioAccepted {
        samples: usize,
    },
    InjectionReady {
        target: AppContext,
        style: AppContext,
        method: InjectionMethod,
        /// False for secure-field outcomes (P-5: raw + processed text are
        /// discarded after the clipboard set) and for unknown targets where a
        /// secure field cannot be ruled out.
        persist_history: bool,
    },
}

/// Data owned for the lifetime of one dictation session. The start snapshot is
/// used for style while `injection_context` is the actual current target (SM-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub session_id: String,
    pub start_context: AppContext,
    pub injection_context: Option<AppContext>,
    pub tail_samples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Listening,
    Finalizing,
    PostProcessing,
    Injecting,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    HotkeyDown,
    Release,
    Finalized,
    Processed,
    InjectDone,
    Escape,
    Reset,
    Failed,
}

/// Explicit, side-effect-free state transition table (§5.2). It is separately
/// useful to hotkey code and pins that illegal moves never mutate the state.
pub struct SessionTransition {
    state: SessionState,
    events: Vec<SessionState>,
}

impl SessionTransition {
    pub fn new() -> Self {
        Self {
            state: SessionState::Idle,
            events: Vec::new(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn take_events(&mut self) -> Vec<SessionState> {
        std::mem::take(&mut self.events)
    }

    pub fn apply(&mut self, event: SessionEvent) -> Result<SessionState, Error> {
        let next = match (self.state, event) {
            (SessionState::Idle, SessionEvent::HotkeyDown) => SessionState::Listening,
            // SM-1: a second press never creates a second session.
            (SessionState::Listening, SessionEvent::HotkeyDown) => SessionState::Listening,
            (SessionState::Listening, SessionEvent::Escape) => SessionState::Cancelled,
            (SessionState::Listening, SessionEvent::Release) => SessionState::Finalizing,
            (SessionState::Finalizing, SessionEvent::Finalized) => SessionState::PostProcessing,
            (SessionState::PostProcessing, SessionEvent::Processed) => SessionState::Injecting,
            (SessionState::Injecting, SessionEvent::InjectDone) => SessionState::Idle,
            (
                SessionState::Listening
                | SessionState::Finalizing
                | SessionState::PostProcessing
                | SessionState::Injecting
                | SessionState::Cancelled
                | SessionState::Error,
                SessionEvent::Reset,
            ) => SessionState::Idle,
            (
                SessionState::Listening
                | SessionState::Finalizing
                | SessionState::PostProcessing
                | SessionState::Injecting,
                SessionEvent::Failed,
            ) => SessionState::Error,
            _ => {
                let error = Error::IllegalTransition(format!("{event:?} from {:?}", self.state));
                tracing::warn!(code = %error.code(), state = ?self.state, event = ?event, "illegal session transition");
                return Err(error);
            }
        };
        if next != self.state {
            self.state = next;
            self.events.push(next);
        }
        Ok(next)
    }
}

impl Default for SessionTransition {
    fn default() -> Self {
        Self::new()
    }
}

/// Single-session owner. All methods are intended for the one coordinator task;
/// it owns the microphone lifetime, snapshots both contexts, and produces ordered
/// domain events without calling Tauri or an OS injector directly.
pub struct SessionCoordinator<M, C> {
    transition: SessionTransition,
    microphone: M,
    context_detector: C,
    microphone_open: bool,
    session: Option<SessionContext>,
    events: Vec<CoordinatorEvent>,
}

impl<M: Microphone, C: ContextDetector> SessionCoordinator<M, C> {
    pub fn new(microphone: M, context_detector: C) -> Self {
        Self {
            transition: SessionTransition::new(),
            microphone,
            context_detector,
            microphone_open: false,
            session: None,
            events: Vec::new(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.transition.state()
    }
    pub fn microphone(&self) -> &M {
        &self.microphone
    }
    pub fn microphone_is_open(&self) -> bool {
        self.microphone_open
    }
    pub fn tail_samples(&self) -> usize {
        self.session
            .as_ref()
            .map_or(0, |session| session.tail_samples)
    }
    pub fn session_context(&self) -> Option<&SessionContext> {
        self.session.as_ref()
    }
    pub fn take_events(&mut self) -> Vec<CoordinatorEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn hotkey_down(&mut self, session_id: impl Into<String>) -> Result<SessionState, Error> {
        if self.state() != SessionState::Idle {
            return self.transition.apply(SessionEvent::HotkeyDown);
        }
        let context = match self.context_detector.snapshot() {
            Ok(context) => context,
            Err(error) => {
                // Accessibility/frontmost metadata is advisory for dictation;
                // preserve the utterance and let injection fall back to its
                // conservative clipboard-only path when the target is not
                // available.
                tracing::warn!(code = %error.code(), %error, "frontmost context unavailable; using unknown context");
                AppContext {
                    bundle_id: "unknown".into(),
                    title: String::new(),
                    secure_input: false,
                }
            }
        };
        self.microphone.open()?;
        self.microphone_open = true;
        self.session = Some(SessionContext {
            session_id: session_id.into(),
            start_context: context,
            injection_context: None,
            tail_samples: 0,
        });
        self.transition_and_emit(SessionEvent::HotkeyDown)
    }

    pub fn release(&mut self) -> Result<SessionState, Error> {
        self.transition_and_emit(SessionEvent::Release)
    }

    /// Stores only the first 300 ms of samples captured after release. The caller
    /// can forward the accepted prefix to its recognizer without reallocating on
    /// the real-time audio thread.
    pub fn feed_tail(&mut self, samples: &[f32]) -> Result<usize, Error> {
        if self.state() != SessionState::Finalizing {
            return Err(Error::DbIo("tail audio received outside finalizing".into()));
        }
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| Error::DbIo("missing session context".into()))?;
        let accepted = (TAIL_SAMPLES - session.tail_samples).min(samples.len());
        session.tail_samples += accepted;
        if accepted != 0 {
            self.events
                .push(CoordinatorEvent::TailAudioAccepted { samples: accepted });
        }
        Ok(accepted)
    }

    pub fn finalized(&mut self) -> Result<SessionState, Error> {
        if self.state() != SessionState::Finalizing {
            return Err(Error::DbIo("illegal session transition".into()));
        }
        self.close_microphone()?;
        self.transition_and_emit(SessionEvent::Finalized)
    }

    pub fn processed(&mut self) -> Result<SessionState, Error> {
        let (target, method, persist_history) = match self.context_detector.snapshot() {
            Ok(target) => {
                // P-5: a resolved secure field suppresses injection strategy
                // and the history row together.
                let method = if target.secure_input {
                    InjectionMethod::ClipboardOnly
                } else {
                    InjectionMethod::Normal
                };
                let persist_history = !target.secure_input;
                (target, method, persist_history)
            }
            Err(error) => {
                // The target may disappear while the utterance is being
                // post-processed. Injection must still complete safely: use
                // an unknown target and force the conservative clipboard-only
                // strategy instead of leaving the coordinator stuck in
                // PostProcessing.
                tracing::warn!(
                    code = %error.code(),
                    %error,
                    "frontmost context unavailable at injection; using clipboard-only fallback"
                );
                (
                    AppContext {
                        bundle_id: "unknown".into(),
                        title: String::new(),
                        secure_input: false,
                    },
                    InjectionMethod::ClipboardOnly,
                    // The destination could not be probed; a secure field
                    // cannot be ruled out, so the utterance is never written
                    // to history (P-5, conservative).
                    false,
                )
            }
        };
        let style = self
            .session
            .clone()
            .ok_or_else(|| Error::DbIo("missing session context".into()))?;
        let style = style.start_context;
        let state = self.transition_and_emit(SessionEvent::Processed)?;
        self.session
            .as_mut()
            .expect("active session required for processing")
            .injection_context = Some(target.clone());
        self.events.push(CoordinatorEvent::InjectionReady {
            method,
            target,
            style,
            persist_history,
        });
        Ok(state)
    }

    pub fn inject_done(&mut self) -> Result<SessionState, Error> {
        let state = self.transition_and_emit(SessionEvent::InjectDone)?;
        self.clear_completed_session();
        Ok(state)
    }

    pub fn escape(&mut self) -> Result<SessionState, Error> {
        if self.state() != SessionState::Listening {
            return Err(Error::DbIo("illegal session transition".into()));
        }
        let close_result = self.close_microphone();
        let state = self.transition_and_emit(SessionEvent::Escape)?;
        close_result.map(|_| state)
    }

    pub fn reset(&mut self) -> Result<SessionState, Error> {
        let close_result = self.close_microphone();
        let state = self.transition_and_emit(SessionEvent::Reset)?;
        self.session = None;
        close_result.map(|_| state)
    }

    pub fn failed(&mut self) -> Result<SessionState, Error> {
        let close_result = self.close_microphone();
        let state = self.transition_and_emit(SessionEvent::Failed)?;
        close_result.map(|_| state)
    }

    fn close_microphone(&mut self) -> Result<(), Error> {
        if self.microphone_open {
            self.microphone.close()?;
            self.microphone_open = false;
        }
        Ok(())
    }

    fn transition_and_emit(&mut self, event: SessionEvent) -> Result<SessionState, Error> {
        let state = self.transition.apply(event)?;
        if self.transition.take_events().last().copied() == Some(state) {
            self.events.push(CoordinatorEvent::State(state));
        }
        Ok(state)
    }

    fn clear_completed_session(&mut self) {
        self.session = None;
    }
}

/// Minimal production coordinator adapter. This gives every external control
/// surface one serialized owner for the T1.5 state machine. Audio polling,
/// VAD feeding, and recognizer finalization remain session-task work and are
/// deliberately not performed by IPC threads.
pub struct CoordinatorRuntime<M, C> {
    coordinator: Mutex<SessionCoordinator<M, C>>,
    next_session: AtomicU64,
}

impl<M: Microphone, C: ContextDetector> CoordinatorRuntime<M, C> {
    pub fn new(microphone: M, context_detector: C) -> Self {
        Self {
            coordinator: Mutex::new(SessionCoordinator::new(microphone, context_detector)),
            next_session: AtomicU64::new(1),
        }
    }

    pub fn take_events(&self) -> Result<Vec<CoordinatorEvent>, Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        Ok(coordinator.take_events())
    }

    pub fn state(&self) -> Result<SessionState, Error> {
        let coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        Ok(coordinator.state())
    }

    pub fn start_control(&self) -> Result<(), Error> {
        self.start_control_with_id().map(|_| ())
    }

    /// Correlation id reserved for the next start attempt. This lets a
    /// platform owner publish a coded error even when context probing fails
    /// before `hotkey_down` can return its id.
    pub fn next_session_id(&self) -> String {
        format!("session-{}", self.next_session.load(Ordering::Relaxed))
    }

    /// Start a session and return the coordinator-authoritative identifier so
    /// runtime adapters can use the same correlation key for every IPC event.
    pub fn start_control_with_id(&self) -> Result<String, Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        let session = self.next_session.fetch_add(1, Ordering::Relaxed);
        let session_id = format!("session-{session}");
        coordinator.hotkey_down(session_id.clone())?;
        Ok(session_id)
    }

    pub fn stop_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.release()?;
        Ok(())
    }

    pub fn finalized_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.finalized()?;
        Ok(())
    }

    /// PostProcessing → Injecting. Returns the ordered coordinator output so
    /// the caller observes the `InjectionReady` payload atomically.
    pub fn processed_control(&self) -> Result<Vec<CoordinatorEvent>, Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.processed()?;
        Ok(coordinator.take_events())
    }

    pub fn inject_done_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.inject_done()?;
        Ok(())
    }

    pub fn cancel_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.escape()?;
        Ok(())
    }

    /// Transition an active session to the terminal error state while closing
    /// the microphone. Runtime adapters use this when capture/VAD/ASR polling
    /// fails outside the explicit stop/cancel commands.
    pub fn failed_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.failed()?;
        Ok(())
    }

    pub fn reset_control(&self) -> Result<(), Error> {
        let mut coordinator = self
            .coordinator
            .lock()
            .map_err(|_| Error::DbIo("dictation runtime lock failed".into()))?;
        coordinator.reset()?;
        Ok(())
    }
}

/// §9.2 wire spelling for the coordinator-level strategy hint.
pub fn injection_method_wire(method: InjectionMethod) -> &'static str {
    match method {
        InjectionMethod::Normal => "paste",
        InjectionMethod::ClipboardOnly => "clipboard_only",
    }
}

#[cfg(target_os = "macos")]
fn accessibility_trusted() -> bool {
    crate::context::macos::accessibility_trusted()
}

#[cfg(not(target_os = "macos"))]
fn accessibility_trusted() -> bool {
    false
}

/// Executes the SM-3/SM-5 injection phase for one finished utterance:
/// PostProcessing → Injecting → (policy inject) → `inject:done` → Idle.
///
/// Public for deterministic tests; the production caller passes the managed
/// injector from `IpcState`. An injection failure never wedges the machine:
/// the coded error is logged and the coordinator still returns to Idle
/// without publishing an outcome event.
pub fn run_injection_phase<
    M: Microphone,
    C: ContextDetector,
    I: crate::inject::Injector + ?Sized,
>(
    runtime: &CoordinatorRuntime<M, C>,
    injector: &I,
    sink: Option<&Arc<dyn SessionEventSink>>,
    session_id: &str,
    final_text: Option<String>,
) -> Result<(), Error> {
    let events = runtime.processed_control()?;
    if let Some(sink) = sink {
        warn_emit(
            sink.emit_state(session_id, SessionState::Injecting),
            "injecting state emission failed",
        );
    }
    let ready = events.into_iter().find_map(|event| match event {
        CoordinatorEvent::InjectionReady {
            target,
            persist_history,
            ..
        } => Some((target, persist_history)),
        _ => None,
    });
    let Some((target, persist_history)) = ready else {
        // No ready payload means the coordinator already degraded; finish the
        // machine so a missing transcript can never strand the state graph.
        return runtime.inject_done_control();
    };
    let context = crate::context::inject_context(Some(&target), accessibility_trusted());
    if let Some(text) = final_text {
        match injector.inject(&text, &context) {
            Ok(method) => {
                if let Some(sink) = sink {
                    warn_emit(
                        sink.emit_injection_outcome(
                            session_id,
                            crate::inject::wire_method(method),
                            persist_history,
                        ),
                        "inject done emission failed",
                    );
                }
            }
            Err(error) => {
                tracing::warn!(code = %error.code(), %error, "injection failed; returning to idle");
            }
        }
    } else {
        tracing::debug!("no final transcript; skipping injection");
    }
    runtime.inject_done_control()?;
    if let Some(sink) = sink {
        warn_emit(
            sink.emit_state(session_id, SessionState::Idle),
            "idle state emission after injection failed",
        );
    }
    Ok(())
}

impl<M: Microphone + Send, C: ContextDetector + Send> DictationRuntime
    for CoordinatorRuntime<M, C>
{
    fn start_dictation(&self) -> Result<(), Error> {
        self.start_control()
    }

    fn stop_dictation(&self) -> Result<(), Error> {
        self.stop_control()
    }

    fn cancel_dictation(&self) -> Result<(), Error> {
        self.cancel_control()
    }

    fn cancel_silent_dictation(&self) -> Result<(), Error> {
        if self.state()? == SessionState::Idle {
            Ok(())
        } else {
            self.cancel_control().map(|_| ())
        }
    }

    fn toggle_dictation(&self) -> Result<(), Error> {
        match self.state()? {
            SessionState::Idle => self.start_control(),
            SessionState::Listening => self.stop_control(),
            _ => Err(Error::IllegalTransition(
                "toggle requested while session is finalizing".into(),
            )),
        }
    }
}
