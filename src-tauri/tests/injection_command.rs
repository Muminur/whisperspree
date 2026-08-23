//! T2.2 — `test_injection` (AC-1.4(5)) over the real Tauri command macros.
//!
//! The managed injector and context provider are deterministic seams on
//! `IpcState`; dispatch itself goes through the production registry so the
//! envelope shape stays regression-protected.

mod common;

use std::sync::{Arc, Mutex};

use serde_json::json;
use whisperspree_lib::{
    error::Error,
    inject::{InjectContext, InjectMethod, Injector},
    ipc::commands,
    store::{keychain::KeyStore, settings::SettingsStore},
    testutil::mocks::InMemoryKeyStore,
};

#[derive(Default)]
struct RecordingInjector {
    result: Mutex<Option<Result<InjectMethod, Error>>>,
    last_context: Mutex<Option<InjectContext>>,
}

impl Injector for RecordingInjector {
    fn inject(&self, _text: &str, ctx: &InjectContext) -> Result<InjectMethod, Error> {
        *self.last_context.lock().unwrap() = Some(*ctx);
        match self.result.lock().unwrap().take() {
            Some(result) => result,
            None => Ok(InjectMethod::Type),
        }
    }
}

type ContextProvider = Arc<dyn Fn() -> Result<InjectContext, Error> + Send + Sync>;

struct NoopRuntime;

impl whisperspree_lib::pipeline::DictationRuntime for NoopRuntime {
    fn start_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
    fn stop_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
    fn cancel_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
}

fn provider(result: Result<InjectContext, Error>) -> ContextProvider {
    Arc::new(move || match &result {
        Ok(ctx) => Ok(*ctx),
        Err(error) => Err(error.clone()),
    })
}

fn normal_context() -> InjectContext {
    InjectContext::default()
}

fn harness(injector: Arc<RecordingInjector>, provider: ContextProvider) -> common::IpcHarness {
    let temp = tempfile::tempdir().expect("real tempdir for settings store");
    let state = commands::IpcState::with_runtime(
        SettingsStore::new(temp.path().to_path_buf()),
        Arc::new(InMemoryKeyStore::new()) as Arc<dyn KeyStore>,
        Arc::new(NoopRuntime),
    )
    .with_injector(injector)
    .with_injection_context_provider(provider);
    common::IpcHarness::with_state(state)
}

fn invoke_test_injection(h: &common::IpcHarness) -> Result<serde_json::Value, serde_json::Value> {
    h.invoke(
        commands::COMMAND_TEST_INJECTION,
        json!({ "sample": "injection smoke test" }),
    )
}

#[test]
fn fr_1_4_test_injection_reports_the_chosen_method_and_live_context() {
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Ok(InjectMethod::Type));
    let h = harness(injector.clone(), provider(Ok(normal_context())));

    let response = invoke_test_injection(&h).expect("test_injection must succeed");

    assert_eq!(response["method"], "type");
    let recorded = injector.last_context.lock().unwrap().expect("context seen");
    assert_eq!(recorded, normal_context());
}

#[test]
fn fr_1_4_secure_target_resolves_to_clipboard_only_method() {
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Ok(InjectMethod::ClipboardOnly));
    let secure = InjectContext {
        secure_input: true,
        ..normal_context()
    };
    let h = harness(injector.clone(), provider(Ok(secure)));

    let response = invoke_test_injection(&h).expect("secure path is not an error");

    assert_eq!(response["method"], "clipboard_only");
    assert!(
        injector
            .last_context
            .lock()
            .unwrap()
            .expect("context seen")
            .secure_input
    );
}

#[test]
fn ec_1_4_unavailable_frontmost_maps_ax_perm() {
    let h = harness(
        Arc::new(RecordingInjector::default()),
        provider(Err(Error::AxPerm("accessibility unavailable".into()))),
    );

    let error = invoke_test_injection(&h).expect_err("provider failure must surface");

    assert_eq!(error["code"], "AX-PERM");
}

#[test]
fn ec_1_4_all_strategies_failed_maps_inj_fail() {
    let injector = Arc::new(RecordingInjector::default());
    *injector.result.lock().unwrap() = Some(Err(Error::InjFail("synthesis refused".into())));
    let h = harness(injector, provider(Ok(normal_context())));

    let error = invoke_test_injection(&h).expect_err("injector failure must surface");

    assert_eq!(error["code"], "INJ-FAIL");
}
