//! Keychain-backed secret storage (§4.2 `keyring` 3, service `whisperspree`)
//! and the provider→account mapping used by the `set_api_key`/`has_api_key`/
//! `delete_api_key` command logic (§9.1; the thin `#[tauri::command]`
//! wrappers + registration land in T0.5).
//!
//! PRD refs:
//! - §4.2 — `keyring` 3, service `"whisperspree"`.
//! - §4.4 / §12 P-3 — API keys never reach settings.json/DB/logs; keychain
//!   only, accounts `anthropic_api_key` / `deepgram_api_key`.
//! - §14 / OPEN_QUESTIONS Q8 — a keychain I/O failure maps to `DB-IO`.
//! - OPEN_QUESTIONS Q4 — the keychain is an OS-permission surface; tests use
//!   the `KeyStore` trait with `testutil::mocks` doubles by default, the real
//!   `keyring` path is exercised only by the `#[ignore]` opt-in test below.

use crate::error::Error;

/// The two secret-bearing providers (§4.3 / §10). Maps to the exact PRD §12
/// P-3 keychain account literals via [`Provider::account_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    Deepgram,
}

impl Provider {
    /// The exact keychain account name for this provider (§12 P-3:
    /// `anthropic_api_key` / `deepgram_api_key`, service `"whisperspree"`).
    pub fn account_name(&self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic_api_key",
            Provider::Deepgram => "deepgram_api_key",
        }
    }
}

/// A secret store keyed by account name. `Send + Sync` so it can live behind
/// Tauri managed state.
// PRD-QUESTION(Q4): the keychain is an OS-permission surface CI cannot grant;
// the only sanctioned double lives in `testutil::mocks` (`InMemoryKeyStore`,
// `FailingKeyStore`). Production wiring uses the real [`KeyringStore`].
pub trait KeyStore: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<(), Error>;
    fn get(&self, account: &str) -> Result<Option<String>, Error>;
    fn delete(&self, account: &str) -> Result<(), Error>;
}

/// The real macOS Keychain-backed [`KeyStore`] (service `"whisperspree"`,
/// `keyring` 3 `apple-native` backend, PRD §4.2). Exercised in tests only by
/// the `#[ignore]` `keyring_real_roundtrip_ignored` opt-in test (Q4).
pub struct KeyringStore;

/// The keychain service name every account lives under (§4.2).
const SERVICE: &str = "whisperspree";

impl KeyStore for KeyringStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), Error> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|e| crate::store::db_io("opening keychain entry", e))?;
        entry
            .set_password(secret)
            .map_err(|e| crate::store::db_io("writing keychain entry", e))
    }

    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|e| crate::store::db_io("opening keychain entry", e))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(crate::store::db_io("reading keychain entry", e)),
        }
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        let entry = keyring::Entry::new(SERVICE, account)
            .map_err(|e| crate::store::db_io("opening keychain entry", e))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(crate::store::db_io("deleting keychain entry", e)),
        }
    }
}

/// §9.1 `set_api_key` command logic: resolve `provider` to its account name
/// and delegate to `ks`. The Tauri `#[tauri::command]` wrapper + registration
/// is T0.5.
pub fn set_api_key(ks: &dyn KeyStore, provider: Provider, secret: &str) -> Result<(), Error> {
    ks.set(provider.account_name(), secret)
}

/// §9.1 `has_api_key` command logic.
pub fn has_api_key(ks: &dyn KeyStore, provider: Provider) -> Result<bool, Error> {
    Ok(ks.get(provider.account_name())?.is_some())
}

