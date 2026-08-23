//! Direct text injection strategy selection (T2.2 / FR-1.4).
//!
//! Platform adapters implement the small [`Clipboard`], [`Typist`], and
//! [`PasteVerifier`] boundaries.  Keeping the policy here makes its
//! privacy-sensitive decisions testable without reading or changing the host
//! clipboard in CI.

use std::{thread, time::Duration};

use crate::error::Error;

#[cfg(target_os = "macos")]
pub mod macos;

/// The strategy reported by `inject:done` (§9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectMethod {
    Paste,
    Type,
    ClipboardOnly,
}

/// Clipboard data whose preservation is relevant to FR-1.4.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClipboardContents {
    Empty,
    Text {
        plain: String,
        rtf: Option<String>,
    },
    /// Images, files, and any other representation we must not overwrite.
    NonText,
}

impl ClipboardContents {
    fn can_be_restored_after_paste(&self) -> bool {
        matches!(self, Self::Empty | Self::Text { .. })
    }
}

/// A clipboard capture plus an opaque monotonically-changing revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub contents: ClipboardContents,
    pub revision: u64,
}

/// NSPasteboard UTI for the plain-UTF-8 text flavor (`NSPasteboardTypeString`).
pub const PASTEBOARD_TYPE_UTF8_PLAIN_TEXT: &str = "public.utf8-plain-text";
/// NSPasteboard UTI for the rich-text flavor captured alongside plain text.
pub const PASTEBOARD_TYPE_RTF: &str = "public.rtf";

/// Legacy flavor still written by some Carbon-era applications; its payload is
/// plain UTF-8 text for our purposes (FR-1.4 "save public.utf8-plain-text").
const LEGACY_PLAIN_TEXT_TYPE: &str = "NSStringPboardType";

/// True when the pasteboard payload is entirely plain-UTF-8 text (optionally
/// with its RTF companion) and can be snapshotted and restored verbatim
/// (FR-1.4). Any other declared flavor — image, file URL, HTML — makes the
/// board opaque so the non-text guard can never destroy that data, even when
/// a text flavor is also present.
pub fn pasteboard_carries_restorable_text(types: &[&str]) -> bool {
    const KNOWN_TEXT: [&str; 3] = [
        PASTEBOARD_TYPE_UTF8_PLAIN_TEXT,
        PASTEBOARD_TYPE_RTF,
        LEGACY_PLAIN_TEXT_TYPE,
    ];
    let has_plain = types
        .iter()
        .any(|t| *t == PASTEBOARD_TYPE_UTF8_PLAIN_TEXT || *t == LEGACY_PLAIN_TEXT_TYPE);
    let all_flavors_known = types.iter().all(|t| KNOWN_TEXT.contains(t));
    has_plain && all_flavors_known
}

/// The concrete restore operation the native adapter performs. Public so the
/// macOS adapter stays a thin executor of this tested decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreAction {
    /// The clipboard was empty before injection: restore it with a true
    /// `clearContents`, never by writing an empty string.
    Clear,
    /// Replay both captured flavors when present (FR-1.4 preservation rule).
    SetText { plain: String, rtf: Option<String> },
}

/// Decides how a captured snapshot is restored. Opaque non-text data must
/// never be rewritten, mirroring [`ClipboardContents::can_be_restored_after_paste`].
pub fn restore_action(snapshot: &ClipboardSnapshot) -> Result<RestoreAction, Error> {
    match &snapshot.contents {
        ClipboardContents::Empty => Ok(RestoreAction::Clear),
        ClipboardContents::Text { plain, rtf } => Ok(RestoreAction::SetText {
            plain: plain.clone(),
            rtf: rtf.clone(),
        }),
        ClipboardContents::NonText => Err(Error::InjFail(
            "refusing to restore opaque non-text clipboard data".into(),
        )),
    }
}

/// Platform clipboard boundary. `revision` is the native change count where it
/// exists; an adapter may use a content hash where it does not (FR-1.4).
pub trait Clipboard: Clone + Send + Sync + 'static {
    fn snapshot(&self) -> Result<ClipboardSnapshot, Error>;
    fn set_text(&self, text: &str) -> Result<(), Error>;
    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), Error>;
}

