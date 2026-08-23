//! WhisperSpree application shell (T0.1 scaffold).
//!
//! `run()` wires up the Tauri builder: the single-instance guard (PRD §4.1) and
//! a minimal menu-bar tray stub (FR-5.2). Feature modules (audio, asr, llm,
//! pipeline, …) are added by later tasks per the §11 layout.

pub mod asr;
pub mod audio;
pub mod context;
pub mod error;
pub mod hotkey;
pub mod inject;
pub mod ipc;
pub mod llm;
pub mod network;
pub mod pipeline;
pub mod state;
pub mod store;
pub mod testutil;

use crate::store::{keychain::KeyringStore, settings::SettingsStore};
use crate::{
    error::Error,
    ipc::events::{self},
    pipeline::{SessionEventSink, SessionState},
};
use std::sync::Arc;
use std::time::Instant;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tracing_appender::non_blocking::WorkerGuard;

struct TauriSessionEventSink<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    hotkey_controller: Option<crate::hotkey::HotkeyController>,
}

impl<R: tauri::Runtime> SessionEventSink for TauriSessionEventSink<R> {
    fn emit_task(
        &self,
        session_id: &str,
        event: crate::pipeline::session_task::SessionTaskEvent,
    ) -> Result<(), Error> {
        events::emit_session_task_event(&self.app, session_id, event)
            .map_err(|error| Error::DbIo(format!("session event emit failed: {error}")))
    }

    fn emit_state(&self, session_id: &str, state: SessionState) -> Result<(), Error> {
        if state == SessionState::Idle {
            if let Some(controller) = &self.hotkey_controller {
                let _ = controller.send(crate::hotkey::HotkeyControl::SessionBecameIdle(
                    Instant::now(),
                ));
            }
        }
        let state = match state {
            SessionState::Idle => events::SessionState::Idle,
            SessionState::Listening => events::SessionState::Listening,
            SessionState::Finalizing => events::SessionState::Finalizing,
            SessionState::PostProcessing => events::SessionState::PostProcessing,
            SessionState::Injecting => events::SessionState::Injecting,
            SessionState::Cancelled => events::SessionState::Cancelled,
            SessionState::Error => events::SessionState::Error,
        };
        events::emit_session_state(
            &self.app,
            events::SessionStatePayload {
                session_id: session_id.to_string(),
                state,
                engine: Some("local".to_string()),
                style_id: None,
                notice: None,
            },
        )
        .map_err(|error| Error::DbIo(format!("session state emit failed: {error}")))
    }

    fn emit_audio_level(&self, _session_id: &str, rms: f32, peak: f32) -> Result<(), Error> {
        events::emit_audio_level(&self.app, events::AudioLevelPayload { rms, peak })
            .map_err(|error| Error::DbIo(format!("audio level event emit failed: {error}")))
    }

    fn speech_observed(&self) {
        if let Some(controller) = &self.hotkey_controller {
            controller.mark_speech_observed();
        }
    }

    fn emit_injection_outcome(
        &self,
        session_id: &str,
        method: &str,
        _persist_history: bool,
    ) -> Result<(), Error> {
        // §9.2 wire payload is {sessionId, method}; `persist_history` is the
        // P-5 signal consumed by the T6.1 history writer inside this flow.
        events::emit_inject_done(
            &self.app,
            events::InjectDonePayload {
                session_id: session_id.to_string(),
                method: method.to_string(),
            },
        )
        .map_err(|error| Error::DbIo(format!("inject done emit failed: {error}")))
    }
}

/// The app log directory: `~/Library/Application Support/WhisperSpree/logs`
/// (PRD §4.4). `dirs::data_dir()` yields the macOS Application Support root.
///
/// Lives here (not in `error.rs`) per OPEN_QUESTIONS Q7: this and [`init_tracing`]
/// are headless-untestable process-bootstrap glue, so they belong in the
/// coverage-ignored bootstrap file alongside the Tauri builder.
fn log_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("WhisperSpree")
        .join("logs")
}

/// Initialize the global `tracing` subscriber: a **daily-rotating** file appender
/// under [`log_dir`] (7-file native retention, PRD §4.4), written through a
/// non-blocking worker, with every formatted line passed through
/// [`error::redact`] (P-3) before it touches disk. The level is read from
/// `WHISPERSPREE_LOG` (PRD §4.2 Logging), lossily.
///
/// Returns the [`WorkerGuard`] that flushes the non-blocking writer; the caller
/// ([`run`]) MUST hold it for the process lifetime or buffered log lines are
/// dropped on exit.
///
/// Lives here (not in `error.rs`) per OPEN_QUESTIONS Q7: it installs a *global*
/// subscriber, spawns a worker thread, and touches the real filesystem, so it is
/// headless-untestable and belongs in the coverage-ignored bootstrap file. The
/// tested redaction logic ([`error::RedactingWriter`] / [`error::redact`]) stays
/// in `error.rs`.
fn init_tracing() -> WorkerGuard {
    let dir = log_dir();
    // Best-effort: the appender also creates the file lazily; ensure the dir first
    // so first-run (no Application Support subtree yet) does not lose early lines.
    let _ = std::fs::create_dir_all(&dir);

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("whisperspree")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .expect("failed to build the rolling log appender");

    let (writer, guard) = tracing_appender::non_blocking(appender);

    // Default to INFO when WHISPERSPREE_LOG is unset: without a default directive
    // `EnvFilter` falls back to ERROR-only, so a stock install would log almost
    // nothing to the rolling file (PRD §4.4 expects a usable default log).
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
        .with_env_var("WHISPERSPREE_LOG")
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(error::RedactingWriter::new(writer))
        .init();

    guard
}

