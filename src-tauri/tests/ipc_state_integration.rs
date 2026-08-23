//! T0.5 — real managed IPC state and completed T0.3 adapter integration tests.
//!
//! Every settings assertion uses a real `SettingsStore` in a `tempfile`
//! directory. Keychain interaction uses only the Q4-sanctioned in-memory and
//! failing doubles; no test accesses an OS keychain, network, or permission
//! surface.

mod common;

use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
    thread,
};

use serde_json::{json, Value};
use whisperspree_lib::{
    ipc::commands,
    store::keychain::{KeyStore, Provider},
    testutil::mocks::{FailingKeyStore, InMemoryKeyStore},
};

/// Test-only keychain double. It stays in this integration test because thread
/// observation is a Tauri scheduling assertion, not a reusable app boundary.
#[derive(Default)]
struct ThreadRecordingKeyStore {
    secrets: Mutex<HashMap<String, String>>,
    call_threads: Mutex<Vec<thread::ThreadId>>,
}

impl ThreadRecordingKeyStore {
    fn call_threads(&self) -> Vec<thread::ThreadId> {
        self.call_threads
            .lock()
            .expect("ThreadRecordingKeyStore mutex poisoned")
            .clone()
    }

    fn record(&self) {
        self.call_threads
            .lock()
            .expect("ThreadRecordingKeyStore mutex poisoned")
            .push(thread::current().id());
    }
}

impl KeyStore for ThreadRecordingKeyStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), whisperspree_lib::error::Error> {
        self.record();
        self.secrets
            .lock()
            .expect("ThreadRecordingKeyStore mutex poisoned")
            .insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn get(&self, account: &str) -> Result<Option<String>, whisperspree_lib::error::Error> {
        self.record();
        Ok(self
            .secrets
            .lock()
            .expect("ThreadRecordingKeyStore mutex poisoned")
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), whisperspree_lib::error::Error> {
        self.record();
        self.secrets
            .lock()
            .expect("ThreadRecordingKeyStore mutex poisoned")
            .remove(account);
        Ok(())
    }
}

fn assert_send_sync<T: Send + Sync + 'static>() {}

#[test]
fn fr_0_5_ipc_state_is_send_sync() {
    assert_send_sync::<commands::IpcState>();
}

#[test]
fn fr_0_5_tauri_state_dispatch_get_and_update_settings_round_trips_real_store() {
    let temp = tempfile::tempdir().expect("real tempdir for settings store");
    let keys: Arc<dyn KeyStore> = Arc::new(InMemoryKeyStore::new());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), keys);

    let initial = ipc
        .invoke(commands::COMMAND_GET_SETTINGS, json!({}))
        .expect("get_settings must be a completed T0.3 adapter, not TODO");
    assert_eq!(initial["postprocess"]["timeoutMs"], 6000);

    let updated = ipc
        .invoke(
            commands::COMMAND_UPDATE_SETTINGS,
            json!({ "patch": { "postprocess": { "timeoutMs": 3210 } } }),
        )
        .expect("update_settings must dispatch the existing deep-merge store adapter");
    assert_eq!(updated["postprocess"]["timeoutMs"], 3210);
    assert_eq!(
        updated["postprocess"]["personaId"], "clean",
        "partial update must preserve untouched nested defaults"
    );

    let reread = ipc
        .invoke(commands::COMMAND_GET_SETTINGS, json!({}))
        .expect("a later get_settings dispatch must read the real persisted file");
    assert_eq!(reread, updated, "IPC update must persist before returning");

    let persisted: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("settings.json"))
            .expect("completed update_settings must write settings.json"),
    )
    .expect("settings.json must remain valid JSON");
    assert_eq!(persisted["postprocess"]["timeoutMs"], 3210);
}

