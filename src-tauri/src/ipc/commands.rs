//! T0.5 — IPC command skeleton for §9.1.
//!
//! Every command is currently a minimal stub that returns `Err(ApiError {
//! code: "TODO" }` until later milestones implement them.
//! This file is intentionally dependency-light and keeps `serde_json::Value`
//! for many payloads to preserve shape flexibility while the upstream services are
//! added.

use crate::{
    asr::model_manager::{DownloadProgress, InstalledModel, ModelManager, ModelManagerError},
    audio::{self, AudioInputDevice},
    error::{ApiError, Error},
    hotkey::{HotkeyConfig, HotkeyControl, HotkeyController},
    inject::{InjectContext, InjectMethod, Injector},
    pipeline::DictationRuntime,
    state::{DownloadReservation, ModelDownloadService},
    store::{
        keychain::{self, KeyStore, Provider},
        settings::{Settings, SettingsStore},
    },
};
// Compatibility re-export for existing command callers while managed-state
// ownership lives in the normative `crate::state` module.
pub use crate::state::IpcState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub const COMMAND_GET_SETTINGS: &str = "get_settings";
pub const COMMAND_UPDATE_SETTINGS: &str = "update_settings";
pub const COMMAND_SET_API_KEY: &str = "set_api_key";
pub const COMMAND_HAS_API_KEY: &str = "has_api_key";
pub const COMMAND_DELETE_API_KEY: &str = "delete_api_key";
pub const COMMAND_START_DICTATION: &str = "start_dictation";
pub const COMMAND_STOP_DICTATION: &str = "stop_dictation";
pub const COMMAND_CANCEL_DICTATION: &str = "cancel_dictation";
pub const COMMAND_LIST_MODELS: &str = "list_models";
pub const COMMAND_DOWNLOAD_MODEL: &str = "download_model";
pub const COMMAND_CANCEL_DOWNLOAD: &str = "cancel_download";
pub const COMMAND_DELETE_MODEL: &str = "delete_model";
pub const COMMAND_LIST_DICTATIONS: &str = "list_dictations";
pub const COMMAND_GET_DICTATION: &str = "get_dictation";
pub const COMMAND_DELETE_DICTATION: &str = "delete_dictation";
pub const COMMAND_CLEAR_HISTORY: &str = "clear_history";
pub const COMMAND_REPROCESS_DICTATION: &str = "reprocess_dictation";
pub const COMMAND_GET_AUDIO_URL: &str = "get_audio_url";
pub const COMMAND_LIST_DICTIONARY_ENTRY: &str = "list_dictionary_entry";
pub const COMMAND_ADD_DICTIONARY_ENTRY: &str = "add_dictionary_entry";
pub const COMMAND_UPDATE_DICTIONARY_ENTRY: &str = "update_dictionary_entry";
pub const COMMAND_DELETE_DICTIONARY_ENTRY: &str = "delete_dictionary_entry";
pub const COMMAND_LIST_SNIPPET: &str = "list_snippet";
pub const COMMAND_ADD_SNIPPET: &str = "add_snippet";
pub const COMMAND_UPDATE_SNIPPET: &str = "update_snippet";
pub const COMMAND_DELETE_SNIPPET: &str = "delete_snippet";
pub const COMMAND_LIST_CUSTOM_PROMPT: &str = "list_custom_prompt";
pub const COMMAND_ADD_CUSTOM_PROMPT: &str = "add_custom_prompt";
pub const COMMAND_UPDATE_CUSTOM_PROMPT: &str = "update_custom_prompt";
pub const COMMAND_DELETE_CUSTOM_PROMPT: &str = "delete_custom_prompt";
pub const COMMAND_LIST_APP_RULE: &str = "list_app_rule";
pub const COMMAND_ADD_APP_RULE: &str = "add_app_rule";
pub const COMMAND_UPDATE_APP_RULE: &str = "update_app_rule";
pub const COMMAND_DELETE_APP_RULE: &str = "delete_app_rule";
pub const COMMAND_LIST_PERSONAS: &str = "list_personas";
pub const COMMAND_LIST_TEMPLATES: &str = "list_templates";
pub const COMMAND_LIST_INPUT_DEVICES: &str = "list_input_devices";
pub const COMMAND_TEST_INJECTION: &str = "test_injection";
pub const COMMAND_CHECK_PERMISSIONS: &str = "check_permissions";
pub const COMMAND_OPEN_PERMISSION_PANE: &str = "open_permission_pane";
pub const COMMAND_EXPORT_HISTORY: &str = "export_history";
pub const COMMAND_GET_APP_VERSION: &str = "get_app_version";

pub const IPC_COMMANDS: [&str; 42] = [
    "get_settings",
    "update_settings",
    "set_api_key",
    "has_api_key",
    "delete_api_key",
    "start_dictation",
    "stop_dictation",
    "cancel_dictation",
    "list_models",
    "download_model",
    "cancel_download",
    "delete_model",
    "list_dictations",
    "get_dictation",
    "delete_dictation",
    "clear_history",
    "reprocess_dictation",
    "get_audio_url",
    "list_dictionary_entry",
    "add_dictionary_entry",
    "update_dictionary_entry",
    "delete_dictionary_entry",
    "list_snippet",
    "add_snippet",
    "update_snippet",
    "delete_snippet",
    "list_custom_prompt",
    "add_custom_prompt",
    "update_custom_prompt",
    "delete_custom_prompt",
    "list_app_rule",
    "add_app_rule",
    "update_app_rule",
    "delete_app_rule",
    "list_personas",
    "list_templates",
    "list_input_devices",
    "test_injection",
    "check_permissions",
    "open_permission_pane",
    "export_history",
    "get_app_version",
];

fn todo_error(name: &str) -> ApiError {
    ApiError {
        code: "TODO".to_string(),
        message: format!("{name} not implemented yet"),
    }
}

type ModelProgressCallback = Box<dyn FnMut(DownloadProgress) + Send>;
type ModelDownloadFuture = Pin<Box<dyn Future<Output = Result<(), ModelManagerError>> + Send>>;

/// Startup can create the IPC graph before the platform capture/context/model
/// services are available. This preserves the stable §14 error surface instead
/// of leaving the dictation commands as M0 `TODO` stubs.
#[allow(dead_code)]
struct UnavailableDictationRuntime;

impl DictationRuntime for UnavailableDictationRuntime {
    fn start_dictation(&self) -> Result<(), Error> {
        Err(Error::AsrNoModel(
            "dictation runtime is not configured".to_string(),
        ))
    }

    fn stop_dictation(&self) -> Result<(), Error> {
        Err(Error::AsrNoModel(
            "dictation runtime is not configured".to_string(),
        ))
    }

