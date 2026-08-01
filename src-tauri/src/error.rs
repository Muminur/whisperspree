//! Error taxonomy, `ApiError` wire type, and the P-3 secret-redaction layer.
//!
//! **T0.2 — implemented (tests-first per CLAUDE.md §3).** The colocated tests
//! below were written before the implementation and pin the exact behaviour of
//! the taxonomy, the `ApiError` wire shape, and the redaction layer.
//!
//! PRD refs:
//! - §14 — the 14 stable error codes (used in `app:error`, HUD chips, tests).
//! - §9.1 — commands return `Result<T, ApiError>` where `ApiError { code, message }`.
//! - §9.2 — the `app:error` event payload is `{ code, message, recoverable }`.
//! - §12 P-3 — secrets (`sk-ant-…`, `Token …`) never reach settings.json, the DB,
//!   logs, or error payloads; a redaction pass strips them from tracing output.
//! - OPEN_QUESTIONS Q6 — the pinned `recoverable` mapping (§14 has no such column).

// ---------------------------------------------------------------------------
// §14 — error taxonomy
// ---------------------------------------------------------------------------

/// Every WhisperSpree error maps to exactly one **stable** §14 code. Each variant
/// carries a human-readable message; that message is redacted (P-3) on its way to
/// the frontend via [`ApiError`] and is never allowed to leak a secret.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `MIC-PERM` — microphone permission denied.
    #[error("{0}")]
    MicPerm(String),
    /// `MIC-DEV` — input device vanished mid-session.
    #[error("{0}")]
    MicDev(String),
    /// `HK-PERM` — Input Monitoring permission missing.
    #[error("{0}")]
    HkPerm(String),
    /// `AX-PERM` — Accessibility permission missing.
    #[error("{0}")]
    AxPerm(String),
    /// `ASR-NOMODEL` — local mode selected but no model is installed.
    #[error("{0}")]
    AsrNoModel(String),
    /// `ASR-SLOW` — local decode too slow (FR-1.1.b(7)).
    #[error("{0}")]
    AsrSlow(String),
    /// `ASR-LOAD` — model load failed / file corrupt.
    #[error("{0}")]
    AsrLoad(String),
    /// `NET-STREAM` — Deepgram connect/auth failure.
    #[error("{0}")]
    NetStream(String),
    /// `LLM-AUTH` — Anthropic 401.
    #[error("{0}")]
    LlmAuth(String),
    /// `LLM-TIMEOUT` — post-process exceeded `timeoutMs`.
    #[error("{0}")]
    LlmTimeout(String),
    /// `LLM-VERIFY` — §7.4 output verifier rejected the result.
    #[error("{0}")]
    LlmVerify(String),
    /// `INJ-FAIL` — every injection strategy failed.
    #[error("{0}")]
    InjFail(String),
    /// `DB-IO` — SQLite failure.
    #[error("{0}")]
    DbIo(String),
    /// `SEC-FIELD` — a secure-input field is active (FR-1.4).
    #[error("{0}")]
    SecField(String),
}