#[test]
fn fr_0_5_tauri_update_settings_preserves_safe_unknown_fields_and_prior_nested_values() {
    let temp = tempfile::tempdir().expect("real tempdir for settings store");
    let keys: Arc<dyn KeyStore> = Arc::new(InMemoryKeyStore::new());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), keys);

    ipc.invoke(
        commands::COMMAND_UPDATE_SETTINGS,
        json!({
            "patch": {
                "futureRoot": { "keep": true },
                "postprocess": { "personaId": "professional" }
            }
        }),
    )
    .expect("first deep update must dispatch");
    ipc.invoke(
        commands::COMMAND_UPDATE_SETTINGS,
        json!({ "patch": { "postprocess": { "timeoutMs": 4321 } } }),
    )
    .expect("second deep update must dispatch");

    let persisted: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("settings.json"))
            .expect("real settings file must exist after update"),
    )
    .expect("persisted settings remain JSON");
    assert_eq!(persisted["futureRoot"], json!({ "keep": true }));
    assert_eq!(persisted["postprocess"]["personaId"], "professional");
    assert_eq!(persisted["postprocess"]["timeoutMs"], 4321);

    let invalid = ipc
        .invoke(
            commands::COMMAND_UPDATE_SETTINGS,
            json!({ "patch": { "postprocess": { "timeoutMs": "not-a-number" } } }),
        )
        .expect_err("type-invalid patch must not be persisted through the IPC boundary");
    assert_eq!(invalid["code"], "DB-IO");

    let after_invalid: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("settings.json"))
            .expect("type-invalid patch must leave the prior file in place"),
    )
    .expect("the prior settings file must remain valid JSON");
    assert_eq!(
        after_invalid, persisted,
        "invalid IPC patch must not corrupt the real store"
    );
}

#[test]
fn p3_tauri_update_settings_never_persists_recursive_secret_patch() {
    let temp = tempfile::tempdir().expect("real tempdir for settings persistence boundary");
    let keys: Arc<dyn KeyStore> = Arc::new(InMemoryKeyStore::new());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), keys);

    ipc.invoke(
        commands::COMMAND_UPDATE_SETTINGS,
        json!({
            "patch": {
                "safeFuture": { "preserve": true },
                "benignNote": "sk-ant-ipc-settings-private-value",
                "tokenizerModel": "safe-ipc-tokenizer-model",
                "secretaryMode": "safe-ipc-secretary-mode",
                "passwordlessMode": true,
                "toKen_value": "safe-ipc-token-lookalike",
                "nested": [{
                    "safeArraySibling": "retain me",
                    "authorizationHeader": "Token ipc-settings-private-value"
                }],
                "credentials": "credentials-ipc-private-value"
            }
        }),
    )
    .expect("secret-bearing IPC patch must be sanitized rather than rejected or persisted");

    let persisted = fs::read_to_string(temp.path().join("settings.json"))
        .expect("real IPC update must write settings.json");
    for forbidden in [
        "sk-ant-ipc-settings-private-value",
        "ipc-settings-private-value",
        "credentials-ipc-private-value",
        "[REDACTED]",
    ] {
        assert!(
            !persisted.contains(forbidden),
            "P-3 IPC settings persistence must not contain secret material or redaction markers"
        );
    }
    let value: Value = serde_json::from_str(&persisted).expect("settings.json stays valid JSON");
    assert_eq!(value["safeFuture"], json!({ "preserve": true }));
    assert_eq!(value["nested"][0]["safeArraySibling"], "retain me");
    assert_eq!(value["tokenizerModel"], "safe-ipc-tokenizer-model");
    assert_eq!(value["secretaryMode"], "safe-ipc-secretary-mode");
    assert_eq!(value["passwordlessMode"], true);
    assert_eq!(value["toKen_value"], "safe-ipc-token-lookalike");
}

#[test]
fn fr_0_5_tauri_state_dispatch_key_commands_round_trip_injected_store() {
    let temp = tempfile::tempdir().expect("real tempdir for no-secret persistence check");
    let keys = Arc::new(InMemoryKeyStore::new());
    let key_store: Arc<dyn KeyStore> = keys.clone();
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), key_store);

    ipc.invoke(
        commands::COMMAND_SET_API_KEY,
        json!({ "provider": "anthropic", "value": "anthropic-secret" }),
    )
    .expect("set_api_key must use the injected store");
    ipc.invoke(
        commands::COMMAND_SET_API_KEY,
        json!({ "provider": "deepgram", "value": "deepgram-secret" }),
    )
    .expect("both closed providers must be supported");

    for provider in ["anthropic", "deepgram"] {
        assert_eq!(
            ipc.invoke(
                commands::COMMAND_HAS_API_KEY,
                json!({ "provider": provider }),
            )
            .expect("has_api_key must dispatch"),
            json!(true)
        );
    }

    ipc.invoke(
        commands::COMMAND_DELETE_API_KEY,
        json!({ "provider": "anthropic" }),
    )
    .expect("delete_api_key must dispatch");
    ipc.invoke(
        commands::COMMAND_DELETE_API_KEY,
        json!({ "provider": "anthropic" }),
    )
    .expect("deleting an absent key must remain idempotently successful");
    assert_eq!(
        ipc.invoke(
            commands::COMMAND_HAS_API_KEY,
            json!({ "provider": "anthropic" }),
        )
        .expect("has_api_key must dispatch after delete"),
        json!(false)
    );
    assert!(keys
        .get(Provider::Anthropic.account_name())
        .expect("in-memory key store reads")
        .is_none());
    assert!(
        !temp.path().join("settings.json").exists(),
        "key adapters must never persist secrets to settings.json"
    );
}