    fn cancel_dictation(&self) -> Result<(), Error> {
        Err(Error::AsrNoModel(
            "dictation runtime is not configured".to_string(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn default_dictation_runtime(
    settings: Arc<Mutex<SettingsStore>>,
    keys: Arc<dyn KeyStore>,
) -> Arc<dyn DictationRuntime> {
    let injector = default_injector(Arc::clone(&settings));
    Arc::new(crate::pipeline::MacSessionRuntime::spawn(
        settings, injector, keys,
    ))
}

#[cfg(not(target_os = "macos"))]
fn default_dictation_runtime(
    _settings: Arc<Mutex<SettingsStore>>,
    _keys: Arc<dyn KeyStore>,
) -> Arc<dyn DictationRuntime> {
    Arc::new(UnavailableDictationRuntime)
}

/// Default macOS injection stack (FR-1.4): the policy service over the native
/// NSPasteboard/enigo adapters, rebuilt per call so live §8.3 injection
/// settings always apply.
#[cfg(target_os = "macos")]
struct SettingsDrivenInjector {
    settings: Arc<Mutex<SettingsStore>>,
}

#[cfg(target_os = "macos")]
impl Injector for SettingsDrivenInjector {
    fn inject(&self, text: &str, ctx: &InjectContext) -> Result<InjectMethod, Error> {
        let mut config = crate::inject::InjectionSettings::default();
        if let Ok(store) = self.settings.lock() {
            if let Ok(settings) = store.get() {
                config.type_threshold_chars = settings.injection.type_threshold_chars as usize;
                config.restore_clipboard_delay_ms =
                    settings.injection.restore_clipboard_delay_ms as u64;
            }
        }
        crate::inject::InjectionService::new(
            crate::inject::macos::MacClipboard,
            crate::inject::macos::MacTypist,
            crate::inject::macos::MacPasteVerifier,
            config,
        )
        .inject(text, ctx)
    }
}

/// Startup can create the IPC graph before platform adapters exist; this keeps
/// the stable §14 error surface instead of a M0 `TODO` stub.
#[allow(dead_code)]
struct UnavailableInjector;

impl Injector for UnavailableInjector {
    fn inject(&self, _text: &str, _ctx: &InjectContext) -> Result<InjectMethod, Error> {
        Err(Error::InjFail("injection requires macOS".to_string()))
    }
}

fn default_injector(shared_settings: Arc<Mutex<SettingsStore>>) -> Arc<dyn Injector> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(SettingsDrivenInjector {
            settings: shared_settings,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = shared_settings;
        Arc::new(UnavailableInjector)
    }
}

#[allow(dead_code)]
struct ConservativePermissionProbe;

impl PermissionProbe for ConservativePermissionProbe {
    fn snapshot(&self) -> PermissionState {
        // Conservative: onboarding asks rather than claims a capability.
        PermissionState::Undetermined
    }
}

#[cfg(target_os = "macos")]
fn default_permission_probe() -> Arc<dyn PermissionProbe> {
    Arc::new(crate::context::macos::MacPermissionProbe)
}

#[cfg(not(target_os = "macos"))]
fn default_permission_probe() -> Arc<dyn PermissionProbe> {
    Arc::new(ConservativePermissionProbe)
}

fn default_injection_context_provider(
) -> Arc<dyn Fn() -> Result<InjectContext, Error> + Send + Sync> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(|| {
            let mut snapshot = crate::context::macos::MacFrontmostSnapshot;
            snapshot.injection_context()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(|| Err(Error::AxPerm("injection requires macOS".to_string())))
    }
}

impl ModelDownloadService for ModelManager {
    fn list_models(&self) -> Result<Vec<InstalledModel>, ModelManagerError> {
        self.list_models()
    }

    fn download(&self, id: String, mut on_progress: ModelProgressCallback) -> ModelDownloadFuture {
        let manager = self.clone();
        Box::pin(async move { manager.download(&id, &mut on_progress).await })
    }

    fn delete(&self, id: &str) -> Result<(), ModelManagerError> {
        self.delete(id)
    }

    fn remove_partial(&self, id: &str) -> Result<(), ModelManagerError> {
        self.remove_partial(id)
    }
}

impl IpcState {
    pub fn new(settings: SettingsStore, key_store: Arc<dyn KeyStore>) -> Self {
        let models_dir = crate::store::app_data_dir().join("models");
        let models = ModelManager::new(models_dir)
            .expect("the embedded Whisper model manifest must be valid");
        let settings = Arc::new(Mutex::new(settings));
        let runtime = default_dictation_runtime(Arc::clone(&settings), Arc::clone(&key_store));
        let mut state =
            Self::with_model_service_and_runtime(settings, key_store, Arc::new(models), runtime);
        #[cfg(target_os = "macos")]
        {
            let config = state
                .settings
                .lock()
                .ok()
                .and_then(|store| store.get().ok())
                .and_then(|settings| {
                    HotkeyConfig::from_wire(
                        &settings.hotkey.mode,
                        &settings.hotkey.push_to_talk_key,
                        &settings.hotkey.toggle_combo,
                        settings.hotkey.esc_cancels,
                    )
                    .ok()
                })
                .unwrap_or_default();
            state.start_hotkeys(config);
        }
        state
    }

    /// Installs the session runtime used by the three §9.1 dictation controls.
    /// Tests use this to prove dispatch without touching microphone hardware.
    pub fn with_runtime(
        settings: SettingsStore,
        key_store: Arc<dyn KeyStore>,
        runtime: Arc<dyn DictationRuntime>,
    ) -> Self {
        let models_dir = crate::store::app_data_dir().join("models");
        let models = ModelManager::new(models_dir)
            .expect("the embedded Whisper model manifest must be valid");
        Self::with_model_service_and_runtime(
            Arc::new(Mutex::new(settings)),
            key_store,
            Arc::new(models),
            runtime,
        )
    }

    pub fn with_runtime_and_hotkey(
        settings: SettingsStore,
        key_store: Arc<dyn KeyStore>,
        runtime: Arc<dyn DictationRuntime>,
        hotkey_controller: HotkeyController,
    ) -> Self {
        let models_dir = crate::store::app_data_dir().join("models");
        let models = ModelManager::new(models_dir)
            .expect("the embedded Whisper model manifest must be valid");
        let mut state = Self::with_model_service_and_runtime(
            Arc::new(Mutex::new(settings)),
            key_store,
            Arc::new(models),
            runtime,
        );
        state.hotkey_controller = Some(hotkey_controller);
        state
    }

    #[cfg(test)]
    fn with_model_service(
        settings: SettingsStore,
        key_store: Arc<dyn KeyStore>,
        model_service: Arc<dyn ModelDownloadService>,
    ) -> Self {
        Self::with_model_service_and_runtime(
            Arc::new(Mutex::new(settings)),
            key_store,
            model_service,
            Arc::new(UnavailableDictationRuntime),
        )
    }

    fn with_model_service_and_runtime(
        settings: Arc<Mutex<SettingsStore>>,
        key_store: Arc<dyn KeyStore>,
        model_service: Arc<dyn ModelDownloadService>,
        runtime: Arc<dyn DictationRuntime>,
    ) -> Self {
        let injector = default_injector(Arc::clone(&settings));
        Self {
            settings,
            key_store,
            model_service,
            downloads: Arc::new(Mutex::new(HashMap::new())),
            download_reservations: Arc::new(Mutex::new(HashMap::new())),
            next_download_generation: Arc::new(AtomicU64::new(1)),
            runtime,
            hotkey_controller: None,
            injector,
            injection_context_provider: default_injection_context_provider(),
            permission_probe: default_permission_probe(),
        }
    }

    /// Installs a deterministic §9.3 `Injector` seam (tests, slice wiring).
    pub fn with_injector(mut self, injector: Arc<dyn Injector>) -> Self {
        self.injector = injector;
        self
    }

    /// Installs a deterministic permission-probe seam (tests, slice wiring).
    pub fn with_permission_probe(mut self, probe: Arc<dyn PermissionProbe>) -> Self {
        self.permission_probe = probe;
        self
    }

    /// Installs a deterministic injection-context seam (tests, slice wiring).
    pub fn with_injection_context_provider(
        mut self,
        provider: Arc<dyn Fn() -> Result<InjectContext, Error> + Send + Sync>,
    ) -> Self {
        self.injection_context_provider = provider;
        self
    }

    fn get_settings(&self) -> Result<Settings, ApiError> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| map_ipc_error(Error::DbIo("IPC settings lock failed".to_string())))?;
        settings.get().map_err(map_ipc_error)
    }

    fn update_settings(&self, mut patch: Value) -> Result<Settings, ApiError> {
        let settings = self
            .settings
            .lock()
            .map_err(|_| map_ipc_error(Error::DbIo("IPC settings lock failed".to_string())))?;
        if let Some(hotkey_patch) = patch.get("hotkey").and_then(Value::as_object) {
            let current = settings.get().map_err(map_ipc_error)?;
            let mode = hotkey_patch
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or(&current.hotkey.mode);
            let key = hotkey_patch
                .get("pushToTalkKey")
                .and_then(Value::as_str)
                .unwrap_or(&current.hotkey.push_to_talk_key);
            let combo = hotkey_patch
                .get("toggleCombo")
                .and_then(Value::as_str)
                .unwrap_or(&current.hotkey.toggle_combo);
            let esc = hotkey_patch
                .get("escCancels")
                .and_then(Value::as_bool)
                .unwrap_or(current.hotkey.esc_cancels);
            HotkeyConfig::from_wire(mode, key, combo, esc).map_err(|error| {
                map_ipc_error(Error::HkPerm(format!(
                    "invalid hotkey configuration: {error}"
                )))
            })?;
        }
        // A user reselecting the configured local model explicitly clears any
        // prior automatic slow-decode downgrade. The override is session-local
        // policy, not a permanent replacement for the user's setting.
        if patch
            .get("asr")
            .and_then(Value::as_object)
            .is_some_and(|asr| asr.contains_key("localModel"))
        {
            if let Some(asr) = patch.get_mut("asr").and_then(Value::as_object_mut) {
                asr.insert("effectiveLocalModel".to_string(), Value::Null);
            }
        }
        let previous = settings.get().map_err(map_ipc_error)?;
        let updated = settings.update(patch).map_err(map_ipc_error)?;
        drop(settings);
        if let Some(controller) = &self.hotkey_controller {
            if let Ok(config) = HotkeyConfig::from_wire(
                &updated.hotkey.mode,
                &updated.hotkey.push_to_talk_key,
                &updated.hotkey.toggle_combo,
                updated.hotkey.esc_cancels,
            ) {
                if let Err(error) = controller.send(HotkeyControl::Rebind(config)) {
                    // Keep the persisted and live bindings atomic from the
                    // caller's perspective when the manager has gone away.
                    let rollback_result = match self.settings.lock() {
                        Ok(store) => {
                            let rollback = serde_json::json!({
                                "hotkey": {
                                    "mode": previous.hotkey.mode,
                                    "pushToTalkKey": previous.hotkey.push_to_talk_key,
                                    "toggleCombo": previous.hotkey.toggle_combo,
                                    "escCancels": previous.hotkey.esc_cancels,
                                }
                            });
                            store.update(rollback).map(|_| ())
                        }
                        Err(_) => Err(Error::DbIo("hotkey settings rollback lock failed".into())),
                    };
                    if let Err(rollback_error) = rollback_result {
                        tracing::warn!(
                            code = %rollback_error.code(),
                            %rollback_error,
                            "hotkey settings rollback failed"
                        );
                        return Err(map_ipc_error(rollback_error));
                    }
                    return Err(map_ipc_error(Error::HkPerm(format!(
                        "hotkey manager unavailable: {error}"
                    ))));
                }
            }
        }
        Ok(updated)
    }

    fn set_api_key(&self, provider: Provider, value: String) -> Result<(), ApiError> {
        keychain::set_api_key(self.key_store.as_ref(), provider, &value).map_err(map_ipc_error)
    }

    fn has_api_key(&self, provider: Provider) -> Result<bool, ApiError> {
        keychain::has_api_key(self.key_store.as_ref(), provider).map_err(map_ipc_error)
    }

    fn delete_api_key(&self, provider: Provider) -> Result<(), ApiError> {
        keychain::delete_api_key(self.key_store.as_ref(), provider).map_err(map_ipc_error)
    }

    fn start_dictation(&self) -> Result<(), ApiError> {
        self.runtime.start_dictation().map_err(map_ipc_error)
    }

    fn stop_dictation(&self) -> Result<(), ApiError> {
        self.runtime.stop_dictation().map_err(map_ipc_error)
    }

    fn cancel_dictation(&self) -> Result<(), ApiError> {
        self.runtime.cancel_dictation().map_err(map_ipc_error)
    }

    pub fn toggle_dictation(&self) -> Result<(), ApiError> {
        self.runtime.toggle_dictation().map_err(map_ipc_error)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>, ApiError> {
        self.model_service
            .list_models()
            .map(|models| models.into_iter().map(ModelInfo::from).collect())
            .map_err(map_model_error)
    }

    fn delete_model(&self, id: &str) -> Result<(), ApiError> {
        self.model_service.delete(id).map_err(map_model_error)
    }

    #[cfg(test)]
    fn start_download<F>(&self, id: String, on_progress: F) -> Result<(), ApiError>
    where
        F: FnMut(DownloadProgress) + Send + 'static,
    {
        self.start_download_with_error(id, on_progress, |_| {})
    }

    fn start_download_with_error<F, G>(
        &self,
        id: String,
        on_progress: F,
        on_error: G,
    ) -> Result<(), ApiError>
    where
        F: FnMut(DownloadProgress) + Send + 'static,
        G: FnOnce(Error) + Send + 'static,
    {
        let mut reservations = self.download_reservations.lock().map_err(|_| {
            map_ipc_error(Error::DbIo(
                "IPC model download reservation lock failed".to_string(),
            ))
        })?;
        if reservations.contains_key(&id) {
            return Err(map_ipc_error(Error::AsrLoad(format!(
                "model download already active: {id}"
            ))));
        }
        let generation = self
            .next_download_generation
            .fetch_add(1, Ordering::Relaxed);
        reservations.insert(
            id.clone(),
            DownloadReservation {
                generation,
                cancelling: false,
            },
        );
        let mut downloads = match self.downloads.lock() {
            Ok(downloads) => downloads,
            Err(_) => {
                reservations.remove(&id);
                return Err(map_ipc_error(Error::DbIo(
                    "IPC model download lock failed".to_string(),
                )));
            }
        };
        let service = Arc::clone(&self.model_service);
        let active_downloads = Arc::clone(&self.downloads);
        let active_reservations = Arc::clone(&self.download_reservations);
        let task_id = id.clone();
        let handle = tauri::async_runtime::spawn(async move {
            let result = service
                .download(task_id.clone(), Box::new(on_progress))
                .await;
            if let Err(error) = result {
                // Release this generation's reservation BEFORE reporting the
                // failure so an immediate synchronous restart observes a free
                // slot instead of racing the cleanup above (FR-1.3).
                if let Ok(mut reservations) = active_reservations.lock() {
                    if reservations.get(&task_id).is_some_and(|reservation| {
                        reservation.generation == generation && !reservation.cancelling
                    }) {
                        reservations.remove(&task_id);
                        if let Ok(mut active) = active_downloads.lock() {
                            active.remove(&task_id);
                        }
                    }
                }
                on_error(Error::AsrLoad(error.to_string()));
            } else if let Ok(mut reservations) = active_reservations.lock() {
                if reservations.get(&task_id).is_some_and(|reservation| {
                    reservation.generation == generation && !reservation.cancelling
                }) {
                    reservations.remove(&task_id);
                    if let Ok(mut active) = active_downloads.lock() {
                        active.remove(&task_id);
                    }
                }
            }
        });
        downloads.insert(id, handle);
        Ok(())
    }

    async fn cancel_download(&self, id: &str) -> Result<(), ApiError> {
        let generation = {
            let mut reservations = self.download_reservations.lock().map_err(|_| {
                map_ipc_error(Error::DbIo(
                    "IPC model download reservation lock failed".to_string(),
                ))
            })?;
            if let Some(reservation) = reservations.get_mut(id) {
                reservation.cancelling = true;
                Some(reservation.generation)
            } else {
                let generation = self
                    .next_download_generation
                    .fetch_add(1, Ordering::Relaxed);
                reservations.insert(
                    id.to_string(),
                    DownloadReservation {
                        generation,
                        cancelling: true,
                    },
                );
                Some(generation)
            }
        };
        let handle = self
            .downloads
            .lock()
            .map_err(|_| map_ipc_error(Error::DbIo("IPC model download lock failed".to_string())))?
            .remove(id);
        if let Some(handle) = handle {
            handle.abort();
            if let Err(error) = handle.await {
                tracing::warn!(code = "DB-IO", %error, "model download task join failed");
            }
        }
        let cleanup = self
            .model_service
            .remove_partial(id)
            .map_err(map_model_error);
        if let (Some(generation), Ok(mut reservations)) =
            (generation, self.download_reservations.lock())
        {
            if reservations.get(id).is_some_and(|reservation| {
                reservation.generation == generation && reservation.cancelling
            }) {
                reservations.remove(id);
            }
        }
        cleanup
    }
}

impl From<InstalledModel> for ModelInfo {
    fn from(model: InstalledModel) -> Self {
        Self {
            id: model.id,
            label: model.label,
            size_bytes: model.size_bytes,
            installed: model.installed,
            path: model.path.map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

fn map_model_error(error: ModelManagerError) -> ApiError {
    map_ipc_error(Error::AsrLoad(error.to_string()))
}

/// Convert a core error exactly once at the IPC boundary. Its message has
/// already passed through `ApiError::from`; only stable, redacted metadata is
/// ever sent to tracing.
fn map_ipc_error(error: Error) -> ApiError {
    let api_error = ApiError::from(&error);
    tracing::warn!(code = %api_error.code, message = %api_error.message, "IPC command failed");
    api_error
}

async fn await_blocking<T, E>(
    handle: tauri::async_runtime::JoinHandle<Result<T, E>>,
) -> Result<Result<T, E>, ApiError> {
    match handle.await {
        Ok(result) => Ok(result),
        Err(_) => Err(map_ipc_error(Error::DbIo(
            "IPC blocking task failed".to_string(),
        ))),
    }
}

async fn run_blocking<T, F>(operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    await_blocking(tauri::async_runtime::spawn_blocking(operation)).await?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size_bytes: u64,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationRow {
    pub id: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStateSet {
    pub microphone: PermissionState,
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionState {
    Granted,
    Denied,
    Undetermined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessOptions {
    pub id: String,
    pub kind: ReprocessKind,
    pub ref_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReprocessKind {
    Template,
    Persona,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReprocessResult {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionTestResult {
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDictationsQuery {
    pub q: Option<String>,
    pub limit: Option<u32>,
    #[serde(rename = "beforeId")]
    pub before_id: Option<String>,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, IpcState>) -> Result<Settings, ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.get_settings()).await
}

#[tauri::command]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, IpcState>,
    patch: Value,
) -> Result<Settings, ApiError> {
    let state = state.inner().clone();
    let state_for_rebind = state.clone();
    let previous_combo = state_for_rebind.get_settings()?.hotkey.toggle_combo;
    let updated = run_blocking(move || state.update_settings(patch)).await?;
    if previous_combo != updated.hotkey.toggle_combo {
        let controller = state_for_rebind
            .hotkey_controller
            .clone()
            .ok_or_else(|| map_ipc_error(Error::HkPerm("hotkey manager unavailable".into())))?;
        if let Err(error) = app.global_shortcut().unregister(previous_combo.as_str()) {
            let rollback = state_for_rebind.update_settings(serde_json::json!({
                "hotkey": { "toggleCombo": previous_combo }
            }));
            if let Err(rollback_error) = rollback {
                return Err(map_ipc_error(Error::HkPerm(format!(
                    "could not restore previous toggle setting after unregister failure: {} {}",
                    rollback_error.code, rollback_error.message
                ))));
            }
            return Err(map_ipc_error(Error::HkPerm(format!(
                "could not unregister previous toggle shortcut: {error}"
            ))));
        }
        let combo = updated.hotkey.toggle_combo.clone();
        if let Err(error) = app.global_shortcut().on_shortcut(combo.as_str(), {
            let controller = controller.clone();
            move |_app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    if let Err(error) = controller.send(HotkeyControl::Toggle(Instant::now())) {
                        tracing::warn!(
                            code = "HK-PERM",
                            ?error,
                            "global shortcut toggle policy unavailable"
                        );
                    }
                }
            }
        }) {
            // Restore the old registration and persisted setting when the OS
            // rejects the new accelerator, preserving atomic live rebinding.
            let restore_registration =
                app.global_shortcut().on_shortcut(previous_combo.as_str(), {
                    let controller = controller.clone();
                    move |_app, _shortcut, event| {
                        if event.state == ShortcutState::Pressed {
                            if let Err(error) =
                                controller.send(HotkeyControl::Toggle(Instant::now()))
                            {
                                tracing::warn!(
                                    code = "HK-PERM",
                                    ?error,
                                    "global shortcut toggle policy unavailable"
                                );
                            }
                        }
                    }
                });
            let rollback = state_for_rebind.update_settings(serde_json::json!({
                "hotkey": { "toggleCombo": previous_combo }
            }));
            if let Err(restore_error) = restore_registration {
                return Err(map_ipc_error(Error::HkPerm(format!(
                    "could not restore previous toggle shortcut: {restore_error}"
                ))));
            }
            if let Err(rollback_error) = rollback {
                return Err(map_ipc_error(Error::HkPerm(format!(
                    "could not restore previous toggle setting: {} {}",
                    rollback_error.code, rollback_error.message
                ))));
            }
            return Err(map_ipc_error(Error::HkPerm(format!(
                "could not register toggle shortcut: {error}"
            ))));
        }
    }
    Ok(updated)
}

#[tauri::command]
pub async fn set_api_key(
    state: State<'_, IpcState>,
    provider: Provider,
    value: String,
) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.set_api_key(provider, value)).await
}

#[tauri::command]
pub async fn has_api_key(state: State<'_, IpcState>, provider: Provider) -> Result<bool, ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.has_api_key(provider)).await
}

#[tauri::command]
pub async fn delete_api_key(
    state: State<'_, IpcState>,
    provider: Provider,
) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.delete_api_key(provider)).await
}

#[tauri::command]
pub async fn start_dictation(state: State<'_, IpcState>) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.start_dictation()).await
}

#[tauri::command]
pub async fn stop_dictation(state: State<'_, IpcState>) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.stop_dictation()).await
}

#[tauri::command]
pub async fn cancel_dictation(state: State<'_, IpcState>) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.cancel_dictation()).await
}

#[tauri::command]
pub async fn list_models(state: State<'_, IpcState>) -> Result<Vec<ModelInfo>, ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.list_models()).await
}

#[tauri::command]
pub fn download_model<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, IpcState>,
    id: String,
) -> Result<(), ApiError> {
    let state = state.inner().clone();
    let error_app = app.clone();
    state.start_download_with_error(
        id,
        move |progress| {
            if let Err(error) = app.emit(
                crate::ipc::events::EVENT_MODEL_DOWNLOAD_PROGRESS,
                crate::ipc::events::ModelDownloadProgressPayload {
                    id: progress.id,
                    received: progress.received,
                    total: progress.total,
                },
            ) {
                tracing::warn!(code = "DB-IO", %error, "model progress event emission failed");
            }
        },
        move |error| {
            tracing::warn!(code = %error.code(), %error, "model download failed");
            if let Err(emit_error) = error_app.emit(
                crate::ipc::events::EVENT_APP_ERROR,
                crate::ipc::events::AppErrorPayload::from_error(&error),
            ) {
                tracing::warn!(code = "DB-IO", %emit_error, "model error event emission failed");
            }
        },
    )
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, IpcState>, id: String) -> Result<(), ApiError> {
    state.inner().cancel_download(&id).await
}

#[tauri::command]
pub async fn delete_model(state: State<'_, IpcState>, id: String) -> Result<(), ApiError> {
    let state = state.inner().clone();
    run_blocking(move || state.delete_model(&id)).await
}

// PRD-QUESTION(Q13)
#[tauri::command]
pub fn list_dictations(_query: Option<ListDictationsQuery>) -> Result<Vec<DictationRow>, ApiError> {
    Err(todo_error(COMMAND_LIST_DICTATIONS))
}

#[tauri::command]
pub fn get_dictation(_id: String) -> Result<DictationRow, ApiError> {
    Err(todo_error(COMMAND_GET_DICTATION))
}

#[tauri::command]
pub fn delete_dictation(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_DICTATION))
}

#[tauri::command]
pub fn clear_history() -> Result<u64, ApiError> {
    Err(todo_error(COMMAND_CLEAR_HISTORY))
}

#[tauri::command]
pub fn reprocess_dictation(_input: ReprocessOptions) -> Result<ReprocessResult, ApiError> {
    Err(todo_error(COMMAND_REPROCESS_DICTATION))
}

#[tauri::command]
pub fn get_audio_url(_id: String) -> Result<String, ApiError> {
    Err(todo_error(COMMAND_GET_AUDIO_URL))
}

#[tauri::command]
pub fn list_dictionary_entry(_query: Option<String>) -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_DICTIONARY_ENTRY))
}

