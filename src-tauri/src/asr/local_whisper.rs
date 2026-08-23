//! Local Whisper tumbling-window recognizer (FR-1.1.b).
//!
//! `LocalDecoder` is deliberately narrow and injected.  The production
//! whisper-rs adapter belongs behind this boundary; unit tests use an in-memory
//! decoder and therefore never load or download a model.

use super::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer};
use crate::{error::Error, ipc::events::WordTiming};
use async_trait::async_trait;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Select the whisper worker count from physical cores, capped by the PRD's
/// eight-thread ceiling. The helper is pure so the platform probe can be
/// tested independently from model loading.
pub fn select_thread_count(physical_cores: Option<usize>, logical_cores: usize) -> i32 {
    physical_cores
        .filter(|cores| *cores > 0)
        .unwrap_or(logical_cores.max(1))
        .clamp(1, 8) as i32
}

#[cfg(target_os = "macos")]
fn physical_core_count() -> Option<usize> {
    crate::asr::macos::physical_core_count()
}

#[cfg(not(target_os = "macos"))]
fn physical_core_count() -> Option<usize> {
    None
}

pub const SAMPLE_RATE_HZ: usize = 16_000;
pub const WINDOW_MS: u32 = 4_000;
pub const ENDPOINT_SILENCE_MS: u32 = 700;
pub const MIN_PENDING_SPEECH_MS: u32 = 600;
const WINDOW_SAMPLES: usize = SAMPLE_RATE_HZ * WINDOW_MS as usize / 1_000;
const MIN_PENDING_SPEECH_SAMPLES: usize = SAMPLE_RATE_HZ * MIN_PENDING_SPEECH_MS as usize / 1_000;
const MAX_QUEUED_WINDOWS: usize = 2;
const INITIAL_PROMPT_CARRY_CHARS: usize = 200;

/// A timestamped Whisper token. Token text is merged to whitespace-word
/// granularity before it crosses the ASR boundary (§8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenTiming {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedWindow {
    pub text: String,
    pub tokens: Vec<TokenTiming>,
    pub language: Option<String>,
    pub language_confidence: Option<f32>,
}

/// The model-owning decoder boundary. No implementation may initiate a network
/// request: models are installed separately by `ModelManager`.
pub trait LocalDecoder: Send + 'static {
    /// Applies per-session Whisper options before the first window. Fakes need
    /// no configuration; the production adapter records the explicit FR-1.1.b
    /// language/translation choices here.
    fn configure(&mut self, _cfg: &AsrConfig) -> Result<(), Error> {
        Ok(())
    }

    fn decode(&mut self, samples: &[f32], initial_prompt: &str) -> Result<DecodedWindow, Error>;
}

/// Persistence seam for `asr.effectiveLocalModel`. The session/settings owner
/// supplies the real implementation; tests use a recording closure/object.
pub trait EffectiveModelPersistence: Send + Sync {
    fn set_effective_local_model(&self, model: &str) -> Result<(), Error>;
}

/// Production whisper.cpp adapter. Its constructor opens only an already
/// installed local model path; it has no downloader or network dependency.
pub struct WhisperRsDecoder {
    // The state borrows native resources from this owning context; retain it
    // for the decoder's full lifetime even though calls use `state` directly.
    _context: whisper_rs::WhisperContext,
    state: whisper_rs::WhisperState,
    language: Option<String>,
    translate_to_english: bool,
    n_threads: i32,
}

impl WhisperRsDecoder {
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, Error> {
        let model_path = model_path.as_ref();
        if !model_path.is_file() {
            return Err(Error::AsrNoModel("no installed local Whisper model".into()));
        }
        let context = whisper_rs::WhisperContext::new_with_params(
            model_path,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|error| Error::AsrLoad(format!("loading local Whisper model: {error}")))?;
        let state = context
            .create_state()
            .map_err(|error| Error::AsrLoad(format!("creating local Whisper state: {error}")))?;
        Ok(Self {
            _context: context,
            state,
            language: None,
            translate_to_english: false,
            n_threads: select_thread_count(
                physical_core_count(),
                std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
            ),
        })
    }
}

