//! Deterministic dictation commands and wake-phrase intent parsing (T4.4).
//!
//! This module has no provider or session side effects.  The session pipeline
//! supplies final ASR segments, then uses [`CommandOutput::cancelled`] to stop
//! the session or [`WakePhrase`] plus [`parse_intent_response`] to request a
//! transform.  Keeping those boundaries explicit makes malformed model output
//! fail open to ordinary dictation (FR-3.3).

use crate::llm::prompts::{PersonaId, TemplateId};
use serde::Deserialize;

/// A transform or persona allowed by the closed `INTENT_DETECT` prompt list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Template(TemplateId),
    Persona(PersonaId),
}

/// The instruction that follows a leading `whisper` wake phrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePhrase {
    pub instruction: String,
}

/// Output of deterministic command processing.  Segments remain distinct so a
/// subsequent utterance can delete the most recent final ASR segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub segments: Vec<String>,
    pub cancelled: bool,
}

impl CommandOutput {
    pub fn text(&self) -> String {
        self.segments.join(" ")
    }
}

/// Pure command parser for one final transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandLayer {
    enabled: bool,
}

impl CommandLayer {
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Applies FR-3.3 deterministic commands to ordered final ASR segments.
    ///
    /// `new paragraph` is handled before `new line` (longest-match-first).
    /// A standalone delete command removes an earlier segment, including one
    /// emitted by a prior final ASR event.  With only one segment, it removes
    /// the preceding sentence from that segment instead.
    pub fn apply(&self, segments: impl IntoIterator<Item = String>) -> CommandOutput {
        let mut output = CommandOutput {
            segments: Vec::new(),
            cancelled: false,
        };

        for segment in segments {
            if !self.enabled {
                output.segments.push(segment);
                continue;
            }

            let normalized = normalize_command(&segment);
            if normalized == "cancel dictation" {
                output.cancelled = true;
                output.segments.clear();
                break;
            }
            if normalized == "scratch that" || normalized == "delete that" {
                remove_most_recent(&mut output.segments);
                continue;
            }

            output.segments.push(replace_layout_commands(&segment));
        }
        output
    }

    /// Returns a wake-phrase instruction only when it begins the transcript.
    /// The instruction is deliberately left intact until the strict intent
    /// result is available; a `none`/malformed result must inject the original
    /// utterance unchanged (FR-3.3 fail-open).
    pub fn wake_phrase(&self, transcript: &str) -> Option<WakePhrase> {
        if !self.enabled {
            return None;
        }
        strip_wake_prefix(transcript).map(|instruction| WakePhrase { instruction })
    }

    /// Applies a resolved wake intent.  Unknown or malformed output returns the
    /// original transcript, which is the required fail-open behavior.
    pub fn resolve_wake_phrase(&self, transcript: &str, model_response: &str) -> WakeResolution {
        let Some(wake) = self.wake_phrase(transcript) else {
            return WakeResolution::Dictation(transcript.to_owned());
        };
        match parse_intent_response(model_response) {
            Some(intent) => WakeResolution::Transform { wake, intent },
            None => WakeResolution::Dictation(transcript.to_owned()),
        }
    }
}