#[tauri::command]
pub fn add_dictionary_entry(_entry: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_ADD_DICTIONARY_ENTRY))
}

#[tauri::command]
pub fn update_dictionary_entry(_id: String, _entry: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_UPDATE_DICTIONARY_ENTRY))
}

#[tauri::command]
pub fn delete_dictionary_entry(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_DICTIONARY_ENTRY))
}

#[tauri::command]
pub fn list_snippet(_query: Option<String>) -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_SNIPPET))
}

#[tauri::command]
pub fn add_snippet(_snippet: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_ADD_SNIPPET))
}

#[tauri::command]
pub fn update_snippet(_id: String, _snippet: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_UPDATE_SNIPPET))
}

#[tauri::command]
pub fn delete_snippet(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_SNIPPET))
}

#[tauri::command]
pub fn list_custom_prompt(_query: Option<String>) -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_CUSTOM_PROMPT))
}

#[tauri::command]
pub fn add_custom_prompt(_prompt: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_ADD_CUSTOM_PROMPT))
}

#[tauri::command]
pub fn update_custom_prompt(_id: String, _prompt: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_UPDATE_CUSTOM_PROMPT))
}

#[tauri::command]
pub fn delete_custom_prompt(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_CUSTOM_PROMPT))
}

#[tauri::command]
pub fn list_app_rule(_query: Option<String>) -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_APP_RULE))
}

