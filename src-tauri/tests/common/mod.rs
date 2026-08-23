//! Shared real `MockRuntime` IPC harness for T0.5 integration tests.
//!
//! It always configures the entire production command registry through the
//! future `ipc::configure_ipc` helper; individual tests never install a subset
//! of handlers that could drift from production.

use std::sync::Arc;

use serde_json::Value;
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY},
    webview::InvokeRequest,
    WebviewWindowBuilder,
};
use whisperspree_lib::{
    ipc::commands,
    store::{keychain::KeyStore, settings::SettingsStore},
};

pub struct IpcHarness {
    _app: tauri::App<tauri::test::MockRuntime>,
    webview: tauri::WebviewWindow<tauri::test::MockRuntime>,
}

impl IpcHarness {
    #[allow(dead_code)]
    pub fn new_at(settings_dir: std::path::PathBuf, key_store: Arc<dyn KeyStore>) -> Self {
        Self::with_state(commands::IpcState::new(
            SettingsStore::new(settings_dir),
            key_store,
        ))
    }

    /// Full registry over a caller-built state so feature tasks can install
    /// deterministic seams before dispatching through real Tauri macros.
    pub fn with_state(state: commands::IpcState) -> Self {
        let app = whisperspree_lib::ipc::configure_ipc(mock_builder(), state)
            .build(mock_context(noop_assets()))
            .expect("MockRuntime app must use the canonical full IPC configuration");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("MockRuntime webview must build for IPC dispatch");
        Self { _app: app, webview }
    }

    pub fn invoke(&self, command: &str, body: Value) -> Result<Value, Value> {
        get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: command.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        )
        .map(|response| {
            response
                .deserialize::<Value>()
                .expect("IPC command result must be valid JSON")
        })
    }
}