impl Default for CommandLayer {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Whether a wake phrase was safely mapped to a transform or remains dictation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeResolution {
    Transform { wake: WakePhrase, intent: Intent },
    Dictation(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentEnvelope {
    intent: String,
}

/// Strictly parses the exact closed JSON response specified by PRD §7.6.
///
/// `none`, an unknown identifier, extra fields, surrounding prose, and every
/// malformed JSON form intentionally produce `None`.
pub fn parse_intent_response(response: &str) -> Option<Intent> {
    let parsed: IntentEnvelope = serde_json::from_str(response).ok()?;
    if parsed.intent == "none" {
        return None;
    }
    if let Some(template) = TemplateId::from_id(&parsed.intent) {
        return Some(Intent::Template(template));
    }
    match parsed.intent.as_str() {
        "formal" => Some(Intent::Persona(PersonaId::Formal)),
        "casual" => Some(Intent::Persona(PersonaId::Casual)),
        "polite" => Some(Intent::Persona(PersonaId::Polite)),
        "funny" => Some(Intent::Persona(PersonaId::Funny)),
        "social" => Some(Intent::Persona(PersonaId::Social)),
        _ => None,
    }
}

fn strip_wake_prefix(transcript: &str) -> Option<String> {
    let trimmed = transcript.trim_start();
    let lower = trimmed.to_lowercase();
    let after = if lower.starts_with("hey whisper") {
        &trimmed["hey whisper".len()..]
    } else if lower.starts_with("whisper") {
        &trimmed["whisper".len()..]
    } else {
        return None;
    };
    if !matches!(after.chars().next(), Some(',') | Some(':'))
        && !after
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace())
    {
        return None;
    }
    let instruction = after.trim_start_matches(|c: char| c == ',' || c == ':' || c.is_whitespace());
    (!instruction.is_empty()).then(|| instruction.to_owned())
}

fn normalize_command(segment: &str) -> String {
    segment
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn replace_layout_commands(segment: &str) -> String {
    // Delimiters ensure command words embedded in a larger token are untouched.
    let paragraph = regex::Regex::new(r"(?i)\bnew\s+paragraph\b").expect("literal regex");
    let line = regex::Regex::new(r"(?i)\bnew\s+line\b").expect("literal regex");
    line.replace_all(&paragraph.replace_all(segment, "\n\n"), "\n")
        .into_owned()
}

fn remove_most_recent(segments: &mut Vec<String>) {
    if segments.len() > 1 {
        segments.pop();
    } else if let Some(segment) = segments.last_mut() {
        *segment = without_preceding_sentence(segment).to_owned();
        if segment.trim().is_empty() {
            segments.pop();
        }
    }
}

fn without_preceding_sentence(segment: &str) -> &str {
    let trimmed = segment.trim_end();
    match trimmed.rfind(['.', '!', '?']) {
        Some(last_boundary) => match trimmed[..last_boundary].rfind(['.', '!', '?']) {
            Some(previous_boundary) => trimmed[..=previous_boundary].trim_end(),
            None => "",
        },
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_3_3_replaces_new_paragraph_before_new_line() {
        let result = CommandLayer::default().apply(["one new paragraph two new line three".into()]);
        assert_eq!(result.text(), "one \n\n two \n three");
        assert!(!result.cancelled);
    }

    #[test]
    fn fr_3_3_scratch_that_removes_prior_asr_segment_across_boundary() {
        let result = CommandLayer::default().apply([
            "keep this".into(),
            "remove this".into(),
            "scratch that".into(),
        ]);
        assert_eq!(result.segments, ["keep this"]);
    }

    #[test]
    fn fr_3_3_delete_that_removes_preceding_sentence_when_only_one_segment() {
        let result =
            CommandLayer::default().apply(["Keep this. Remove this.".into(), "delete that".into()]);
        assert_eq!(result.text(), "Keep this.");
    }

    #[test]
    fn fr_3_3_cancel_dictation_discards_everything() {
        let result =
            CommandLayer::default().apply(["do not inject".into(), "cancel dictation".into()]);
        assert!(result.cancelled);
        assert!(result.segments.is_empty());
    }

    #[test]
    fn fr_3_3_commands_disabled_keeps_literal_words() {
        let result = CommandLayer::new(false).apply(["new line cancel dictation".into()]);
        assert_eq!(result.text(), "new line cancel dictation");
        assert!(!result.cancelled);
    }

    #[test]
    fn fr_3_3_strict_json_accepts_only_closed_allowed_intents() {
        assert_eq!(
            parse_intent_response(r#"{"intent":"tweet"}"#),
            Some(Intent::Template(TemplateId::Tweet))
        );
        assert_eq!(
            parse_intent_response(r#"{"intent":"formal"}"#),
            Some(Intent::Persona(PersonaId::Formal))
        );
        assert_eq!(parse_intent_response(r#"{"intent":"none"}"#), None);
        assert_eq!(parse_intent_response(r#"{"intent":"open_chrome"}"#), None);
        assert_eq!(
            parse_intent_response(r#"{"intent":"tweet","extra":true}"#),
            None
        );
        assert_eq!(parse_intent_response("Here: {\"intent\":\"tweet\"}"), None);
    }

    #[test]
    fn fr_3_3_unknown_wake_instruction_fails_open_to_literal_dictation() {
        let transcript = "Hey Whisper, open Chrome";
        assert_eq!(
            CommandLayer::default().resolve_wake_phrase(transcript, r#"{"intent":"none"}"#),
            WakeResolution::Dictation(transcript.into())
        );
    }

    #[test]
    fn fr_3_3_wake_prefix_is_anchored_and_strips_instruction() {
        assert_eq!(
            CommandLayer::default().wake_phrase("  whisper: make that a tweet"),
            Some(WakePhrase {
                instruction: "make that a tweet".into()
            })
        );
        assert_eq!(
            CommandLayer::default().wake_phrase("I told Whisper to make a tweet"),
            None
        );
        assert_eq!(
            CommandLayer::default().wake_phrase("whispering is hard"),
            None
        );
    }
}
