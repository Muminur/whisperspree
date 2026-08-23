//! T0.5 — real Tauri MockRuntime command-envelope regression tests.
//!
//! Tauri 2.11.5's command macro reads each Rust parameter from a *top-level*
//! JSON key named after that parameter. The frontend wrappers therefore send
//! `{ query: {...} }` and `{ input: {...} }`. This is an envelope regression:
//! it protects the raw Tauri extraction shape while the public TypeScript API
//! remains ergonomic.

mod common;

use serde_json::json;
use whisperspree_lib::{ipc::commands, testutil::mocks::InMemoryKeyStore};

#[test]
fn fr_0_5_tauri_dispatch_routes_nested_query_and_input_envelopes_to_todo_stubs() {
    let temp = tempfile::tempdir().expect("real tempdir for full IPC harness");
    let harness = common::IpcHarness::new_at(
        temp.path().to_path_buf(),
        std::sync::Arc::new(InMemoryKeyStore::new()),
    );
    let list_nested = harness
        .invoke(
            commands::COMMAND_LIST_DICTATIONS,
            json!({ "query": { "q": "status", "limit": 50, "beforeId": "older" } }),
        )
        .expect_err("nested list_dictations envelope must reach its TODO stub");
    assert_eq!(list_nested["code"], "TODO");

    let reprocess_nested = harness
        .invoke(
            commands::COMMAND_REPROCESS_DICTATION,
            json!({ "input": { "id": "dictation-1", "kind": "template", "refId": "email" } }),
        )
        .expect_err("nested reprocess_dictation envelope must reach its TODO stub");
    assert_eq!(reprocess_nested["code"], "TODO");
}