impl LocalDecoder for WhisperRsDecoder {
    fn configure(&mut self, cfg: &AsrConfig) -> Result<(), Error> {
        self.language = cfg.language.clone();
        self.translate_to_english = cfg.translate_to_english;
        Ok(())
    }

    fn decode(&mut self, samples: &[f32], initial_prompt: &str) -> Result<DecodedWindow, Error> {
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_initial_prompt(initial_prompt);
        params.set_language(self.language.as_deref());
        params.set_translate(self.translate_to_english);
        params.set_n_threads(self.n_threads);
        params.set_token_timestamps(true);
        params.set_single_segment(false);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        self.state
            .full(params, samples)
            .map_err(|error| Error::AsrLoad(format!("decoding local Whisper audio: {error}")))?;

        let mut text = String::new();
        let mut tokens = Vec::new();
        for segment in self.state.as_iter() {
            text.push_str(&segment.to_string());
            for index in 0..segment.n_tokens() {
                if let Some(token) = segment.get_token(index) {
                    let data = token.token_data();
                    tokens.push(TokenTiming {
                        text: token.to_string(),
                        start_ms: (data.t0.max(0) as u32).saturating_mul(10),
                        end_ms: (data.t1.max(0) as u32).saturating_mul(10),
                    });
                }
            }
        }
        Ok(DecodedWindow {
            text,
            tokens,
            language: self.language.clone(),
            language_confidence: None,
        })
    }
}

#[derive(Debug)]
struct Window {
    samples: Vec<f32>,
    start_ms: u32,
}

struct CompletedWindow {
    window: Window,
    decoded: DecodedWindow,
    elapsed_ms: u64,
}

/// FR-1.1.b local engine. `feed` only copies audio and closes windows; callers
/// drive `drain` from the session task, where each decode runs on Tokio's
/// blocking pool and is strictly serialized.
pub struct LocalWhisperEngine<D: LocalDecoder> {
    decoder: Arc<Mutex<D>>,
    pending: Vec<f32>,
    queued: VecDeque<Window>,
    dictionary_bias: String,
    prior_text: String,
    transcript: FinalTranscript,
    sender: Option<mpsc::Sender<AsrEvent>>,
    config: Option<AsrConfig>,
    next_start_ms: u32,
    consecutive_slow_decodes: u8,
    effective_model_next_session: Option<String>,
    effective_model_persistence: Option<Arc<dyn EffectiveModelPersistence>>,
    live_tasks: VecDeque<JoinHandle<Result<CompletedWindow, Error>>>,
    serial_decode: Arc<tokio::sync::Mutex<()>>,
    live_prompt: Arc<Mutex<String>>,
}

