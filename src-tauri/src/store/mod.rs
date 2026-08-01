//! Store module root (§11): declares the `settings` + `keychain` submodules
//! (T0.3) and the T0.4 SQLite stores (`db`, `history`,
//! `dictionary_entries`, `snippets`, `custom_prompts`, `app_rules`), plus the
//! shared app-data-dir helper + local-persistence error mapping used by all of
//! them.
//!
//! PRD refs:
//! - §4.4 — app data root `~/Library/Application Support/WhisperSpree/`.
//! - §14 / OPEN_QUESTIONS Q8 — settings.json, keychain, and (per Q8's "local
//!   persistence" broadening) SQLite I/O failures all map onto the existing
//!   `DB-IO` code; the §14 matrix is closed (14 codes), so no 15th code is
//!   introduced.
//! - §11 — `store/ db.rs settings.rs keychain.rs history.rs`; the four
//!   lookup-table stores (`dictionary_entries.rs`, `snippets.rs`,
//!   `custom_prompts.rs`, `app_rules.rs`) are OPEN_QUESTIONS Q9 (§11 doesn't
//!   enumerate them; one-file-per-table mirrors the `settings.rs`/`keychain.rs`
//!   precedent).

pub mod app_rules;
pub mod custom_prompts;
pub mod db;
pub mod dictionary_entries;
pub mod history;
pub mod keychain;
pub mod settings;
pub mod snippets;

/// The WhisperSpree app-data root: `dirs::data_dir()/WhisperSpree` (PRD §4.4; on
/// macOS this resolves under `~/Library/Application Support/`).
///
/// Pinned by `tests::app_data_dir_points_at_application_support`.
pub fn app_data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("WhisperSpree")
}

/// Map any local-persistence I/O failure (settings.json read/write, keychain
/// set/get/delete) onto the single closed-matrix `DB-IO` code.
///
// PRD-QUESTION(Q8): §14 has no dedicated code for settings-file or keychain
// I/O; `DB-IO` ("sqlite failure") is broadened to "local persistence failure"
// per the pinned OPEN_QUESTIONS Q8 resolution. Do not invent a 15th code.
pub(crate) fn db_io<E: std::fmt::Display>(context: &str, err: E) -> crate::error::Error {
    crate::error::Error::DbIo(format!("{context}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC: PRD §4.4 — `app_data_dir()` resolves under the macOS Application
    /// Support root and is namespaced to `WhisperSpree`.
    #[test]
    fn app_data_dir_points_at_application_support() {
        let dir = app_data_dir();
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("WhisperSpree"),
            "app_data_dir must be namespaced to WhisperSpree: {dir:?}"
        );
        #[cfg(target_os = "macos")]
        assert!(
            dir.to_string_lossy()
                .contains("Library/Application Support"),
            "on macOS the app data dir must live under Library/Application Support: {dir:?}"
        );
    }
}
