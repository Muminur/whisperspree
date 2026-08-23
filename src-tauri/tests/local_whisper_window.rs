//! T1.4 red tests for the FR-1.1.b local tumbling-window engine.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use whisperspree_lib::{
    asr::{
        local_whisper::{DecodedWindow, LocalDecoder, LocalWhisperEngine, TokenTiming},
        AsrConfig, AsrEvent, SpeechRecognizer,
    },
    error::Error,
};

const SAMPLE_RATE: usize = 16_000;

#[test]
fn fr_1_1_physical_core_thread_selection_is_clamped_to_one_through_eight() {
    assert_eq!(
        whisperspree_lib::asr::local_whisper::select_thread_count(Some(12), 32),
        8
    );
    assert_eq!(
        whisperspree_lib::asr::local_whisper::select_thread_count(Some(4), 32),
        4
    );
    assert_eq!(
        whisperspree_lib::asr::local_whisper::select_thread_count(Some(0), 0),
        1
    );
}

#[derive(Clone, Default)]
struct RecordingDecoder {
    calls: Arc<Mutex<Vec<(usize, String)>>>,
}

struct FailingDecoder(Error);

struct BlockingDecoder;
impl LocalDecoder for BlockingDecoder {
    fn decode(&mut self, _: &[f32], _: &str) -> Result<DecodedWindow, Error> {
        std::thread::sleep(Duration::from_millis(75));
        Ok(DecodedWindow {
            text: "live".into(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        })
    }
}

#[derive(Clone)]
struct ScriptedDecoder {
    responses: Arc<Mutex<Vec<DecodedWindow>>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ScriptedDecoder {
    fn new(responses: Vec<DecodedWindow>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl LocalDecoder for ScriptedDecoder {
    fn decode(&mut self, _: &[f32], prompt: &str) -> Result<DecodedWindow, Error> {
        self.prompts.lock().unwrap().push(prompt.to_owned());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

impl LocalDecoder for FailingDecoder {
    fn decode(&mut self, _: &[f32], _: &str) -> Result<DecodedWindow, Error> {
        Err(match &self.0 {
            Error::AsrNoModel(message) => Error::AsrNoModel(message.clone()),
            Error::AsrLoad(message) => Error::AsrLoad(message.clone()),
            _ => unreachable!("T1.4 failure fixture uses ASR model errors only"),
        })
    }
}

impl RecordingDecoder {
    fn calls(&self) -> Vec<(usize, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl LocalDecoder for RecordingDecoder {
    fn decode(&mut self, samples: &[f32], initial_prompt: &str) -> Result<DecodedWindow, Error> {
        self.calls
            .lock()
            .unwrap()
            .push((samples.len(), initial_prompt.to_owned()));
        Ok(DecodedWindow {
            text: format!("window{}", self.calls().len()),
            tokens: vec![TokenTiming {
                text: format!("window{}", self.calls().len()),
                start_ms: 0,
                end_ms: (samples.len() * 1_000 / SAMPLE_RATE) as u32,
            }],
            language: Some("en".into()),
            language_confidence: Some(0.99),
        })
    }
}

fn samples(ms: usize) -> Vec<f32> {
    vec![0.25; SAMPLE_RATE * ms / 1_000]
}

async fn engine(
    decoder: RecordingDecoder,
) -> (
    LocalWhisperEngine<RecordingDecoder>,
    mpsc::Receiver<AsrEvent>,
) {
    let (tx, rx) = mpsc::channel(16);
    let mut engine = LocalWhisperEngine::new(decoder);
    let mut cfg = AsrConfig::local("base");
    cfg.language = Some("en".into());
    engine.start(cfg, tx).await.unwrap();
    (engine, rx)
}

#[tokio::test]
async fn fr_1_1_window_closes_at_4s() {
    let decoder = RecordingDecoder::default();
    let (mut engine, mut events) = engine(decoder.clone()).await;

    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();

    assert_eq!(decoder.calls(), vec![(64_000, String::new())]);
    let mut saw_segment = false;
    while let Ok(event) = events.try_recv() {
        match event {
            AsrEvent::Segment { text, .. } if text == "window1" => saw_segment = true,
            AsrEvent::LanguageDetected { .. } => {}
            unexpected => panic!("unexpected local ASR event: {unexpected:?}"),
        }
    }
    assert!(saw_segment, "window decode must emit transcript:segment");
}

#[tokio::test]
async fn fr_1_1_feed_returns_while_serial_worker_decodes_and_emits_segment() {
    let (tx, mut events) = mpsc::channel(4);
    let mut engine = LocalWhisperEngine::new(BlockingDecoder);
    engine.start(AsrConfig::local("base"), tx).await.unwrap();

    let started = Instant::now();
    engine.feed(&samples(4_000)).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(30),
        "feed must not block on local decode"
    );

    let event = tokio::time::timeout(Duration::from_millis(250), events.recv()).await;
    assert!(matches!(event, Ok(Some(AsrEvent::Segment { text, .. })) if text == "live"));
}

#[tokio::test]
async fn fr_1_1_live_service_reaps_completed_window_before_finalize() {
    let decoder = RecordingDecoder::default();
    let (mut engine, mut events) = engine(decoder.clone()).await;

    engine.feed(&samples(8_000)).unwrap();
    let mut segments = 0;
    for _ in 0..100 {
        engine.service_live().await.unwrap();
        while let Ok(event) = events.try_recv() {
            if matches!(event, AsrEvent::Segment { .. }) {
                segments += 1;
            }
        }
        if segments >= 2 {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(
        decoder.calls().len(),
        2,
        "two live windows must decode before finalize"
    );
    assert_eq!(
        segments, 2,
        "each live window must publish its segment immediately"
    );
}

#[tokio::test]
async fn starting_a_new_session_does_not_carry_live_prompt() {
    let decoder = ScriptedDecoder::new(vec![
        DecodedWindow {
            text: "first".into(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        },
        DecodedWindow {
            text: "second".into(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        },
    ]);
    let prompts = decoder.prompts.clone();
    let (tx, _rx) = mpsc::channel(8);
    let mut engine = LocalWhisperEngine::new(decoder);
    engine
        .start(AsrConfig::local("base"), tx.clone())
        .await
        .unwrap();
    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();
    engine.start(AsrConfig::local("base"), tx).await.unwrap();
    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();
    let prompts = prompts.lock().unwrap().clone();
    assert_eq!(prompts.last().map(String::as_str), Some(""));
}

#[tokio::test]
async fn fr_1_1_window_closes_on_700ms_endpoint_after_600ms_speech() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder.clone()).await;

    engine.feed(&samples(600)).unwrap();
    engine.endpoint_silence(700).unwrap();
    engine.drain().await.unwrap();

    assert_eq!(decoder.calls(), vec![(9_600, String::new())]);
}

#[tokio::test]
async fn fr_1_1_finalize_flushes_short_pending_window() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder.clone()).await;

    engine.feed(&samples(240)).unwrap();
    let final_transcript = engine.finalize().await.unwrap();

    assert_eq!(decoder.calls(), vec![(3_840, String::new())]);
    assert_eq!(final_transcript.text, "window1");
}

#[tokio::test]
async fn fr_1_1_windows_are_non_overlapping_and_decode_serialized() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder.clone()).await;

    engine.feed(&samples(8_000)).unwrap();
    engine.drain().await.unwrap();

    assert_eq!(
        decoder.calls(),
        vec![(64_000, String::new()), (64_000, "window1".into())]
    );
}

#[tokio::test]
async fn fr_1_1_queue_depth_two_concatenates_overflow() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder.clone()).await;

    engine.feed(&samples(12_000)).unwrap();
    engine.drain().await.unwrap();

    assert_eq!(
        decoder.calls(),
        vec![(64_000, String::new()), (128_000, "window1".into())]
    );
}

#[tokio::test]
async fn fr_1_1_initial_prompt_carries_last_200_chars_after_dictionary_bias() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder.clone()).await;
    let bias = "Vocabulary: Rust, WhisperSpree.";
    engine.set_dictionary_bias(bias);