/// §9.1 `delete_api_key` command logic.
pub fn delete_api_key(ks: &dyn KeyStore, provider: Provider) -> Result<(), Error> {
    ks.delete(provider.account_name())
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test keychain::` filters to this module).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mocks::{FailingKeyStore, InMemoryKeyStore};

    /// AC: §12 P-3 — provider → exact keychain account literal.
    #[test]
    fn provider_maps_to_prd_account_names() {
        assert_eq!(Provider::Anthropic.account_name(), "anthropic_api_key");
        assert_eq!(Provider::Deepgram.account_name(), "deepgram_api_key");
    }

    /// AC: §9.1 — `set_api_key` resolves the provider to its account name and
    /// writes the secret there, so a direct `KeyStore::get` under the exact
    /// account literal reads it back. Goes through the command-logic wrapper
    /// (not `ks.set` directly) so it proves `set_api_key`/`Provider::account_name`
    /// behaviour, not just the `InMemoryKeyStore` double.
    #[test]
    fn keystore_set_get_roundtrip() {
        let ks = InMemoryKeyStore::new();
        set_api_key(&ks, Provider::Anthropic, "sk-ant-test-only").unwrap();
        assert_eq!(
            ks.get(Provider::Anthropic.account_name()).unwrap(),
            Some("sk-ant-test-only".to_string())
        );
    }

    /// AC: §9.1 `has_api_key` — reflects presence/absence through the
    /// `Provider` wrapper.
    #[test]
    fn has_api_key_reflects_presence() {
        let ks = InMemoryKeyStore::new();
        assert!(!has_api_key(&ks, Provider::Deepgram).unwrap());
        set_api_key(&ks, Provider::Deepgram, "dg-secret").unwrap();
        assert!(has_api_key(&ks, Provider::Deepgram).unwrap());
    }

    /// AC: §9.1 `delete_api_key` — removes the secret so a subsequent
    /// `has_api_key` is `false`.
    #[test]
    fn delete_api_key_removes_secret() {
        let ks = InMemoryKeyStore::new();
        set_api_key(&ks, Provider::Anthropic, "sk-ant-remove-me").unwrap();
        assert!(has_api_key(&ks, Provider::Anthropic).unwrap());
        delete_api_key(&ks, Provider::Anthropic).unwrap();
        assert!(!has_api_key(&ks, Provider::Anthropic).unwrap());
    }

    /// AC: §14 `DB-IO` / OPEN_QUESTIONS Q8-b — a keychain failure maps to
    /// `DB-IO` through the `Provider` wrapper.
    #[test]
    fn keystore_failure_maps_db_io() {
        let ks = FailingKeyStore::always_fails();
        let err = set_api_key(&ks, Provider::Anthropic, "irrelevant").unwrap_err();
        assert_eq!(
            err.code(),
            "DB-IO",
            "keychain I/O failure must map to DB-IO (Q8)"
        );
    }

    /// AC: §12 P-3 / §14 "codes never carry secrets" — a keychain error whose
    /// raw message happens to echo a secret-looking token is redacted before
    /// it reaches `ApiError`. Routed through the `set_api_key` command-logic
    /// wrapper (not `ks.set` directly) so it proves the T0.3 error path, not
    /// just the already-implemented T0.2 `redact()`.
    #[test]
    fn keychain_error_message_redacted() {
        let ks = FailingKeyStore::failing_with_message(
            "keyring backend rejected sk-ant-LEAKED123 for account anthropic_api_key",
        );
        let err = set_api_key(&ks, Provider::Anthropic, "irrelevant").unwrap_err();
        assert_eq!(err.code(), "DB-IO");

        let api = crate::error::ApiError::from(&err);
        assert!(
            !api.message.contains("sk-ant-LEAKED123"),
            "ApiError.message leaked a secret: {}",
            api.message
        );
        assert!(
            api.message.contains("[REDACTED]"),
            "ApiError.message missing redaction marker: {}",
            api.message
        );
    }

    /// Real-interface opt-in (Q4): exercises the actual macOS Keychain via
    /// `keyring` 3. Never run in CI/headless; run manually with
    /// `cargo test --manifest-path src-tauri/Cargo.toml keyring_real_roundtrip_ignored -- --ignored`.
    #[test]
    #[ignore = "touches the real macOS Keychain (OPEN_QUESTIONS Q4); opt-in only"]
    fn keyring_real_roundtrip_ignored() {
        let ks = KeyringStore;
        let account = "whisperspree_test_probe_t0_3";
        ks.set(account, "probe-secret-value").expect("keyring set");
        assert_eq!(
            ks.get(account).expect("keyring get"),
            Some("probe-secret-value".to_string())
        );
        ks.delete(account).expect("keyring delete");
        assert_eq!(ks.get(account).expect("keyring get after delete"), None);
    }
}