/// Build and run the WhisperSpree desktop application.
pub fn run() {
    // Install the rolling-file tracing subscriber (P-3 redaction) before anything
    // else can log. `_guard` MUST stay bound for the whole function: it flushes the
    // non-blocking log worker on drop, and `.run()` below blocks for the app's
    // lifetime, so the guard lives exactly as long as the process (PRD §4.4 / §12).
    let _guard = init_tracing();

    let builder = tauri::Builder::default()
        // Single-instance guard MUST be the first plugin so a second launch is
        // folded into the running instance before any other init (PRD §4.1).
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init());
    #[rustfmt::skip]
    let state = ipc::commands::IpcState::new(SettingsStore::new(store::app_data_dir()), Arc::new(KeyringStore));

    ipc::configure_ipc(builder, state)
        .setup(|app| {
            let state = app.state::<ipc::commands::IpcState>().inner().clone();
            state.install_event_sink(Arc::new(TauriSessionEventSink {
                app: app.handle().clone(),
                hotkey_controller: state.hotkey_controller.clone(),
            }));
            let hotkey_controller = state.hotkey_controller.clone();
            let toggle_combo = state
                .settings
                .lock()
                .ok()
                .and_then(|store| store.get().ok())
                .map(|settings| settings.hotkey.toggle_combo)
                .unwrap_or_else(|| "Ctrl+Alt+Space".to_string());
            app.global_shortcut().on_shortcut(
                toggle_combo.as_str(),
                move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(controller) = &hotkey_controller {
                            if let Err(error) = controller
                                .send(crate::hotkey::HotkeyControl::Toggle(Instant::now()))
                            {
                                tracing::warn!(
                                    code = "HK-PERM",
                                    ?error,
                                    "global shortcut toggle policy unavailable"
                                );
                            }
                        } else {
                            tracing::warn!(
                                code = "HK-PERM",
                                "global shortcut toggle policy unavailable"
                            );
                        }
                    }
                },
            )?;
            // Menu-bar controls (FR-5.2). Every action uses the same managed
            // runtime/settings boundaries as IPC, so tray clicks cannot create
            // a second session owner or bypass persisted mode selection.
            let start = MenuItem::with_id(
                app,
                "start_dictation",
                "Start dictation",
                true,
                None::<&str>,
            )?;
            let stop =
                MenuItem::with_id(app, "stop_dictation", "Stop dictation", true, None::<&str>)?;
            let mode_auto = MenuItem::with_id(app, "mode_auto", "Mode: Auto", true, None::<&str>)?;
            let mode_local =
                MenuItem::with_id(app, "mode_local", "Mode: Local", true, None::<&str>)?;
            let mode_cloud =
                MenuItem::with_id(app, "mode_cloud", "Mode: Cloud", true, None::<&str>)?;
            let pause =
                MenuItem::with_id(app, "pause_hotkeys", "Pause hotkeys", true, None::<&str>)?;
            let settings =
                MenuItem::with_id(app, "open_settings", "Open Settings", true, None::<&str>)?;
            let history =
                MenuItem::with_id(app, "open_history", "Open History", true, None::<&str>)?;
            let onboarding = MenuItem::with_id(
                app,
                "open_onboarding",
                "Open Onboarding",
                true,
                None::<&str>,
            )?;
            let launch = MenuItem::with_id(
                app,
                "launch_at_login",
                "Launch at Login",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit WhisperSpree", true, None::<&str>)?;
            let items: [&dyn tauri::menu::IsMenuItem<_>; 11] = [
                &start,
                &stop,
                &mode_auto,
                &mode_local,
                &mode_cloud,
                &pause,
                &settings,
                &history,
                &onboarding,
                &launch,
                &quit,
            ];
            let menu = Menu::with_items(app, &items)?;
            let state_for_tray = state.clone();

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| {
                    let id = event.id.as_ref();
                    let result = match id {
                        "start_dictation" => state_for_tray.runtime.start_dictation(),
                        "stop_dictation" => state_for_tray.runtime.stop_dictation(),
                        "mode_auto" | "mode_local" | "mode_cloud" => {
                            let mode = id.strip_prefix("mode_").unwrap_or("auto");
                            state_for_tray
                                .settings
                                .lock()
                                .map_err(|_| Error::DbIo("tray settings lock failed".into()))
                                .and_then(|store| {
                                    store.update(serde_json::json!({"mode": mode})).map(|_| ())
                                })
                        }
                        "pause_hotkeys" => state_for_tray
                            .hotkey_controller
                            .as_ref()
                            .map(|controller| {
                                controller
                                    .send(crate::hotkey::HotkeyControl::SetPermission(
                                        crate::hotkey::PermissionState::Denied,
                                    ))
                                    .map_err(|e| Error::HkPerm(e.to_string()))
                            })
                            .unwrap_or(Ok(())),
                        "launch_at_login" => state_for_tray
                            .settings
                            .lock()
                            .map_err(|_| Error::DbIo("tray settings lock failed".into()))
                            .and_then(|store| {
                                store
                                    .update(serde_json::json!({"launchAtLogin": true}))
                                    .map(|_| ())
                            }),
                        "open_settings" | "open_history" | "open_onboarding" => {
                            let _ = app
                                .get_webview_window(id.strip_prefix("open_").unwrap_or("settings"))
                                .map(|window| window.show());
                            Ok(())
                        }
                        "quit" => {
                            app.exit(0);
                            Ok(())
                        }
                        _ => Ok(()),
                    };
                    if let Err(error) = result {
                        tracing::warn!(code = %error.code(), %error, "tray action failed");
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running WhisperSpree");
}
