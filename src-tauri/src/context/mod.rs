//! Frontmost-app context snapshot and FR-2.3 style resolution.
//!
//! The operating-system boundary is deliberately a small synchronous trait so
//! session code can take one deterministic snapshot at hotkey-down (SM-5), and
//! tests never need an active desktop window.

#[cfg(target_os = "macos")]
pub mod macos;
mod rules;

pub use rules::{AppStyle, ContextResolver, ResolvedContext, StyleId};

use crate::{error::Error, inject::InjectContext, pipeline::AppContext};

/// This application's bundle identifier (`tauri.conf.json`). A frontmost match
/// means the user is looking at one of WhisperSpree's own windows, which FR-1.4
/// treats as focus loss (never synthesize keys into ourselves).
pub const OWN_BUNDLE_ID: &str = "com.whisperspree.app";

/// Pure FR-1.4 mapping from a frontmost snapshot to the injection policy
/// context. Total function: missing AX trust disables typing only, and a
/// missing/opaque target invalidates it instead of failing resolution.
pub fn inject_context(
    frontmost: Option<&AppContext>,
    accessibility_trusted: bool,
) -> InjectContext {
    let Some(target) = frontmost else {
        return InjectContext {
            secure_input: false,
            target_is_valid: false,
            typing_available: accessibility_trusted,
        };
    };
    InjectContext {
        secure_input: target.secure_input,
        target_is_valid: !matches!(target.bundle_id.as_str(), "" | "unknown" | OWN_BUNDLE_ID),
        typing_available: accessibility_trusted,
    }
}

/// Source of a frontmost application/window snapshot.
///
/// The macOS backend belongs behind this trait; fixtures can provide an exact
/// snapshot without querying the host OS.
pub trait FrontmostSnapshot {
    fn frontmost(&mut self) -> Result<AppContext, Error>;
}

/// Normative §9.3 context-provider boundary consumed by runtime composition.
pub trait ContextProvider: Send {
    fn frontmost(&mut self) -> Result<AppContext, Error>;
}

impl<T: FrontmostSnapshot + Send> ContextProvider for T {
    fn frontmost(&mut self) -> Result<AppContext, Error> {
        FrontmostSnapshot::frontmost(self)
    }
}
