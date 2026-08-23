//! macOS clipboard and typing adapters for the policy in [`super`].
//!
//! These are deliberately thin OS-permission boundaries. They are never used
//! by headless tests; deterministic doubles exercise the strategy matrix.
//! The clipboard adapter speaks to NSPasteboard directly: FR-1.4 needs the
//! monotonic `changeCount`, the `public.rtf` flavor, and a true
//! `clearContents`, none of which arboard exposes.

use super::{
    pasteboard_carries_restorable_text, restore_action, Clipboard, ClipboardContents,
    ClipboardSnapshot, PasteVerifier, Typist, PASTEBOARD_TYPE_RTF, PASTEBOARD_TYPE_UTF8_PLAIN_TEXT,
};
use crate::error::Error;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

// Slice C (T2.2) re-enables these on the typing adapter below.
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// All types currently declared on the general pasteboard.
fn pasteboard_types(pb: &NSPasteboard) -> Vec<String> {
    pb.types()
        .map(|types| types.to_vec().iter().map(|t| t.to_string()).collect())
        .unwrap_or_default()
}

#[derive(Clone, Copy, Default)]
pub struct MacClipboard;

impl Clipboard for MacClipboard {
    fn snapshot(&self) -> Result<ClipboardSnapshot, Error> {
        let pb = NSPasteboard::generalPasteboard();
        // Strictly monotonic across every pasteboard ownership change, so a
        // user copying identical content during the restore window still wins.
        let revision = u64::try_from(pb.changeCount()).unwrap_or(u64::MAX);
        let types = pasteboard_types(&pb);
        let restorable = pasteboard_carries_restorable_text(
            &types.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let plain_type = &*NSString::from_str(PASTEBOARD_TYPE_UTF8_PLAIN_TEXT);
        let contents = if restorable {
            match pb.stringForType(plain_type) {
                Some(plain) => ClipboardContents::Text {
                    plain: plain.to_string(),
                    rtf: pb
                        .dataForType(&NSString::from_str(PASTEBOARD_TYPE_RTF))
                        .map(|data| String::from_utf8_lossy(&data.to_vec()).into_owned()),
                },
                // The flavor was declared but cannot be read back; treat the
                // payload as opaque rather than risking an overwrite.
                None => {
                    tracing::warn!(code = "INJ-FAIL", "utf-8 flavor declared but unreadable");
                    ClipboardContents::NonText
                }
            }
        } else if types.is_empty() {
            ClipboardContents::Empty
        } else {
            ClipboardContents::NonText
        };
        Ok(ClipboardSnapshot { contents, revision })
    }

    fn set_text(&self, text: &str) -> Result<(), Error> {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let plain_type = &*NSString::from_str(PASTEBOARD_TYPE_UTF8_PLAIN_TEXT);
        if pb.setString_forType(&NSString::from_str(text), plain_type) {
            Ok(())
        } else {
            Err(Error::InjFail("clipboard write failed".into()))
        }
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), Error> {
        let pb = NSPasteboard::generalPasteboard();
        match restore_action(snapshot)? {
            super::RestoreAction::Clear => {
                pb.clearContents();
                Ok(())
            }
            super::RestoreAction::SetText { plain, rtf } => {
                pb.clearContents();
                let plain_type = &*NSString::from_str(PASTEBOARD_TYPE_UTF8_PLAIN_TEXT);
                if !pb.setString_forType(&NSString::from_str(&plain), plain_type) {
                    return Err(Error::InjFail("clipboard restore failed".into()));
                }
                if let Some(rtf) = rtf {
                    if !pb.setString_forType(
                        &NSString::from_str(&rtf),
                        &NSString::from_str(PASTEBOARD_TYPE_RTF),
                    ) {
                        tracing::warn!(code = "INJ-FAIL", type_ = %PASTEBOARD_TYPE_RTF, "rtf flavor restore failed; plain text restored");
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct MacTypist;

impl Typist for MacTypist {
    fn type_text(&self, text: &str, inter_key_delay_ms: u64) -> Result<(), Error> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|error| Error::InjFail(format!("keyboard unavailable: {error}")))?;
        // FR-1.4: per-character synthesis with a 3 ms (configurable) inter-key
        // delay; enigo applies it between synthesized key events.
        enigo.set_delay(u32::try_from(inter_key_delay_ms).unwrap_or(u32::MAX));
        enigo
            .text(text)
            .map_err(|error| Error::InjFail(format!("typing failed: {error}")))
    }

    fn paste(&self) -> Result<(), Error> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|error| Error::InjFail(format!("keyboard unavailable: {error}")))?;
        enigo
            .key(Key::Meta, Direction::Press)
            .map_err(|error| Error::InjFail(format!("paste shortcut failed: {error}")))?;
        // The modifier release is attempted even when the V click fails — a
        // stuck ⌘ would corrupt every subsequent keystroke in the target app.
        let click = enigo.key(Key::Unicode('v'), Direction::Click);
        let released = enigo.key(Key::Meta, Direction::Release);
        click
            .and(released)
            .map_err(|error| Error::InjFail(format!("paste shortcut failed: {error}")))
    }
}

#[derive(Clone, Copy, Default)]
pub struct MacPasteVerifier;

impl PasteVerifier for MacPasteVerifier {
    fn paste_succeeded(&self, _text: &str) -> Result<Option<bool>, Error> {
        // AX readback is a separate permission-bound adapter; until it is
        // available, the policy treats the paste as unverified and preserves
        // the clipboard-change guard.
        Ok(None)
    }
}