#[test]
fn ec_0_5_tauri_rejects_unknown_provider_before_keystore_access() {
    let temp = tempfile::tempdir().expect("real tempdir");
    let failure: Arc<dyn KeyStore> = Arc::new(FailingKeyStore::always_fails());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), failure);
    let error = ipc
        .invoke(
            commands::COMMAND_SET_API_KEY,
            json!({ "provider": "not-a-prd-provider", "value": "untrusted-input" }),
        )
        .expect_err("closed Provider deserialization must reject unknown wire values");
    assert_ne!(
        error.get("code"),
        Some(&json!("DB-IO")),
        "a failing key store would yield DB-IO, so that code would prove unknown provider reached it"
    );
}

#[test]
fn p3_tauri_unknown_provider_does_not_echo_or_touch_keystore() {
    let temp = tempfile::tempdir().expect("real tempdir");
    let key_store = Arc::new(ThreadRecordingKeyStore::default());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), key_store.clone());
    let expected = json!("invalid args `provider` for command `set_api_key`: unsupported provider");
    for untrusted_provider in [
        json!("other-provider"),
        json!(""),
        json!("ANTHROPIC"),
        json!("sk-ant-provider-input Token provider-private-value"),
        json!({ "credential": "sk-ant-object-private-value" }),
        json!(42),
        json!(null),
        json!(true),
        json!(["sk-ant-array-private-value"]),
    ] {
        let error = ipc
            .invoke(
                commands::COMMAND_SET_API_KEY,
                json!({ "provider": untrusted_provider, "value": "irrelevant" }),
            )
            .expect_err("invalid Provider JSON must be rejected at argument deserialization");
        assert!(
            error == expected,
            "the public invalid-provider response must be stable and independent of input shape"
        );
        let wire = error.to_string();
        for leaked in [
            "sk-ant-provider-input",
            "provider-private-value",
            "sk-ant-object-private-value",
            "sk-ant-array-private-value",
            "Token ",
        ] {
            assert!(
                !wire.contains(leaked),
                "untrusted provider text must never echo through InvokeError"
            );
        }
    }
    assert!(
        key_store.call_threads().is_empty(),
        "provider deserialization must reject before the injected key store is called"
    );
}

