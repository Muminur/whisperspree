//! Opt-in real-model ASR regression goldens (PRD §§17.1, 17.3).
//!
//! These are intentionally ignored: CI never downloads or owns a Whisper
//! model. Run with `WHISPERSPREE_TEST_MODEL=/path/to/ggml-tiny.bin cargo test
//! --manifest-path src-tauri/Cargo.toml asr_golden -- --ignored`.

use std::{env, fs};
use whisperspree_lib::{
    asr::local_whisper::{LocalDecoder, WhisperRsDecoder},
    testutil::wer::{alignment_diff, word_error_rate},
};

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures");

#[test]
#[ignore = "requires WHISPERSPREE_TEST_MODEL pointing to a locally installed ggml-tiny.bin"]
fn asr_golden_tiny_model_meets_fixture_wer_thresholds() {
    let model_path = env::var("WHISPERSPREE_TEST_MODEL")
        .expect("set WHISPERSPREE_TEST_MODEL to a local ggml-tiny.bin path");

    assert_fixture_wer(&model_path, "f1_short", 0.15);
    assert_fixture_wer(&model_path, "f2_long", 0.15);
    assert_fixture_wer(&model_path, "f3_es", 0.25);
}

fn assert_fixture_wer(model_path: &str, fixture_name: &str, threshold: f64) {
    let reference = fs::read_to_string(fixture_path(fixture_name, "txt"))
        .expect("fixture reference transcript must be committed");
    let audio = read_pcm16_mono_16khz_wav(&fixture_path(fixture_name, "wav"));
    let decoded = WhisperRsDecoder::load(model_path)
        .expect("WHISPERSPREE_TEST_MODEL must name a readable Whisper model")
        .decode(&audio, "")
        .expect("fixture audio must decode with the supplied model");
    let wer = word_error_rate(&reference, &decoded.text);

    assert!(
        wer <= threshold,
        "{fixture_name}: WER {wer:.3} exceeds {threshold:.3}\nreference: {reference}\nhypothesis: {}\nalignment:\n{}",
        decoded.text,
        alignment_diff(&reference, &decoded.text),
    );
}

fn fixture_path(name: &str, extension: &str) -> String {
    format!("{FIXTURES_DIR}/{name}.{extension}")
}

fn read_pcm16_mono_16khz_wav(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect("fixture WAV must be committed");
    assert!(bytes.len() >= 44 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE");

    let mut cursor = 12;
    let mut format = None;
    let mut data = None;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let length = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        let end = cursor
            .checked_add(length)
            .expect("fixture WAV chunk length overflow");
        assert!(end <= bytes.len(), "fixture WAV contains a truncated chunk");
        match id {
            b"fmt " => format = Some(&bytes[cursor..end]),
            b"data" => data = Some(&bytes[cursor..end]),
            _ => {}
        }
        cursor = end + length % 2;
    }

    let format = format.expect("fixture WAV must include fmt chunk");
    assert!(format.len() >= 16, "fixture WAV fmt chunk is too short");
    assert_eq!(
        u16::from_le_bytes(format[0..2].try_into().unwrap()),
        1,
        "fixture WAV must be PCM"
    );
    assert_eq!(
        u16::from_le_bytes(format[2..4].try_into().unwrap()),
        1,
        "fixture WAV must be mono"
    );
    assert_eq!(
        u32::from_le_bytes(format[4..8].try_into().unwrap()),
        16_000,
        "fixture WAV must be 16 kHz"
    );
    assert_eq!(
        u16::from_le_bytes(format[14..16].try_into().unwrap()),
        16,
        "fixture WAV must be 16-bit"
    );

    let data = data.expect("fixture WAV must include data chunk");
    assert_eq!(
        data.len() % 2,
        0,
        "fixture WAV PCM data must have whole samples"
    );
    data.chunks_exact(2)
        .map(|sample| i16::from_le_bytes(sample.try_into().unwrap()) as f32 / i16::MAX as f32)
        .collect()
}
