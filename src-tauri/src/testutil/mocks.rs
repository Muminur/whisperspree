//! Test doubles for OS-permission-bound surfaces (CLAUDE.md §3 / OPEN_QUESTIONS
//! Q4). The keychain (`store::keychain::KeyStore`) is the only trait T0.3
//! introduces; doubles for the other §9.3 traits (mic, key capture, injection,
//! AX/`ContextProvider`) land with the tasks that introduce those traits.
//!
//! These doubles carry real (not `todo!()`) logic deliberately: they are the
//! CLAUDE.md-sanctioned boundary substitute for an OS-permission surface, not
//! production business logic under test.

use crate::error::Error;
use crate::store::keychain::KeyStore;
use std::collections::HashMap;
use std::sync::Mutex;

/// An in-memory [`KeyStore`] double — real `HashMap` logic, no OS keychain
/// access, so it is deterministic and safe in CI (OPEN_QUESTIONS Q4).
#[derive(Default)]
pub struct InMemoryKeyStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl InMemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for InMemoryKeyStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), Error> {
        self.secrets
            .lock()
            .expect("InMemoryKeyStore mutex poisoned")
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, account: &str) -> Result<Option<String>, Error> {
        Ok(self
            .secrets
            .lock()
            .expect("InMemoryKeyStore mutex poisoned")
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), Error> {
        self.secrets
            .lock()
            .expect("InMemoryKeyStore mutex poisoned")
            .remove(account);
        Ok(())
    }
}

/// A [`KeyStore`] double that deterministically fails every call with
/// `Error::DbIo` (OPEN_QUESTIONS Q8-b), for exercising the keychain failure
/// path without touching the real OS keychain.
pub struct FailingKeyStore {
    message: String,
}

impl FailingKeyStore {
    /// Fails with a generic, non-secret-bearing message.
    pub fn always_fails() -> Self {
        Self {
            message: "simulated keychain failure".to_string(),
        }
    }

    /// Fails with a caller-supplied raw message — used to prove that whatever
    /// the underlying keychain backend echoes back gets redacted (P-3) once it
    /// reaches `ApiError`.
    pub fn failing_with_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl KeyStore for FailingKeyStore {
    fn set(&self, _account: &str, _secret: &str) -> Result<(), Error> {
        Err(Error::DbIo(self.message.clone()))
    }

    fn get(&self, _account: &str) -> Result<Option<String>, Error> {
        Err(Error::DbIo(self.message.clone()))
    }

    fn delete(&self, _account: &str) -> Result<(), Error> {
        Err(Error::DbIo(self.message.clone()))
    }
}