#[tauri::command]
pub fn add_app_rule(_rule: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_ADD_APP_RULE))
}

#[tauri::command]
pub fn update_app_rule(_id: String, _rule: Value) -> Result<Value, ApiError> {
    Err(todo_error(COMMAND_UPDATE_APP_RULE))
}

#[tauri::command]
pub fn delete_app_rule(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_APP_RULE))
}

#[tauri::command]
pub fn list_personas() -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_PERSONAS))
}

#[tauri::command]
pub fn list_templates() -> Result<Vec<Value>, ApiError> {
    Ok(crate::llm::prompts::TemplateId::all()
        .iter()
        .map(|template| {
            serde_json::json!({
                "id": template.id(),
                "name": template.name(),
            })
        })
        .collect())
}

#[tauri::command]
pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, ApiError> {
    audio::list_input_devices().map_err(|message| map_ipc_error(Error::MicDev(message)))
}

#[tauri::command]
pub fn test_injection(
    state: State<'_, IpcState>,
    sample: String,
) -> Result<InjectionTestResult, ApiError> {
    // AC-1.4(5): types the sample into whatever is focused. The provider
    // composes the FR-1.4 secure/focus checks; a secure target resolves to the
    // clipboard-only strategy inside the policy, not an error (§14 SEC-FIELD).
    let context = (state.injection_context_provider)().map_err(map_ipc_error)?;
    let method = state
        .injector
        .inject(&sample, &context)
        .map_err(map_ipc_error)?;
    Ok(InjectionTestResult {
        method: crate::inject::wire_method(method).to_string(),
    })
}