#[test]
fn fr_0_5_all_five_adapters_delegate_through_spawn_blocking() {
    let commands_source = fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ipc/commands.rs"),
    )
    .expect("T0.5 command module source must exist");
    let production_source = commands_source
        .split("\n#[cfg(test)]")
        .next()
        .expect("the production command module must precede its unit-test module");
    assert_eq!(
        production_source
            .matches("tauri::async_runtime::spawn_blocking(")
            .count(),
        1,
        "the common runner must be the sole direct Tauri spawn_blocking call site"
    );
    let runner_start = production_source
        .find("async fn run_blocking")
        .expect("the completed adapters must share a private async blocking runner");
    let runner_tail = &production_source[runner_start..];
    let runner_end = runner_tail
        .find("\n#[tauri::command]")
        .map(|first_command| runner_start + first_command)
        .expect("the private runner must end before the first public Tauri command");
    let runner_body = &production_source[runner_start..runner_end];
    assert_eq!(
        runner_body
            .matches("tauri::async_runtime::spawn_blocking(")
            .count(),
        1,
        "the sole direct Tauri spawn_blocking call must live inside run_blocking"
    );
    for (function, operation) in [
        ("get_settings", "state.get_settings()"),
        ("update_settings", "state.update_settings(patch)"),
        ("set_api_key", "state.set_api_key(provider,value)"),
        ("has_api_key", "state.has_api_key(provider)"),
        ("delete_api_key", "state.delete_api_key(provider)"),
    ] {
        let async_command = format!("#[tauri::command]\npub async fn {function}");
        assert!(
            production_source.contains(&async_command),
            "blocking {function} must expose an ordinary Tauri async command wrapper"
        );
        let wrapper_start = production_source
            .find(&format!("pub async fn {function}"))
            .expect("the async wrapper was found above");
        let wrapper_tail = &production_source[wrapper_start..];
        let wrapper_end = wrapper_tail
            .find("\n#[tauri::command]")
            .map(|next_command| wrapper_start + next_command)
            .unwrap_or(production_source.len());
        let command_body = &production_source[wrapper_start..wrapper_end];
        let compact = command_body
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            compact.matches("run_blocking(").count() == 1,
            "each adapter wrapper must have exactly one executable common-runner call"
        );
        assert_eq!(
            command_body
                .matches("tauri::async_runtime::spawn_blocking(")
                .count(),
            0,
            "{function} must not bypass the common blocking runner"
        );
        assert!(
            compact.contains("letstate=state.inner().clone();"),
            "{function} must clone its managed IpcState before entering the blocking closure"
        );
        let expected_closure = format!("run_blocking(move||{operation})");
        assert!(
            compact.contains(&expected_closure),
            "each adapter must run its complete synchronous operation inside the common runner closure"
        );
        assert_eq!(
            compact.matches(operation).count(),
            1,
            "{function} must perform its synchronous state operation exactly once, inside the runner"
        );
    }

    let temp = tempfile::tempdir().expect("real tempdir");
    let key_store = Arc::new(ThreadRecordingKeyStore::default());
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), key_store.clone());
    let invoker_thread = thread::current().id();
    ipc.invoke(commands::COMMAND_GET_SETTINGS, json!({}))
        .expect("get_settings must use the shared configured MockRuntime harness");
    ipc.invoke(
        commands::COMMAND_UPDATE_SETTINGS,
        json!({ "patch": { "unrelatedFuture": { "thread": "settings" } } }),
    )
    .expect("update_settings must use the shared configured MockRuntime harness");
    for (command, body) in [
        (
            commands::COMMAND_SET_API_KEY,
            json!({ "provider": "anthropic", "value": "thread-test" }),
        ),
        (
            commands::COMMAND_HAS_API_KEY,
            json!({ "provider": "anthropic" }),
        ),
        (
            commands::COMMAND_DELETE_API_KEY,
            json!({ "provider": "anthropic" }),
        ),
    ] {
        ipc.invoke(command, body)
            .expect("completed key adapter must dispatch through MockRuntime");
    }

    let threads = key_store.call_threads();
    assert_eq!(
        threads.len(),
        3,
        "set/get/delete must each call the key store once"
    );
    assert!(
        threads.iter().all(|thread_id| *thread_id != invoker_thread),
        "blocking keychain work must not execute on the MockRuntime invoke thread"
    );
}

#[test]
fn p3_tauri_key_command_failure_returns_redacted_db_io_and_never_persists_secret() {
    let temp = tempfile::tempdir().expect("real tempdir");
    let secret = "sk-ant-ipc-test Token private-deepgram-value";
    let failing: Arc<dyn KeyStore> = Arc::new(FailingKeyStore::failing_with_message(secret));
    let ipc = common::IpcHarness::new_at(temp.path().to_path_buf(), failing);
    let error = ipc
        .invoke(
            commands::COMMAND_SET_API_KEY,
            json!({ "provider": "anthropic", "value": secret }),
        )
        .expect_err("injected key-store failure must cross the real Tauri IPC boundary");
    assert_eq!(error["code"], "DB-IO");
    let message = error["message"]
        .as_str()
        .expect("ApiError message must serialize as a string");
    assert!(message.contains("[REDACTED]"));
    assert!(!message.contains("sk-ant-ipc-test"));
    assert!(!message.contains("private-deepgram-value"));
    assert!(
        !temp.path().join("settings.json").exists(),
        "failed key writes must not create a settings file containing the key"
    );
}
