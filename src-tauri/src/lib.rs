//! WhisperSpree application shell (T0.1 scaffold).
//!
//! `run()` wires up the Tauri builder: the single-instance guard (PRD §4.1) and
//! a minimal menu-bar tray stub (FR-5.2). Feature modules (audio, asr, llm,
//! pipeline, …) are added by later tasks per the §11 layout.

pub mod error;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tracing_appender::non_blocking::WorkerGuard;

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

    tauri::Builder::default()
        // Single-instance guard MUST be the first plugin so a second launch is
        // folded into the running instance before any other init (PRD §4.1).
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .setup(|app| {
            // Menu-bar tray stub (FR-5.2): a single Quit item. Richer states and
            // the start/stop controls land with T2.6.
            let quit = MenuItem::with_id(app, "quit", "Quit WhisperSpree", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running WhisperSpree");
}
