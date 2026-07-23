//! WhisperSpree application shell (T0.1 scaffold).
//!
//! `run()` wires up the Tauri builder: the single-instance guard (PRD §4.1) and
//! a minimal menu-bar tray stub (FR-5.2). Feature modules (audio, asr, llm,
//! pipeline, …) are added by later tasks per the §11 layout.

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

/// Build and run the WhisperSpree desktop application.
pub fn run() {
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