impl Error {
    /// The stable §14 code string for this error (e.g. `"LLM-AUTH"`).
    ///
    /// Pinned by `error::tests::all_14_codes_roundtrip_stable_strings`.
    pub fn code(&self) -> &'static str {
        match self {
            Error::MicPerm(_) => "MIC-PERM",
            Error::MicDev(_) => "MIC-DEV",
            Error::HkPerm(_) => "HK-PERM",
            Error::AxPerm(_) => "AX-PERM",
            Error::AsrNoModel(_) => "ASR-NOMODEL",
            Error::AsrSlow(_) => "ASR-SLOW",
            Error::AsrLoad(_) => "ASR-LOAD",
            Error::NetStream(_) => "NET-STREAM",
            Error::LlmAuth(_) => "LLM-AUTH",
            Error::LlmTimeout(_) => "LLM-TIMEOUT",
            Error::LlmVerify(_) => "LLM-VERIFY",
            Error::InjFail(_) => "INJ-FAIL",
            Error::DbIo(_) => "DB-IO",
            Error::SecField(_) => "SEC-FIELD",
        }
    }

    /// Whether the session can continue / degrade **without** the user having to
    /// unblock something (used for `app:error { recoverable }`, §9.2).
    ///
    /// The §14 matrix has no `recoverable` column, so the true/false split is the
    /// per-row derivation pinned in OPEN_QUESTIONS Q6 and enforced by
    /// `error::tests::recoverable_flag_matches_matrix`.
    // PRD-QUESTION(Q6): recoverable:true  = MIC-DEV, ASR-SLOW, NET-STREAM,
    //   LLM-AUTH, LLM-TIMEOUT, LLM-VERIFY, INJ-FAIL, DB-IO, SEC-FIELD, AX-PERM;
    //   recoverable:false = MIC-PERM, HK-PERM, ASR-NOMODEL, ASR-LOAD.
    //   AX-PERM is the borderline: it is `true` because injection degrades to
    //   clipboard_only (§14 Recovery) instead of blocking the session task.
    pub fn recoverable(&self) -> bool {
        // One arm per §14 row, ordered as the matrix; the true/false split is the
        // Q6 derivation marked above. AX-PERM is the borderline `true`.
        match self {
            Error::MicPerm(_) => false,
            Error::MicDev(_) => true,
            Error::HkPerm(_) => false,
            Error::AxPerm(_) => true,
            Error::AsrNoModel(_) => false,
            Error::AsrSlow(_) => true,
            Error::AsrLoad(_) => false,
            Error::NetStream(_) => true,
            Error::LlmAuth(_) => true,
            Error::LlmTimeout(_) => true,
            Error::LlmVerify(_) => true,
            Error::InjFail(_) => true,
            Error::DbIo(_) => true,
            Error::SecField(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// §9.1 / §9.2 — the frontend-facing error shape
// ---------------------------------------------------------------------------

/// The serialized error every IPC command returns on the `Err` path
/// (`Result<T, ApiError>`, §9.1) and the core of the `app:error` event (§9.2).
///
/// `Serialize` is hand-written (rust-m0-brief) so the two fields reach the
/// frontend verbatim as `{"code":"…","message":"…"}` — no extra fields, so the
/// §14 codes stay a stable API surface.
pub struct ApiError {
    /// A stable §14 code (see [`Error::code`]).
    pub code: String,
    /// A human-readable, **already-redacted** (P-3) message.
    pub message: String,
}

impl serde::Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // Exactly two fields, in this order → `{"code":…,"message":…}` with
        // serde_json. No extra fields, so §14 codes stay a stable API surface.
        let mut state = serializer.serialize_struct("ApiError", 2)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("message", &self.message)?;
        state.end()
    }
}

impl From<&Error> for ApiError {
    fn from(err: &Error) -> Self {
        // The message MUST pass through `redact` so P-3 holds for every error
        // payload that crosses the IPC boundary (§12 P-3 / §14 "codes never carry
        // secrets").
        ApiError {
            code: err.code().into(),
            message: redact(&err.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// §12 P-3 — secret redaction
// ---------------------------------------------------------------------------

/// Strip secrets from `input` before it can reach a log line or an error payload.
///
/// Contract pinned by the `redaction_*` tests:
/// - an `sk-ant-…` token becomes `[REDACTED]`
///   (`"key sk-ant-abc123XYZ done"` → `"key [REDACTED] done"`);
/// - a `Token <value>` header keeps the word `Token` and masks the value
///   (`"Authorization: Token dg_secret_key9"` → `"Authorization: Token [REDACTED]"`);
/// - text containing neither pattern is returned byte-identical.
pub fn redact(input: &str) -> String {
    // Anthropic keys: drop the whole `sk-ant-…` token (prefix + value).
    let stripped = sk_ant_re().replace_all(input, REDACTED);
    // Bearer/Deepgram tokens: keep the literal word `Token`, mask only the value.
    token_re()
        .replace_all(&stripped, format!("Token {REDACTED}").as_str())
        .into_owned()
}

/// The marker every redacted secret collapses to (pinned by the `redaction_*` tests).
const REDACTED: &str = "[REDACTED]";

/// `sk-ant-<value>` — the Anthropic key shape (P-3, matches `sk-ant-|` in §12).
fn sk_ant_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"sk-ant-\S*").expect("static regex is valid"))
}

/// `Token <value>` — the Deepgram/Bearer header shape (P-3, matches `Token ` in §12).
fn token_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"Token \S+").expect("static regex is valid"))
}

/// A [`tracing_subscriber::fmt::MakeWriter`] wrapper that runs every formatted log
/// line through [`redact`] before it reaches the inner sink (`M`). This is the
/// P-3 idiom from rust-m0-brief: layers cannot mutate event fields, so redaction
/// happens at the writer boundary.
pub struct RedactingWriter<M> {
    inner: M,
}