/// OS-permission boundary behind `check_permissions` (FR-5.4). The default
/// answers feed every slot; the native macOS probe overrides each slot with
/// its own authoritative source.
pub use crate::state::PermissionProbe;

/// AVAuthorizationStatus -> §9.1 PermissionState (pure, headless-tested).
pub fn av_status_to_permission(status: i64) -> PermissionState {
    match status {
        3 => PermissionState::Granted,
        1 | 2 => PermissionState::Denied,
        _ => PermissionState::Undetermined,
    }
}

#[tauri::command]
pub fn check_permissions(state: State<'_, IpcState>) -> Result<PermissionStateSet, ApiError> {
    let probe = &state.permission_probe;
    Ok(PermissionStateSet {
        microphone: probe.microphone(),
        accessibility: probe.accessibility(),
        input_monitoring: probe.input_monitoring(),
    })
}

#[tauri::command]
pub fn permission_pane_url(kind: &str) -> Result<&'static str, ApiError> {
    match kind {
        "microphone" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        "accessibility" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        }
        "inputMonitoring" | "input_monitoring" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        }
        _ => Err(map_ipc_error(Error::HkPerm(
            "unknown permission pane".into(),
        ))),
    }
}

#[tauri::command]
pub fn open_permission_pane<R: Runtime>(app: AppHandle<R>, kind: String) -> Result<(), ApiError> {
    let url = permission_pane_url(&kind)?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(url, None::<&str>)
        .map_err(|error| {
            map_ipc_error(Error::HkPerm(format!(
                "could not open permission settings: {error}"
            )))
        })
}