/// Platform keyboard synthesis boundary.
pub trait Typist: Clone + Send + Sync + 'static {
    /// Emits Unicode text through a layout-independent text API.
    fn type_text(&self, text: &str, inter_key_delay_ms: u64) -> Result<(), Error>;
    fn paste(&self) -> Result<(), Error>;
}

/// Optional AX readback. `Ok(None)` means AX is unavailable, not a failure.
pub trait PasteVerifier: Clone + Send + Sync + 'static {
    fn paste_succeeded(&self, text: &str) -> Result<Option<bool>, Error>;
}

/// Normative §9.3 injection boundary used by the session task. The concrete
/// strategy service remains generic over clipboard, typing, and AX adapters.
pub trait Injector: Send + Sync {
    fn inject(&self, text: &str, ctx: &InjectContext) -> Result<InjectMethod, Error>;
}

/// Injection-time context. The secure-input checks and focus detection happen
/// in the macOS adapter before this policy is invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectContext {
    pub secure_input: bool,
    /// False for no frontmost application and WhisperSpree-owned windows.
    pub target_is_valid: bool,
    pub typing_available: bool,
}

impl Default for InjectContext {
    fn default() -> Self {
        Self {
            secure_input: false,
            target_is_valid: true,
            typing_available: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectionSettings {
    pub type_threshold_chars: usize,
    pub inter_key_delay_ms: u64,
    pub restore_clipboard_delay_ms: u64,
}

impl Default for InjectionSettings {
    fn default() -> Self {
        Self {
            type_threshold_chars: 200,
            inter_key_delay_ms: 3,
            restore_clipboard_delay_ms: 600,
        }
    }
}

/// A started injection whose paste restoration is awaiting its delay. This is
/// public solely so platform schedulers can hold it between the two phases.
pub enum PendingInjection {
    Complete(InjectMethod),
    Restore {
        snapshot: ClipboardSnapshot,
        injection_revision: u64,
    },
}

/// Ordered policy implementation for the FR-1.4 strategy chain.
#[derive(Clone)]
pub struct InjectionService<C, T, V> {
    clipboard: C,
    typist: T,
    verifier: V,
    settings: InjectionSettings,
}

impl<C: Clipboard, T: Typist, V: PasteVerifier> InjectionService<C, T, V> {
    pub fn new(clipboard: C, typist: T, verifier: V, settings: InjectionSettings) -> Self {
        Self {
            clipboard,
            typist,
            verifier,
            settings,
        }
    }

    /// Injects text and schedules clipboard restoration after a successful paste.
    /// Secure/invalid targets and all strategy failures retain the dictated text
    /// as clipboard-only output so the user can explicitly paste it.
    pub fn inject(&self, text: &str, ctx: &InjectContext) -> Result<InjectMethod, Error> {
        let pending = self.begin_inject(text, ctx)?;
        if let PendingInjection::Restore { .. } = pending {
            if self.settings.restore_clipboard_delay_ms == 0 {
                return self.finish_inject(pending);
            }
            let clipboard = self.clipboard.clone();
            let delay = self.settings.restore_clipboard_delay_ms;
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(delay));
                if let PendingInjection::Restore {
                    snapshot,
                    injection_revision,
                } = pending
                {
                    // A restore failure is non-fatal: text was already pasted and
                    // the user must never lose a newer clipboard value.
                    let unchanged = match clipboard.snapshot() {
                        Ok(current) => current.revision == injection_revision,
                        Err(error) => {
                            tracing::warn!(code = "INJ-FAIL", %error, "clipboard snapshot before restore failed");
                            false
                        }
                    };
                    if unchanged {
                        if let Err(error) = clipboard.restore(&snapshot) {
                            tracing::warn!(code = "INJ-FAIL", %error, "clipboard restore failed");
                        }
                    }
                }
            });
            Ok(InjectMethod::Paste)
        } else {
            self.finish_inject(pending)
        }
    }