impl<D: LocalDecoder> LocalWhisperEngine<D> {
    pub fn new(decoder: D) -> Self {
        Self {
            decoder: Arc::new(Mutex::new(decoder)),
            pending: Vec::new(),
            queued: VecDeque::new(),
            dictionary_bias: String::new(),
            prior_text: String::new(),
            transcript: FinalTranscript {
                text: String::new(),
                words: Vec::new(),
                language: None,
            },
            sender: None,
            config: None,
            next_start_ms: 0,
            consecutive_slow_decodes: 0,
            effective_model_next_session: None,
            effective_model_persistence: None,
            live_tasks: VecDeque::new(),
            serial_decode: Arc::new(tokio::sync::Mutex::new(())),
            live_prompt: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Dictionary phrases are supplied by T5.1. The carry-over is appended
    /// after this prefix exactly as FR-1.1.b(4) requires.
    pub fn set_dictionary_bias(&mut self, bias: impl Into<String>) {
        self.dictionary_bias = bias.into();
    }

    pub fn set_effective_model_persistence(
        &mut self,
        persistence: Arc<dyn EffectiveModelPersistence>,
    ) {
        self.effective_model_persistence = Some(persistence);
    }

    /// VAD owns silence counting. It calls this only after an endpoint was
    /// reported, keeping silence samples out of the pending ASR buffer.
    pub fn endpoint_silence(&mut self, silence_ms: u32) -> Result<(), Error> {
        if silence_ms >= ENDPOINT_SILENCE_MS && self.pending.len() >= MIN_PENDING_SPEECH_SAMPLES {
            self.close_pending_window();
            self.schedule_live_decode();
        }
        Ok(())
    }

    /// Runs every queued window in order. The public helper lets the future
    /// session task service finished windows before `finalize`; no two decoder
    /// calls can overlap because this loop awaits each blocking job.
    pub async fn drain(&mut self) -> Result<(), Error> {
        while let Some(task) = self.live_tasks.pop_front() {
            let completed = task.await.map_err(|error| {
                Error::AsrLoad(format!("local decoder worker failed: {error}"))
            })??;
            self.apply_completed_window(completed).await?;
        }
        while let Some(window) = self.queued.pop_front() {
            self.decode_window(window).await?;
        }
        Ok(())
    }

    /// Reap completed live work and immediately start the next queued window.
    /// This is bounded to work that is already finished: capture never waits
    /// for a decoder during the live session.
    pub async fn service_live(&mut self) -> Result<(), Error> {
        if self.live_tasks.front().is_some_and(JoinHandle::is_finished) {
            let task = self.live_tasks.pop_front().expect("checked front task");
            let completed = task.await.map_err(|error| {
                Error::AsrLoad(format!("local decoder worker failed: {error}"))
            })??;
            self.apply_completed_window(completed).await?;
        }
        self.schedule_live_decode();
        Ok(())
    }

    /// Selected after two slow decodes and consumed by the next session's
    /// settings adapter (T1.5). It is intentionally not applied mid-session.
    pub fn effective_model_next_session(&self) -> Option<&str> {
        self.effective_model_next_session.as_deref()
    }

    pub fn queued_timeline_for_test(&self) -> Vec<(u32, u32)> {
        self.queued
            .iter()
            .map(|w| (w.start_ms, w.start_ms + samples_to_ms(w.samples.len())))
            .collect()
    }

    fn close_pending_window(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let window = Window {
            samples: std::mem::take(&mut self.pending),
            start_ms: self.next_start_ms,
        };
        self.next_start_ms += samples_to_ms(window.samples.len());
        if self.queued.len() + self.live_tasks.len() < MAX_QUEUED_WINDOWS {
            self.queued.push_back(window);
        } else if let Some(last) = self.queued.back_mut() {
            // Queue overflow deliberately joins with the newest queued window:
            // no audio is dropped and the two-window bound remains true.
            last.samples.extend(window.samples);
        }
    }

    fn schedule_live_decode(&mut self) {
        if !self.live_tasks.is_empty() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let Some(window) = self.queued.pop_front() else {
            return;
        };
        let decoder = Arc::clone(&self.decoder);
        let serial_decode = Arc::clone(&self.serial_decode);
        let prompt_state = Arc::clone(&self.live_prompt);
        let dictionary_bias = self.dictionary_bias.clone();
        let tx = self.sender.clone();
        self.live_tasks.push_back(tokio::spawn(async move {
            let _serial = serial_decode.lock().await;
            let carry = prompt_state
                .lock()
                .map_err(|_| Error::AsrLoad("local prompt lock poisoned".into()))?
                .clone();
            let carry = suffix_chars(&carry, INITIAL_PROMPT_CARRY_CHARS);
            let prompt = match (dictionary_bias.trim(), carry.is_empty()) {
                ("", _) => carry,
                (bias, true) => bias.to_owned(),
                (bias, false) => format!("{bias}\n{carry}"),
            };
            let samples = window.samples.clone();
            let started = Instant::now();
            let decoded = tokio::task::spawn_blocking(move || {
                decoder
                    .lock()
                    .map_err(|_| Error::AsrLoad("local decoder lock poisoned".into()))?
                    .decode(&samples, &prompt)
            })
            .await
            .map_err(|error| Error::AsrLoad(format!("local decoder worker failed: {error}")))??;
            *prompt_state
                .lock()
                .map_err(|_| Error::AsrLoad("local prompt lock poisoned".into()))? =
                decoded.text.trim().to_owned();
            if let Some(tx) = tx {
                let words = words_from_tokens(&decoded.text, &decoded.tokens, window.start_ms, &[]);
                let _ = tx
                    .send(AsrEvent::Segment {
                        text: decoded.text.clone(),
                        words,
                    })
                    .await;
            }
            Ok(CompletedWindow {
                window,
                decoded,
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }));
    }

    async fn apply_completed_window(&mut self, completed: CompletedWindow) -> Result<(), Error> {
        self.record_decode_speed(
            completed.elapsed_ms,
            samples_to_ms(completed.window.samples.len()) as u64,
        )
        .await;
        let words = words_from_tokens(
            &completed.decoded.text,
            &completed.decoded.tokens,
            completed.window.start_ms,
            &self.transcript.words,
        );
        if !completed.decoded.text.trim().is_empty() {
            if !self.transcript.text.is_empty() {
                self.transcript.text.push(' ');
            }
            self.transcript.text.push_str(completed.decoded.text.trim());
            self.prior_text = completed.decoded.text.trim().to_owned();
            self.transcript.words.extend(words);
        }
        if self.transcript.language.is_none() {
            self.transcript.language = completed.decoded.language.clone();
            if self
                .config
                .as_ref()
                .is_some_and(|cfg| cfg.language.is_none())
            {
                if let (Some(code), Some(confidence), Some(tx)) = (
                    completed.decoded.language.clone(),
                    completed.decoded.language_confidence,
                    &self.sender,
                ) {
                    let _ = tx
                        .send(AsrEvent::LanguageDetected { code, confidence })
                        .await;
                }
            }
        }
        Ok(())
    }

    async fn decode_window(&mut self, window: Window) -> Result<(), Error> {
        let prompt = self.initial_prompt();
        let duration_ms = samples_to_ms(window.samples.len());
        let decoder = Arc::clone(&self.decoder);
        let started = Instant::now();
        let decoded = tokio::task::spawn_blocking(move || {
            let mut decoder = decoder
                .lock()
                .map_err(|_| Error::AsrLoad("local decoder lock poisoned".into()))?;
            decoder.decode(&window.samples, &prompt)
        })
        .await
        .map_err(|error| Error::AsrLoad(format!("local decoder worker failed: {error}")))??;
        let elapsed = started.elapsed();

        self.record_decode_speed(elapsed.as_millis() as u64, duration_ms as u64)
            .await;
        let words = words_from_tokens(
            &decoded.text,
            &decoded.tokens,
            window.start_ms,
            &self.transcript.words,
        );
        if !decoded.text.trim().is_empty() {
            if !self.transcript.text.is_empty() {
                self.transcript.text.push(' ');
            }
            self.transcript.text.push_str(decoded.text.trim());
            self.prior_text = decoded.text.trim().to_owned();
            self.transcript.words.extend(words.iter().cloned());
        }
        if self.transcript.language.is_none() {
            self.transcript.language = decoded.language.clone();
            if self
                .config
                .as_ref()
                .is_some_and(|cfg| cfg.language.is_none())
            {
                if let (Some(code), Some(confidence), Some(tx)) =
                    (decoded.language, decoded.language_confidence, &self.sender)
                {
                    let _ = tx
                        .send(AsrEvent::LanguageDetected { code, confidence })
                        .await;
                }
            }
        }
        if let Some(tx) = &self.sender {
            let _ = tx
                .send(AsrEvent::Segment {
                    text: decoded.text,
                    words,
                })
                .await;
        }
        Ok(())
    }

    fn initial_prompt(&self) -> String {
        let carry = suffix_chars(&self.prior_text, INITIAL_PROMPT_CARRY_CHARS);
        match (self.dictionary_bias.trim(), carry.is_empty()) {
            ("", _) => carry,
            (bias, true) => bias.to_owned(),
            (bias, false) => format!("{bias}\n{carry}"),
        }
    }

    async fn record_decode_speed(&mut self, elapsed_ms: u64, duration_ms: u64) {
        if elapsed_ms > duration_ms.saturating_mul(3) / 2 {
            self.consecutive_slow_decodes += 1;
        } else {
            self.consecutive_slow_decodes = 0;
        }
        if self.consecutive_slow_decodes != 2 {
            return;
        }
        if let Some(tx) = &self.sender {
            let _ = tx
                .send(AsrEvent::Error {
                    error: Error::AsrSlow("local decoding is slow; choose a smaller model".into()),
                })
                .await;
        }
        if let Some(model) = self.config.as_ref().map(|cfg| cfg.local_model.as_str()) {
            self.effective_model_next_session = lower_model_tier(model).map(str::to_owned);
            if let (Some(persistence), Some(effective_model)) = (
                &self.effective_model_persistence,
                &self.effective_model_next_session,
            ) {
                if let Err(error) = persistence.set_effective_local_model(effective_model) {
                    if let Some(tx) = &self.sender {
                        let _ = tx.send(AsrEvent::Error { error }).await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl<D: LocalDecoder> SpeechRecognizer for LocalWhisperEngine<D> {
    async fn start(&mut self, cfg: AsrConfig, tx: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        for task in &self.live_tasks {
            task.abort();
        }
        self.live_tasks.clear();
        self.pending.clear();
        self.queued.clear();
        self.prior_text.clear();
        self.live_prompt
            .lock()
            .map_err(|_| Error::AsrLoad("local prompt lock poisoned".into()))?
            .clear();
        self.next_start_ms = 0;
        self.transcript = FinalTranscript {
            text: String::new(),
            words: Vec::new(),
            language: cfg.language.clone(),
        };
        self.consecutive_slow_decodes = 0;
        self.decoder
            .lock()
            .map_err(|_| Error::AsrLoad("local decoder lock poisoned".into()))?
            .configure(&cfg)?;
        self.config = Some(cfg);
        self.sender = Some(tx);
        Ok(())
    }

    fn feed(&mut self, pcm_16k_mono: &[f32]) -> Result<(), Error> {
        self.pending.extend_from_slice(pcm_16k_mono);
        while self.pending.len() >= WINDOW_SAMPLES {
            let remainder = self.pending.split_off(WINDOW_SAMPLES);
            self.close_pending_window();
            self.pending = remainder;
            self.schedule_live_decode();
        }
        Ok(())
    }

    fn endpoint_silence(&mut self, silence_ms: u32) -> Result<(), Error> {
        LocalWhisperEngine::endpoint_silence(self, silence_ms)
    }

    async fn service(&mut self) -> Result<(), Error> {
        self.service_live().await
    }

    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        self.close_pending_window();
        self.drain().await?;
        Ok(self.transcript.clone())
    }

    async fn abort(&mut self) {
        for task in &self.live_tasks {
            task.abort();
        }
        self.live_tasks.clear();
        self.pending.clear();
        self.queued.clear();
        self.sender = None;
    }
}

fn samples_to_ms(samples: usize) -> u32 {
    (samples * 1_000 / SAMPLE_RATE_HZ) as u32
}

fn suffix_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let start = text
        .char_indices()
        .rev()
        .nth(max_chars - 1)
        .map_or(0, |(index, _)| index);
    text[start..].to_owned()
}

fn lower_model_tier(model: &str) -> Option<&'static str> {
    match model {
        "large-v3-turbo" => Some("medium"),
        "medium" => Some("small"),
        "small" => Some("base"),
        _ => None,
    }
}

fn words_from_tokens(
    text: &str,
    tokens: &[TokenTiming],
    window_start_ms: u32,
    existing: &[WordTiming],
) -> Vec<WordTiming> {
    let mut words = Vec::new();
    let mut last_end = existing.last().map_or(0, |word| word.e);
    let mut pending = String::new();
    let mut pending_start = last_end;
    let mut pending_end = last_end;
    for token in tokens {
        let start = window_start_ms.saturating_add(token.start_ms).max(last_end);
        let end = window_start_ms.saturating_add(token.end_ms).max(start);
        for piece in token.text.split_inclusive(char::is_whitespace) {
            let whitespace = piece.chars().last().is_some_and(char::is_whitespace);
            if piece.starts_with(char::is_whitespace) && !pending.is_empty() {
                words.push(WordTiming {
                    w: std::mem::take(&mut pending),
                    s: pending_start,
                    e: pending_end,
                });
                last_end = pending_end;
            }
            if piece.trim().is_empty() {
                continue;
            }
            if pending.is_empty() {
                pending_start = start;
            }
            pending.push_str(piece.trim_end());
            pending_end = end;
            if whitespace {
                words.push(WordTiming {
                    w: std::mem::take(&mut pending),
                    s: pending_start,
                    e: pending_end,
                });
                last_end = pending_end;
            }
        }
    }
    if !pending.is_empty() {
        words.push(WordTiming {
            w: pending,
            s: pending_start,
            e: pending_end,
        });
    }
    if words.is_empty() {
        for word in text.split_whitespace() {
            words.push(WordTiming {
                w: word.to_owned(),
                s: last_end,
                e: last_end,
            });
        }
    }
    words
}

#[doc(hidden)]
pub fn merge_token_timings_for_test(
    text: &str,
    tokens: &[TokenTiming],
    start_ms: u32,
    existing: &[WordTiming],
) -> Vec<WordTiming> {
    words_from_tokens(text, tokens, start_ms, existing)
}

#[cfg(test)]
mod window_logic {
    use super::*;
    use std::sync::Mutex;

    struct Decoder;
    impl LocalDecoder for Decoder {
        fn decode(&mut self, _: &[f32], _: &str) -> Result<DecodedWindow, Error> {
            Ok(DecodedWindow {
                text: String::new(),
                tokens: vec![],
                language: None,
                language_confidence: None,
            })
        }
    }

    #[derive(Default)]
    struct RecordingPersistence(Mutex<Vec<String>>);
    impl EffectiveModelPersistence for RecordingPersistence {
        fn set_effective_local_model(&self, model: &str) -> Result<(), Error> {
            self.0.lock().unwrap().push(model.to_owned());
            Ok(())
        }
    }

    #[test]
    fn fr_1_1_initial_prompt_suffix_is_capped_at_200_unicode_characters() {
        let text = format!("a{}", "é".repeat(200));
        let suffix = suffix_chars(&text, INITIAL_PROMPT_CARRY_CHARS);
        assert_eq!(suffix.chars().count(), 200);
        assert_eq!(suffix, "é".repeat(200));
    }

    #[tokio::test]
    async fn fr_1_1_decode_slow_twice_emits_asr_slow_and_downgrades_next_session() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut engine = LocalWhisperEngine::new(Decoder);
        engine.start(AsrConfig::local("small"), tx).await.unwrap();

        engine.record_decode_speed(6_001, 4_000).await;
        engine.record_decode_speed(6_001, 4_000).await;

        assert!(matches!(
            rx.recv().await,
            Some(AsrEvent::Error {
                error: Error::AsrSlow(_)
            })
        ));
        assert_eq!(engine.effective_model_next_session(), Some("base"));
    }

    #[tokio::test]
    async fn fr_1_1_decode_slow_twice_persists_effective_local_model() {
        let (tx, _) = mpsc::channel(4);
        let persistence = Arc::new(RecordingPersistence::default());
        let mut engine = LocalWhisperEngine::new(Decoder);
        engine.set_effective_model_persistence(persistence.clone());
        engine.start(AsrConfig::local("medium"), tx).await.unwrap();

        engine.record_decode_speed(6_001, 4_000).await;
        engine.record_decode_speed(6_001, 4_000).await;

        assert_eq!(*persistence.0.lock().unwrap(), vec!["small"]);
    }
}