impl<M> RedactingWriter<M> {
    /// Wrap an inner `MakeWriter` (e.g. a rolling-file appender or a test buffer).
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> tracing_subscriber::fmt::MakeWriter<'a> for RedactingWriter<M>
where
    M: tracing_subscriber::fmt::MakeWriter<'a>,
{
    type Writer = RedactingSink<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingSink {
            inner: self.inner.make_writer(),
        }
    }
}

/// The per-write `io::Write` produced by [`RedactingWriter`]; it redacts each
/// buffer before forwarding it to the wrapped writer.
pub struct RedactingSink<W> {
    inner: W,
}

impl<W: std::io::Write> std::io::Write for RedactingSink<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // P-3 LOAD-BEARING INVARIANT: each `write` is redacted independently, so a
        // secret split across two `write` calls would escape. This is safe because
        // the pinned tracing-subscriber fmt layer emits every formatted event as a
        // single `write_all` (proven by `tracing_output_never_contains_secret`).
        // Never insert a line-/chunk-buffering writer between fmt and this sink;
        // if the formatter ever writes fields separately, buffer here first.
        let redacted = redact(&String::from_utf8_lossy(buf));
        self.inner.write_all(redacted.as_bytes())?;
        // Report the ORIGINAL length: the fmt layer counts source bytes, not the
        // (possibly shorter) redacted output, or it will retry the "unwritten" tail.
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

