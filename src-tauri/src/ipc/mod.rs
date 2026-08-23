//! IPC modules (`commands.rs` + `events.rs`) for milestone-zero skeleton.

use crate::ipc;

pub mod commands;
pub mod events;

pub use commands::IPC_COMMANDS;
pub use events::IPC_EVENTS;

/// Install the one managed IPC dependency graph and the complete §9.1 command
/// allowlist. Production and MockRuntime tests both use this exact wiring.
pub fn configure_ipc<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    state: commands::IpcState,
) -> tauri::Builder<R> {
    builder
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            ipc::commands::get_settings,
            ipc::commands::update_settings,
            ipc::commands::set_api_key,
            ipc::commands::has_api_key,
            ipc::commands::delete_api_key,
            ipc::commands::start_dictation,
            ipc::commands::stop_dictation,
            ipc::commands::cancel_dictation,
            ipc::commands::list_models,
            ipc::commands::download_model,
            ipc::commands::cancel_download,
            ipc::commands::delete_model,
            ipc::commands::list_dictations,
            ipc::commands::get_dictation,
            ipc::commands::delete_dictation,
            ipc::commands::clear_history,
            ipc::commands::reprocess_dictation,
            ipc::commands::get_audio_url,
            ipc::commands::list_dictionary_entry,
            ipc::commands::add_dictionary_entry,
            ipc::commands::update_dictionary_entry,
            ipc::commands::delete_dictionary_entry,
            ipc::commands::list_snippet,
            ipc::commands::add_snippet,
            ipc::commands::update_snippet,
            ipc::commands::delete_snippet,
            ipc::commands::list_custom_prompt,
            ipc::commands::add_custom_prompt,
            ipc::commands::update_custom_prompt,
            ipc::commands::delete_custom_prompt,
            ipc::commands::list_app_rule,
            ipc::commands::add_app_rule,
            ipc::commands::update_app_rule,
            ipc::commands::delete_app_rule,
            ipc::commands::list_personas,
            ipc::commands::list_templates,
            ipc::commands::list_input_devices,
            ipc::commands::test_injection,
            ipc::commands::check_permissions,
            ipc::commands::open_permission_pane,
            ipc::commands::export_history,
            ipc::commands::get_app_version,
        ])
}