    /// Starts injection without waiting for the restore delay. Public for the
    /// platform scheduler and deterministic tests; callers must finish it once.
    pub fn begin_inject(&self, text: &str, ctx: &InjectContext) -> Result<PendingInjection, Error> {
        if ctx.secure_input || !ctx.target_is_valid {
            self.clipboard.set_text(text)?;
            return Ok(PendingInjection::Complete(InjectMethod::ClipboardOnly));
        }

        let snapshot = self.clipboard.snapshot()?;
        // FR-1.4 prohibits paste when the clipboard contains an image, file, or
        // other non-text data. Clipboard-only would be equally destructive, so
        // the only safe outcome when typing cannot succeed is INJ-FAIL.
        if !snapshot.contents.can_be_restored_after_paste() {
            if !ctx.typing_available {
                return Err(Error::InjFail(
                    "cannot inject without overwriting non-text clipboard data".into(),
                ));
            }
            return self
                .typist
                .type_text(text, self.settings.inter_key_delay_ms)
                .map(|()| PendingInjection::Complete(InjectMethod::Type))
                .map_err(|_| {
                    Error::InjFail("typing failed; non-text clipboard was left unchanged".into())
                });
        }
        let prefer_type =
            text.chars().count() <= self.settings.type_threshold_chars || !ctx.typing_available;

        if prefer_type && ctx.typing_available {
            if self
                .typist
                .type_text(text, self.settings.inter_key_delay_ms)
                .is_ok()
            {
                return Ok(PendingInjection::Complete(InjectMethod::Type));
            }
            self.clipboard.set_text(text)?;
            return Ok(PendingInjection::Complete(InjectMethod::ClipboardOnly));
        }

        // A long string with unavailable typing attempts paste. If that fails,
        // clipboard-only is still the guaranteed non-destructive fallback.
        if self.clipboard.set_text(text).is_err() || self.typist.paste().is_err() {
            self.clipboard.set_text(text)?;
            return Ok(PendingInjection::Complete(InjectMethod::ClipboardOnly));
        }

        if matches!(self.verifier.paste_succeeded(text)?, Some(false)) {
            // Do not restore: clipboard-only fallback must leave dictated text
            // available for the user after a failed synthetic paste.
            return Ok(PendingInjection::Complete(InjectMethod::ClipboardOnly));
        }
        let injection_revision = self.clipboard.snapshot()?.revision;
        Ok(PendingInjection::Restore {
            snapshot,
            injection_revision,
        })
    }

    /// Completes a pending operation, restoring only when no user clipboard
    /// change occurred since setting the dictated text.
    pub fn finish_inject(&self, pending: PendingInjection) -> Result<InjectMethod, Error> {
        match pending {
            PendingInjection::Complete(method) => Ok(method),
            PendingInjection::Restore {
                snapshot,
                injection_revision,
            } => {
                let unchanged = match self.clipboard.snapshot() {
                    Ok(current) => current.revision == injection_revision,
                    Err(error) => {
                        tracing::warn!(code = "INJ-FAIL", %error, "clipboard snapshot before restore failed");
                        return Err(error);
                    }
                };
                if unchanged {
                    if let Err(error) = self.clipboard.restore(&snapshot) {
                        tracing::warn!(code = "INJ-FAIL", %error, "clipboard restore failed");
                        return Err(error);
                    }
                }
                Ok(InjectMethod::Paste)
            }
        }
    }
}

impl<C: Clipboard, T: Typist, V: PasteVerifier> Injector for InjectionService<C, T, V> {
    fn inject(&self, text: &str, ctx: &InjectContext) -> Result<InjectMethod, Error> {
        InjectionService::inject(self, text, ctx)
    }
}

/// §9.2 `inject:done {method}` spelling for the strategy result.
pub fn wire_method(method: InjectMethod) -> &'static str {
    match method {
        InjectMethod::Paste => "paste",
        InjectMethod::Type => "type",
        InjectMethod::ClipboardOnly => "clipboard_only",
    }
}