// Logging bootstrap (`init_tracing` + `log_dir`) lives in `lib.rs`, not here:
// it installs a *global* subscriber, spawns a worker thread, and touches the
// real filesystem, so it is headless-untestable process glue and belongs in the
// coverage-ignored bootstrap file (OPEN_QUESTIONS Q7). It wraps the tested
// [`RedactingWriter`] / [`redact`] logic that stays in this module.

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test error::` filters to this module).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// AC: §14 — every variant's `code()` equals its exact stable §14 string.
    /// Table-driven over all 14 codes (MIC-PERM … SEC-FIELD).
    #[test]
    fn all_14_codes_roundtrip_stable_strings() {
        let cases: [(Error, &str); 14] = [
            (Error::MicPerm(String::new()), "MIC-PERM"),
            (Error::MicDev(String::new()), "MIC-DEV"),
            (Error::HkPerm(String::new()), "HK-PERM"),
            (Error::AxPerm(String::new()), "AX-PERM"),
            (Error::AsrNoModel(String::new()), "ASR-NOMODEL"),
            (Error::AsrSlow(String::new()), "ASR-SLOW"),
            (Error::AsrLoad(String::new()), "ASR-LOAD"),
            (Error::NetStream(String::new()), "NET-STREAM"),
            (Error::LlmAuth(String::new()), "LLM-AUTH"),
            (Error::LlmTimeout(String::new()), "LLM-TIMEOUT"),
            (Error::LlmVerify(String::new()), "LLM-VERIFY"),
            (Error::InjFail(String::new()), "INJ-FAIL"),
            (Error::DbIo(String::new()), "DB-IO"),
            (Error::SecField(String::new()), "SEC-FIELD"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.code(), expected, "wrong §14 code for {expected}");
        }
    }

    /// AC: §9.1 — `ApiError` serializes to exactly `{"code":…,"message":…}` with
    /// no extra fields, so §14 codes reach the frontend verbatim.
    #[test]
    fn apierror_serializes_code_and_message() {
        let api = ApiError {
            code: "LLM-AUTH".to_string(),
            message: "AI polish off — check API key".to_string(),
        };
        let json = serde_json::to_string(&api).expect("ApiError must serialize");
        assert_eq!(
            json,
            r#"{"code":"LLM-AUTH","message":"AI polish off — check API key"}"#,
        );
    }

    /// AC: §9.2 / OPEN_QUESTIONS Q6 — `recoverable()` matches the pinned matrix.
    /// Table-driven; AX-PERM is the borderline `true` (injection degrades to
    /// clipboard_only per §14 Recovery, so the session task never crashes — Q6).
    #[test]
    fn recoverable_flag_matches_matrix() {
        let cases: [(Error, bool); 14] = [
            (Error::MicPerm(String::new()), false),
            (Error::MicDev(String::new()), true),
            (Error::HkPerm(String::new()), false),
            // AX-PERM: borderline true — injection degrades to clipboard_only (Q6).
            (Error::AxPerm(String::new()), true),
            (Error::AsrNoModel(String::new()), false),
            (Error::AsrSlow(String::new()), true),
            (Error::AsrLoad(String::new()), false),
            (Error::NetStream(String::new()), true),
            (Error::LlmAuth(String::new()), true),
            (Error::LlmTimeout(String::new()), true),
            (Error::LlmVerify(String::new()), true),
            (Error::InjFail(String::new()), true),
            (Error::DbIo(String::new()), true),
            (Error::SecField(String::new()), true),
        ];
        for (err, expected) in cases {
            // Evaluate `recoverable()` first so this test fails on the function it
            // proves; `code()` is only touched to label a mismatch (GREEN path).
            assert_eq!(
                err.recoverable(),
                expected,
                "recoverable mismatch for {}",
                err.code(),
            );
        }
    }

    /// EC / P-3: an `sk-ant-` token is removed and replaced with the marker.
    #[test]
    fn redaction_strips_sk_ant_prefix() {
        let out = redact("key sk-ant-abc123XYZ done");
        assert!(
            !out.contains("sk-ant-abc123XYZ"),
            "the sk-ant secret must not survive redaction: {out}"
        );
        assert!(
            out.contains("[REDACTED]"),
            "redaction marker missing: {out}"
        );
        assert_eq!(out, "key [REDACTED] done");
    }

    /// EC / P-3: a `Token <value>` header keeps the word `Token` and masks the value.
    #[test]
    fn redaction_strips_token_bearer() {
        let out = redact("Authorization: Token dg_secret_key9");
        assert!(
            !out.contains("dg_secret_key9"),
            "the Deepgram token value must not survive redaction: {out}"
        );
        assert!(
            out.contains("[REDACTED]"),
            "redaction marker missing: {out}"
        );
        assert_eq!(out, "Authorization: Token [REDACTED]");
    }

    /// EC / P-3: text with no secret pattern is returned byte-identical.
    #[test]
    fn redaction_leaves_clean_text_unchanged() {
        let clean = "this is a perfectly clean log line 42";
        assert_eq!(redact(clean), clean);
    }

    /// P-3 / §14 rule ("codes never carry secrets"): an `Error` constructed with a
    /// secret in its message yields a redacted `ApiError.message`.
    #[test]
    fn p3_error_messages_never_contain_secrets() {
        let err = Error::LlmAuth("anthropic rejected key sk-ant-LEAK999 (401)".to_string());
        let api = ApiError::from(&err);
        assert_eq!(api.code, "LLM-AUTH");
        assert!(
            !api.message.contains("sk-ant-LEAK999"),
            "ApiError.message leaked a secret: {}",
            api.message
        );
        assert!(
            api.message.contains("[REDACTED]"),
            "ApiError.message missing redaction marker: {}",
            api.message
        );
    }

    /// P-3: REAL tracing integration. A `RedactingWriter` wraps a capture buffer;
    /// after emitting a `warn!` carrying two secrets, the captured bytes contain
    /// neither secret but do contain the redaction marker. Uses a scoped
    /// `set_default` guard (parallel-test safe), never a global init.
    #[test]
    fn tracing_output_never_contains_secret() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;

        // Capture sink — pure test infrastructure (real logic is fine here; the
        // *redaction* under test lives in production `RedactingWriter`/`redact`).
        #[derive(Clone)]
        struct CaptureBuf(Arc<Mutex<Vec<u8>>>);
        struct CaptureGuard(Arc<Mutex<Vec<u8>>>);

        impl Write for CaptureGuard {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for CaptureBuf {
            type Writer = CaptureGuard;
            fn make_writer(&'a self) -> Self::Writer {
                CaptureGuard(self.0.clone())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let capture = CaptureBuf(buf.clone());

        let subscriber = tracing_subscriber::fmt()
            .with_writer(RedactingWriter::new(capture))
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .without_time()
            .finish();

        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::warn!("leaking sk-ant-SECRET123 and Token hunter2 right now");
        drop(_guard);

        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            !captured.contains("sk-ant-SECRET123"),
            "tracing output leaked the sk-ant secret: {captured}"
        );
        assert!(
            !captured.contains("hunter2"),
            "tracing output leaked the Token value: {captured}"
        );
        assert!(
            captured.contains("[REDACTED]"),
            "tracing output missing redaction marker: {captured}"
        );
    }
}