    engine.feed(&samples(8_000)).unwrap();
    engine.drain().await.unwrap();

    let calls = decoder.calls();
    assert_eq!(calls[0].1, bias);
    assert_eq!(calls[1].1, format!("{bias}\nwindow1"));
}

#[tokio::test]
async fn fr_1_1_initial_prompt_uses_only_the_prior_window_suffix_for_three_windows() {
    let long_first = format!("A{}", "é".repeat(220));
    let decoder = ScriptedDecoder::new(vec![
        DecodedWindow {
            text: long_first.clone(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        },
        DecodedWindow {
            text: "second".into(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        },
        DecodedWindow {
            text: "third".into(),
            tokens: vec![],
            language: None,
            language_confidence: None,
        },
    ]);
    let (tx, _) = mpsc::channel(16);
    let mut engine = LocalWhisperEngine::new(decoder.clone());
    engine.set_dictionary_bias("Vocabulary: Rust.");
    engine.start(AsrConfig::local("base"), tx).await.unwrap();

    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();
    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();
    engine.feed(&samples(4_000)).unwrap();
    engine.drain().await.unwrap();

    assert_eq!(
        decoder.prompts()[1],
        format!("Vocabulary: Rust.\n{}", "é".repeat(200))
    );
    assert_eq!(decoder.prompts()[2], "Vocabulary: Rust.\nsecond");
}

#[test]
fn fr_1_1_subword_tokens_merge_into_whitespace_words() {
    let words = whisperspree_lib::asr::local_whisper::merge_token_timings_for_test(
        "hello world",
        &[
            TokenTiming {
                text: "hel".into(),
                start_ms: 0,
                end_ms: 100,
            },
            TokenTiming {
                text: "lo".into(),
                start_ms: 100,
                end_ms: 200,
            },
            TokenTiming {
                text: " world".into(),
                start_ms: 200,
                end_ms: 400,
            },
        ],
        4_000,
        &[],
    );

    assert_eq!(
        words,
        vec![
            whisperspree_lib::ipc::events::WordTiming {
                w: "hello".into(),
                s: 4_000,
                e: 4_200
            },
            whisperspree_lib::ipc::events::WordTiming {
                w: "world".into(),
                s: 4_200,
                e: 4_400
            },
        ]
    );
}

#[test]
fn fr_1_1_overflow_window_source_timeline_remains_contiguous() {
    let decoder = RecordingDecoder::default();
    let mut engine = LocalWhisperEngine::new(decoder);
    engine.feed(&samples(12_000)).unwrap();
    assert_eq!(
        engine.queued_timeline_for_test(),
        vec![(0, 4_000), (4_000, 12_000)]
    );
}

#[tokio::test]
async fn fr_1_1_word_timings_are_monotonic_and_cover_text() {
    let decoder = RecordingDecoder::default();
    let (mut engine, _) = engine(decoder).await;

    engine.feed(&samples(8_000)).unwrap();
    let final_transcript = engine.finalize().await.unwrap();

    assert_eq!(final_transcript.text, "window1 window2");
    assert_eq!(final_transcript.words.len(), 2);
    assert_eq!(final_transcript.words[0].w, "window1");
    assert_eq!(final_transcript.words[0].s, 0);
    assert_eq!(final_transcript.words[0].e, 4_000);
    assert_eq!(final_transcript.words[1].w, "window2");
    assert_eq!(final_transcript.words[1].s, 4_000);
    assert_eq!(final_transcript.words[1].e, 8_000);
}

#[tokio::test]
async fn fr_1_1_no_installed_model_returns_asr_no_model_without_network() {
    let (tx, _) = mpsc::channel(1);
    let mut engine = LocalWhisperEngine::new(FailingDecoder(Error::AsrNoModel(
        "no installed local model".into(),
    )));
    engine.start(AsrConfig::local("base"), tx).await.unwrap();
    engine.feed(&samples(240)).unwrap();

    assert!(matches!(engine.finalize().await, Err(Error::AsrNoModel(_))));
}

#[tokio::test]
async fn fr_1_1_corrupt_model_decoder_returns_asr_load_without_network() {
    let (tx, _) = mpsc::channel(1);
    let mut engine =
        LocalWhisperEngine::new(FailingDecoder(Error::AsrLoad("model corrupt".into())));
    engine.start(AsrConfig::local("base"), tx).await.unwrap();
    engine.feed(&samples(240)).unwrap();

    assert!(matches!(engine.finalize().await, Err(Error::AsrLoad(_))));
}
