//! Application-owned state graph (PRD §11).
//!
//! IPC command functions borrow this type, but ownership of the managed state
//! belongs here so the command module remains an adapter rather than the
//! application-state definition.

use crate::asr::model_manager::{DownloadProgress, ModelManagerError};
use crate::inject::{InjectContext, Injector};
use crate::pipeline::SessionEventSink;
use crate::{
    error::Error,
    hotkey::HotkeyController,
    pipeline::DictationRuntime,
    store::{keychain::KeyStore, settings::SettingsStore},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicU64, Arc, Mutex},
};

#[cfg(target_os = "macos")]
use crate::hotkey::{
    spawn_action_bridge, spawn_rdev_listener_with_control, HotkeyConfig, PermissionState,
};

type ModelProgressCallback = Box<dyn FnMut(DownloadProgress) + Send>;
type ModelDownloadFuture = Pin<Box<dyn Future<Output = Result<(), ModelManagerError>> + Send>>;

/// Managed dependencies shared by all §9.1 command adapters.
#[derive(Clone)]
pub struct IpcState {
    pub(crate) settings: Arc<Mutex<SettingsStore>>,
    pub(crate) key_store: Arc<dyn KeyStore>,
    pub(crate) model_service: Arc<dyn ModelDownloadService>,
    pub(crate) downloads: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    pub(crate) download_reservations: Arc<Mutex<HashMap<String, DownloadReservation>>>,
    pub(crate) next_download_generation: Arc<AtomicU64>,
    pub(crate) runtime: Arc<dyn DictationRuntime>,
    pub(crate) hotkey_controller: Option<HotkeyController>,
    pub(crate) injector: Arc<dyn Injector>,
    pub(crate) injection_context_provider:
        Arc<dyn Fn() -> Result<InjectContext, Error> + Send + Sync>,
}

impl IpcState {
    /// Installs the post-startup Tauri event bridge without exposing the
    /// runtime's concrete platform implementation to command adapters.
    pub fn install_event_sink(&self, sink: Arc<dyn SessionEventSink>) {
        self.runtime.set_event_sink(sink);
    }

    /// Start the real macOS input boundary once the managed runtime exists.
    /// The rdev callback only forwards events; policy and runtime actions stay
    /// on their dedicated worker threads.
    #[cfg(target_os = "macos")]
    pub(crate) fn start_hotkeys(&mut self, config: HotkeyConfig) {
        let (actions, receiver) = std::sync::mpsc::channel();
        let (controller, _manager) =
            // rdev's registration callback is the authoritative OS permission
            // probe. Start enabled so granted Input Monitoring works on the
            // first launch; a denied probe immediately sends SetPermission(Denied)
            // through the retained controller and degrades without panicking.
            spawn_rdev_listener_with_control(config, PermissionState::Granted, actions);
        let _action_bridge = spawn_action_bridge(self.runtime.clone(), receiver);
        self.hotkey_controller = Some(controller);
    }
}

pub(crate) trait ModelDownloadService: Send + Sync {
    fn list_models(
        &self,
    ) -> Result<Vec<crate::asr::model_manager::InstalledModel>, ModelManagerError>;
    fn download(&self, id: String, on_progress: ModelProgressCallback) -> ModelDownloadFuture;
    fn delete(&self, id: &str) -> Result<(), ModelManagerError>;
    fn remove_partial(&self, id: &str) -> Result<(), ModelManagerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DownloadReservation {
    pub(crate) generation: u64,
    pub(crate) cancelling: bool,
}