#[tauri::command]
pub fn export_history(_dest_path: String) -> Result<u64, ApiError> {
    Err(todo_error(COMMAND_EXPORT_HISTORY))
}

#[tauri::command]
pub fn get_app_version() -> Result<String, ApiError> {
    Err(todo_error(COMMAND_GET_APP_VERSION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::Error, pipeline::DictationRuntime, store::settings::SettingsStore,
        testutil::mocks::InMemoryKeyStore,
    };
    use serde_json::json;
    use std::{
        fs,
        io::Write,
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{Arc, Barrier, Mutex},
        thread,
    };

    #[derive(Default)]
    struct RecordingRuntime {
        actions: Mutex<Vec<&'static str>>,
    }

    impl DictationRuntime for RecordingRuntime {
        fn start_dictation(&self) -> Result<(), Error> {
            self.actions.lock().unwrap().push("start");
            Ok(())
        }

        fn stop_dictation(&self) -> Result<(), Error> {
            self.actions.lock().unwrap().push("stop");
            Ok(())
        }

        fn cancel_dictation(&self) -> Result<(), Error> {
            self.actions.lock().unwrap().push("cancel");
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CaptureBuf(Arc<Mutex<Vec<u8>>>);
    struct CaptureGuard(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureGuard {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("scoped log capture lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureBuf {
        type Writer = CaptureGuard;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureGuard(Arc::clone(&self.0))
        }
    }

    #[test]
    fn command_constant_count_is_stable() {
        assert_eq!(IPC_COMMANDS.len(), 42);
        let mut sorted = IPC_COMMANDS.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            IPC_COMMANDS.len(),
            "command constant list must be unique"
        );
    }

    #[test]
    fn ec_1_3_invalid_hotkey_patch_is_rejected_before_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let state = IpcState::with_runtime(
            SettingsStore::new(temp.path().to_path_buf()),
            Arc::new(InMemoryKeyStore::default()),
            Arc::new(RecordingRuntime::default()),
        );
        let error = state
            .update_settings(json!({ "hotkey": { "pushToTalkKey": "Bogus" } }))
            .expect_err("unsupported key must not be persisted");
        assert_eq!(error.code, "HK-PERM");
        let settings = match state.get_settings() {
            Ok(settings) => settings,
            Err(error) => panic!("settings should remain readable: {}", error.message),
        };
        assert_eq!(settings.hotkey.push_to_talk_key, "AltRight");
    }

    #[test]
    fn fr_1_1_reselecting_local_model_clears_effective_downgrade() {
        let temp = tempfile::tempdir().unwrap();
        let state = IpcState::with_runtime(
            SettingsStore::new(temp.path().to_path_buf()),
            Arc::new(InMemoryKeyStore::default()),
            Arc::new(RecordingRuntime::default()),
        );
        state
            .update_settings(json!({"asr": {"effectiveLocalModel": "base"}}))
            .unwrap_or_else(|error| panic!("seed downgrade failed: {}", error.message));
        let settings = state
            .update_settings(json!({"asr": {"localModel": "tiny"}}))
            .unwrap_or_else(|error| panic!("reselection failed: {}", error.message));
        assert_eq!(settings.asr.local_model, "tiny");
        assert_eq!(settings.asr.effective_local_model, None);
    }

    #[test]
    fn fr_0_5_dictation_commands_dispatch_to_the_managed_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let state = IpcState::with_runtime(
            SettingsStore::new(temp.path().to_path_buf()),
            Arc::new(InMemoryKeyStore::new()),
            runtime.clone(),
        );

        state
            .start_dictation()
            .unwrap_or_else(|_| panic!("start must reach runtime"));
        state
            .stop_dictation()
            .unwrap_or_else(|_| panic!("stop must reach runtime"));
        state
            .cancel_dictation()
            .unwrap_or_else(|_| panic!("cancel must reach runtime"));

        assert_eq!(
            *runtime.actions.lock().unwrap(),
            ["start", "stop", "cancel"]
        );
    }

    #[test]
    fn ec_0_5_poisoned_settings_lock_returns_db_io_without_panic() {
        let temp = tempfile::tempdir().expect("real tempdir for poisoned state test");
        let state = IpcState::new(
            SettingsStore::new(temp.path().to_path_buf()),
            Arc::new(InMemoryKeyStore::new()),
        );

        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = state.settings.lock().expect("fresh lock");
            panic!("deliberately poison managed settings lock");
        }));

        let result = catch_unwind(AssertUnwindSafe(|| state.get_settings()));
        let error = result
            .expect("poisoned settings lock must return an API error instead of unwinding")
            .expect_err("poisoned settings lock must not return defaults or TODO");
        assert_eq!(error.code, "DB-IO");
        assert!(
            !error.message.contains("deliberately poison"),
            "lock-failure message must not expose panic internals through IPC"
        );
    }

    #[test]
    fn ec_0_5_concurrent_settings_updates_preserve_every_real_store_patch() {
        let temp = tempfile::tempdir().expect("real tempdir for settings store");
        let state = Arc::new(IpcState::new(
            SettingsStore::new(temp.path().to_path_buf()),
            Arc::new(InMemoryKeyStore::new()),
        ));
        let barrier = Arc::new(Barrier::new(5));
        let mut workers = Vec::new();

        for (field, value) in [
            ("workerOne", "one"),
            ("workerTwo", "two"),
            ("workerThree", "three"),
            ("workerFour", "four"),
        ] {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                state
                    .update_settings(json!({ "concurrent": { (field): value } }))
                    .unwrap_or_else(|_| {
                        panic!("non-conflicting update must not lose another worker's patch")
                    });
            }));
        }

        barrier.wait();
        for worker in workers {
            worker.join().expect("settings worker must not panic");
        }

        let persisted: Value = serde_json::from_str(
            &fs::read_to_string(temp.path().join("settings.json"))
                .expect("concurrent updates must write the real settings file"),
        )
        .expect("concurrent settings write must remain valid JSON");
        assert_eq!(persisted["concurrent"]["workerOne"], "one");
        assert_eq!(persisted["concurrent"]["workerTwo"], "two");
        assert_eq!(persisted["concurrent"]["workerThree"], "three");
        assert_eq!(persisted["concurrent"]["workerFour"], "four");
    }

    #[test]
    fn p3_ipc_error_mapper_warn_is_redacted_before_raw_subscriber() {
        let secret = "sk-ant-log-test Token adapter-private-value";
        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureBuf(Arc::clone(&captured)))
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .finish();

        let error = tracing::subscriber::with_default(subscriber, || {
            map_ipc_error(Error::DbIo(secret.to_string()))
        });
        assert_eq!(error.code, "DB-IO");
        assert!(error.message.contains("[REDACTED]"));

        let logs = String::from_utf8(captured.lock().expect("scoped log capture lock").clone())
            .expect("tracing formatter emits UTF-8");
        assert_eq!(
            logs.matches(" WARN ").count(),
            1,
            "one mapped IPC error must emit exactly one warning"
        );
        assert!(
            logs.contains(&error.message),
            "warning must contain the exact returned redacted error message"
        );
        for required in ["DB-IO", "[REDACTED]"] {
            assert!(
                logs.contains(required),
                "warning must include required redacted stable metadata"
            );
        }
        for forbidden in [
            "sk-ant-log-test",
            "adapter-private-value",
            "Token adapter-private-value",
        ] {
            assert!(
                !logs.contains(forbidden),
                "raw secret/request material must not reach the adapter warning"
            );
        }
    }

    #[test]
    fn ec_0_5_common_blocking_runner_executes_closure_off_caller_thread() {
        let caller = std::thread::current().id();
        let worker =
            tauri::async_runtime::block_on(run_blocking(|| Ok(std::thread::current().id())))
                .unwrap_or_else(|_| {
                    panic!("the common blocking runner must return its closure result")
                });
        assert_ne!(
            worker, caller,
            "the shared runner must schedule synchronous store/keychain work off the invoker thread"
        );
    }

    #[test]
    fn ec_0_5_cancelled_blocking_join_maps_to_constant_db_io_without_panic_detail() {
        let handle = tauri::async_runtime::spawn(async {
            std::future::pending::<Result<(), Error>>().await
        });
        handle.abort();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureBuf(Arc::clone(&captured)))
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .finish();
        let error = tracing::subscriber::with_default(subscriber, || {
            tauri::async_runtime::block_on(await_blocking(handle))
        })
        .expect_err("a cancelled blocking join must map to the normal IPC error path");
        assert_eq!(error.code, "DB-IO");
        assert_eq!(
            error.message, "IPC blocking task failed",
            "the join error must map to a constant message instead of formatting runtime details"
        );
        let logs = String::from_utf8(captured.lock().expect("scoped log capture lock").clone())
            .expect("tracing formatter emits UTF-8");
        assert!(
            logs.matches(" WARN ").count() == 1,
            "cancelled join mapping must emit exactly one warning"
        );
        assert!(
            logs.contains("DB-IO") && logs.contains(&error.message),
            "cancelled join warning must contain the stable code and exact returned message"
        );
        for forbidden in ["cancelled", "JoinError", "task id", "panic", "runtime"] {
            assert!(
                !logs.contains(forbidden),
                "cancelled join warning must not expose runtime detail"
            );
        }
    }

    #[derive(Default)]
    struct ControlledModelDownloads {
        deleted: Mutex<Vec<String>>,
        partial_cleanups: Mutex<Vec<String>>,
        cleanup_started: Mutex<Option<Arc<std::sync::Barrier>>>,
        cleanup_release: Mutex<Option<Arc<std::sync::Barrier>>>,
        fail_download: Mutex<bool>,
    }

    impl ModelDownloadService for ControlledModelDownloads {
        fn list_models(
            &self,
        ) -> Result<
            Vec<crate::asr::model_manager::InstalledModel>,
            crate::asr::model_manager::ModelManagerError,
        > {
            Ok(vec![crate::asr::model_manager::InstalledModel {
                id: "tiny".into(),
                label: "Tiny".into(),
                size_bytes: 4,
                installed: false,
                path: None,
            }])
        }

        fn download(
            &self,
            id: String,
            mut on_progress: ModelProgressCallback,
        ) -> ModelDownloadFuture {
            let fail = *self.fail_download.lock().unwrap();
            Box::pin(async move {
                if fail {
                    return Err(crate::asr::model_manager::ModelManagerError::UnknownModel(
                        id,
                    ));
                }
                on_progress(crate::asr::model_manager::DownloadProgress {
                    id,
                    received: 2,
                    total: 4,
                });
                std::future::pending().await
            })
        }

        fn delete(&self, id: &str) -> Result<(), crate::asr::model_manager::ModelManagerError> {
            self.deleted.lock().unwrap().push(id.into());
            Ok(())
        }

        fn remove_partial(
            &self,
            id: &str,
        ) -> Result<(), crate::asr::model_manager::ModelManagerError> {
            self.partial_cleanups.lock().unwrap().push(id.into());
            let started = self.cleanup_started.lock().unwrap().clone();
            let release = self.cleanup_release.lock().unwrap().clone();
            if let (Some(started), Some(release)) = (started, release) {
                started.wait();
                release.wait();
            }
            Ok(())
        }
    }

    fn state_with_models(models: Arc<ControlledModelDownloads>) -> IpcState {
        let temp = tempfile::tempdir().unwrap();
        // Keep the temporary directory alive by materializing settings before it
        // drops; the model test only exercises the injected download boundary.
        let settings = SettingsStore::new(temp.keep());
        IpcState::with_model_service(settings, Arc::new(InMemoryKeyStore::new()), models)
    }

    #[test]
    fn fr_1_3_managed_model_state_lists_and_deletes_through_model_service() {
        let models = Arc::new(ControlledModelDownloads::default());
        let state = state_with_models(Arc::clone(&models));

        let listed = state
            .list_models()
            .unwrap_or_else(|_| panic!("list_models must use managed model state"));
        assert_eq!(listed[0].id, "tiny");
        assert_eq!(listed[0].size_bytes, 4);
        assert!(!listed[0].installed);

        state
            .delete_model("tiny")
            .unwrap_or_else(|_| panic!("delete_model must use managed model state"));
        assert_eq!(*models.deleted.lock().unwrap(), vec!["tiny"]);
    }

    #[test]
    fn fr_1_3_download_emits_progress_and_cancel_aborts_then_cleans_partial_only() {
        let models = Arc::new(ControlledModelDownloads::default());
        let state = state_with_models(Arc::clone(&models));
        let (progress_tx, progress_rx) = std::sync::mpsc::sync_channel(1);

        state
            .start_download("tiny".into(), move |progress| {
                progress_tx.send(progress).unwrap();
            })
            .unwrap_or_else(|_| panic!("download_model must start a managed background download"));
        assert_eq!(
            progress_rx
                .recv_timeout(std::time::Duration::from_millis(250))
                .expect("download must emit §9.2 model progress"),
            crate::asr::model_manager::DownloadProgress {
                id: "tiny".into(),
                received: 2,
                total: 4,
            }
        );

        tauri::async_runtime::block_on(state.cancel_download("tiny")).unwrap_or_else(|_| {
            panic!("cancel_download must abort the task and clean only its .part file")
        });
        assert_eq!(*models.partial_cleanups.lock().unwrap(), vec!["tiny"]);
        assert!(models.deleted.lock().unwrap().is_empty());
    }

    #[test]
    fn ec_1_3_cancel_keeps_model_reserved_until_partial_cleanup_finishes() {
        let models = Arc::new(ControlledModelDownloads::default());
        let state = state_with_models(Arc::clone(&models));
        let started = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        *models.cleanup_started.lock().unwrap() = Some(Arc::clone(&started));
        *models.cleanup_release.lock().unwrap() = Some(Arc::clone(&release));

        state
            .start_download("tiny".into(), |_| {})
            .unwrap_or_else(|_| panic!("first download must start"));
        let cancel_state = state.clone();
        let cancel =
            tauri::async_runtime::spawn(async move { cancel_state.cancel_download("tiny").await });

        started.wait();
        let restart = state.start_download("tiny".into(), |_| {});
        assert!(restart.is_err(), "same model stays reserved during cleanup");

        *models.cleanup_started.lock().unwrap() = None;
        *models.cleanup_release.lock().unwrap() = None;
        release.wait();
        tauri::async_runtime::block_on(cancel)
            .expect("cancel task must join")
            .unwrap_or_else(|_| panic!("cancel must complete cleanly"));

        state
            .start_download("tiny".into(), |_| {})
            .unwrap_or_else(|_| panic!("a new generation may start after cleanup completes"));
        tauri::async_runtime::block_on(state.cancel_download("tiny"))
            .unwrap_or_else(|_| panic!("replacement generation must cancel cleanly"));
    }

    #[test]
    fn fr_1_3_background_download_failure_is_reported_and_releases_reservation() {
        let models = Arc::new(ControlledModelDownloads::default());
        *models.fail_download.lock().unwrap() = true;
        let state = state_with_models(Arc::clone(&models));
        let (error_tx, error_rx) = std::sync::mpsc::sync_channel(1);
        state
            .start_download_with_error(
                "tiny".into(),
                |_| {},
                move |error| error_tx.send(error.code()).unwrap(),
            )
            .unwrap_or_else(|_| panic!("failing download must still start asynchronously"));
        assert_eq!(
            error_rx
                .recv_timeout(std::time::Duration::from_millis(250))
                .expect("background failure must reach the error callback"),
            "ASR-LOAD"
        );
        state
            .start_download_with_error("tiny".into(), |_| {}, |_| {})
            .unwrap_or_else(|_| panic!("completed failure must release the model reservation"));
        tauri::async_runtime::block_on(state.cancel_download("tiny"))
            .unwrap_or_else(|_| panic!("replacement download must be cancellable"));
    }

    #[test]
    fn ec_1_3_cancel_without_registered_handle_still_blocks_restart_until_cleanup() {
        let models = Arc::new(ControlledModelDownloads::default());
        let state = state_with_models(Arc::clone(&models));
        let started = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        *models.cleanup_started.lock().unwrap() = Some(Arc::clone(&started));
        *models.cleanup_release.lock().unwrap() = Some(Arc::clone(&release));
        state.download_reservations.lock().unwrap().insert(
            "tiny".into(),
            DownloadReservation {
                generation: 99,
                cancelling: false,
            },
        );

        let cancel_state = state.clone();
        let cancel =
            tauri::async_runtime::spawn(async move { cancel_state.cancel_download("tiny").await });
        started.wait();
        assert!(
            state.start_download("tiny".into(), |_| {}).is_err(),
            "a cancellation with no task handle must still reserve the model id"
        );
        *models.cleanup_started.lock().unwrap() = None;
        *models.cleanup_release.lock().unwrap() = None;
        release.wait();
        tauri::async_runtime::block_on(cancel)
            .expect("cancel task must join")
            .unwrap_or_else(|_| panic!("cancel must complete cleanly"));
        state
            .start_download("tiny".into(), |_| {})
            .unwrap_or_else(|_| panic!("restart is allowed after no-handle cleanup"));
        tauri::async_runtime::block_on(state.cancel_download("tiny"))
            .unwrap_or_else(|_| panic!("replacement must cancel"));
    }

    #[test]
    fn ec_1_3_cancel_with_no_registry_entry_reserves_id_before_partial_cleanup() {
        let models = Arc::new(ControlledModelDownloads::default());
        let state = state_with_models(Arc::clone(&models));
        let started = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        *models.cleanup_started.lock().unwrap() = Some(Arc::clone(&started));
        *models.cleanup_release.lock().unwrap() = Some(Arc::clone(&release));

        let cancel_state = state.clone();
        let cancel =
            tauri::async_runtime::spawn(async move { cancel_state.cancel_download("tiny").await });
        started.wait();
        assert!(
            state.start_download("tiny".into(), |_| {}).is_err(),
            "an untracked cancellation must reserve the id before unlinking"
        );
        *models.cleanup_started.lock().unwrap() = None;
        *models.cleanup_release.lock().unwrap() = None;
        release.wait();
        tauri::async_runtime::block_on(cancel)
            .expect("cancel task must join")
            .unwrap_or_else(|_| panic!("cancel must complete cleanly"));
    }
}
