//! T0.5 — IPC skeleton contract tests (RED first, then GREEN stubs).
//!
//! AC-9.1 for milestones M0: every command exists and is registered. Only the
//! five completed T0.3 settings/keychain adapters are functional in T0.5;
//! later-owned command families remain explicit TODO stubs.

use std::{fs, path::PathBuf};

use whisperspree_lib::{error::ApiError, ipc, ipc::commands};

fn assert_todo_error<T>(result: Result<T, ApiError>, expected_command: &str) {
    let err = match result {
        Ok(_) => panic!("{expected_command} should return TODO"),
        Err(err) => err,
    };
    assert_eq!(err.code, "TODO", "todo marker must be code='TODO'");
    assert!(
        err.message.contains(expected_command),
        "todo message must mention command '{expected_command}'"
    );
}

#[test]
fn fr_0_5_1_all_commands_return_todo_except_completed_t0_3_adapters() {
    use serde_json::json;
    assert!(commands::IPC_COMMANDS.contains(&commands::COMMAND_START_DICTATION));
    assert!(commands::IPC_COMMANDS.contains(&commands::COMMAND_STOP_DICTATION));
    assert!(commands::IPC_COMMANDS.contains(&commands::COMMAND_CANCEL_DICTATION));
    assert_todo_error(
        commands::list_dictations(Some(commands::ListDictationsQuery {
            q: Some("q".into()),
            limit: Some(10),
            before_id: None,
        })),
        commands::COMMAND_LIST_DICTATIONS,
    );
    assert_todo_error(
        commands::get_dictation("id".into()),
        commands::COMMAND_GET_DICTATION,
    );
    assert_todo_error(
        commands::delete_dictation("id".into()),
        commands::COMMAND_DELETE_DICTATION,
    );
    assert_todo_error(commands::clear_history(), commands::COMMAND_CLEAR_HISTORY);
    assert_todo_error(
        commands::reprocess_dictation(commands::ReprocessOptions {
            id: "id".into(),
            kind: commands::ReprocessKind::Template,
            ref_id: "t".into(),
        }),
        commands::COMMAND_REPROCESS_DICTATION,
    );
    assert_todo_error(
        commands::get_audio_url("id".into()),
        commands::COMMAND_GET_AUDIO_URL,
    );
    assert_todo_error(
        commands::list_dictionary_entry(Some("hello".into())),
        commands::COMMAND_LIST_DICTIONARY_ENTRY,
    );
    assert_todo_error(
        commands::add_dictionary_entry(json!({})),
        commands::COMMAND_ADD_DICTIONARY_ENTRY,
    );
    assert_todo_error(
        commands::update_dictionary_entry("id".into(), json!({})),
        commands::COMMAND_UPDATE_DICTIONARY_ENTRY,
    );
    assert_todo_error(
        commands::delete_dictionary_entry("id".into()),
        commands::COMMAND_DELETE_DICTIONARY_ENTRY,
    );
    assert_todo_error(
        commands::list_snippet(Some("term".into())),
        commands::COMMAND_LIST_SNIPPET,
    );
    assert_todo_error(
        commands::add_snippet(json!({})),
        commands::COMMAND_ADD_SNIPPET,
    );
    assert_todo_error(
        commands::update_snippet("id".into(), json!({})),
        commands::COMMAND_UPDATE_SNIPPET,
    );
    assert_todo_error(
        commands::delete_snippet("id".into()),
        commands::COMMAND_DELETE_SNIPPET,
    );
    assert_todo_error(
        commands::list_custom_prompt(Some("term".into())),
        commands::COMMAND_LIST_CUSTOM_PROMPT,
    );
    assert_todo_error(
        commands::add_custom_prompt(json!({})),
        commands::COMMAND_ADD_CUSTOM_PROMPT,
    );
    assert_todo_error(
        commands::update_custom_prompt("id".into(), json!({})),
        commands::COMMAND_UPDATE_CUSTOM_PROMPT,
    );
    assert_todo_error(
        commands::delete_custom_prompt("id".into()),
        commands::COMMAND_DELETE_CUSTOM_PROMPT,
    );
    assert_todo_error(
        commands::list_app_rule(Some("rule".into())),
        commands::COMMAND_LIST_APP_RULE,
    );
    assert_todo_error(
        commands::add_app_rule(json!({})),
        commands::COMMAND_ADD_APP_RULE,
    );
    assert_todo_error(
        commands::update_app_rule("id".into(), json!({})),
        commands::COMMAND_UPDATE_APP_RULE,
    );
    assert_todo_error(
        commands::delete_app_rule("id".into()),
        commands::COMMAND_DELETE_APP_RULE,
    );
    assert_todo_error(commands::list_personas(), commands::COMMAND_LIST_PERSONAS);
    let templates = match commands::list_templates() {
        Ok(value) => value,
        Err(error) => panic!("template metadata unexpectedly failed: {}", error.message),
    };
    assert_eq!(templates.len(), 17);
    assert!(templates
        .iter()
        .all(|template| template.get("id").is_some()));
    match commands::list_input_devices() {
        Ok(devices) => assert!(devices.iter().all(|device| !device.id.is_empty())),
        Err(error) => assert_eq!(error.code, "MIC-DEV"),
    }
    // T2.2 owns `test_injection`; it now dispatches through the managed
    // §9.3 Injector and is covered by tests/injection_command.rs.
    let permissions = match commands::check_permissions() {
        Ok(permissions) => permissions,
        Err(error) => panic!("permission snapshot failed: {}", error.message),
    };
    assert_eq!(
        permissions.microphone,
        commands::PermissionState::Undetermined
    );
    assert!(commands::permission_pane_url("microphone").is_ok());
    assert_todo_error(
        commands::export_history("/tmp/out.jsonl".into()),
        commands::COMMAND_EXPORT_HISTORY,
    );
    assert_todo_error(
        commands::get_app_version(),
        commands::COMMAND_GET_APP_VERSION,
    );
}

/// Q13 / §12 P-6: T0.4 persistence primitives do not authorize partial M0
/// command adapters. These specific store-facing families remain `TODO` until
/// their later task owns complete DTOs and, for history deletion, audio-path
/// confinement.
#[test]
fn fr_0_5_later_owned_store_commands_remain_registered_todo() {
    use serde_json::json;

    assert_todo_error(
        commands::list_dictations(None),
        commands::COMMAND_LIST_DICTATIONS,
    );
    assert_todo_error(
        commands::get_dictation("dictation-1".into()),
        commands::COMMAND_GET_DICTATION,
    );
    assert_todo_error(
        commands::delete_dictation("dictation-1".into()),
        commands::COMMAND_DELETE_DICTATION,
    );
    assert_todo_error(commands::clear_history(), commands::COMMAND_CLEAR_HISTORY);
    assert_todo_error(
        commands::reprocess_dictation(commands::ReprocessOptions {
            id: "dictation-1".into(),
            kind: commands::ReprocessKind::Template,
            ref_id: "follow-up-email".into(),
        }),
        commands::COMMAND_REPROCESS_DICTATION,
    );
    assert_todo_error(
        commands::get_audio_url("dictation-1".into()),
        commands::COMMAND_GET_AUDIO_URL,
    );
    assert_todo_error(
        commands::export_history("/tmp/history.jsonl".into()),
        commands::COMMAND_EXPORT_HISTORY,
    );
    assert_todo_error(
        commands::add_dictionary_entry(json!({ "spoken": "WhisperSpree" })),
        commands::COMMAND_ADD_DICTIONARY_ENTRY,
    );
    assert_todo_error(
        commands::add_snippet(json!({ "trigger": "address" })),
        commands::COMMAND_ADD_SNIPPET,
    );
    assert_todo_error(
        commands::add_custom_prompt(json!({ "name": "custom" })),
        commands::COMMAND_ADD_CUSTOM_PROMPT,
    );
    assert_todo_error(
        commands::add_app_rule(json!({ "bundleId": "com.example.app" })),
        commands::COMMAND_ADD_APP_RULE,
    );
    assert_todo_error(commands::list_personas(), commands::COMMAND_LIST_PERSONAS);
    let templates = match commands::list_templates() {
        Ok(value) => value,
        Err(error) => panic!("template metadata unexpectedly failed: {}", error.message),
    };
    assert_eq!(templates.len(), 17);
}

/// Q13 fixes the M0 ownership boundary in source as well as behavior: only
/// the completed T0.3 adapters may precede the explicit later-owned TODO
/// group. The marker belongs at that boundary, not on an unrelated command.
#[test]
fn q13_marker_is_adjacent_to_deferred_later_owned_todo_command_group() {
    let commands_source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ipc/commands.rs"))
            .expect("the production IPC command module must exist");
    let marker = "// PRD-QUESTION(Q13)";
    assert_eq!(
        commands_source.matches(marker).count(),
        1,
        "Q13 must have exactly one source marker"
    );
    let marker_start = commands_source
        .find(marker)
        .expect("the exact Q13 marker must be present in commands.rs");
    let marker_end = marker_start + marker.len();

    let first_deferred_function = commands_source
        .find("fn list_dictations")
        .expect("the deferred command group must begin at list_dictations");
    let first_deferred_attribute = commands_source[..first_deferred_function]
        .rfind("#[tauri::command]")
        .expect("the first deferred command must retain its Tauri command attribute");
    assert!(
        commands_source[marker_end..first_deferred_attribute]
            .trim()
            .is_empty(),
        "Q13 must be immediately before the first deferred command group attribute"
    );

    for completed_function in [
        "fn get_settings",
        "fn update_settings",
        "fn set_api_key",
        "fn has_api_key",
        "fn delete_api_key",
    ] {
        let position = commands_source
            .find(completed_function)
            .expect("each completed T0.3 adapter must remain declared");
        assert!(
            position < marker_start,
            "Q13 must not annotate one of the five completed T0.3 adapters"
        );
    }

    let first_later_owned_store_function = commands_source
        .find("fn list_dictations")
        .expect("the later-owned history TODO family must remain in the deferred group");
    assert_eq!(first_deferred_function, first_later_owned_store_function);
}

#[test]
fn fr_0_5_2_all_commands_are_registered() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ipc_mod = fs::read_to_string(manifest_dir.join("src/ipc/mod.rs")).unwrap();
    let handler = ipc_mod
        .split("generate_handler!")
        .nth(1)
        .expect("T0.5: ipc::configure_ipc must have invoke_handler(generate_handler![...])");

    for command in commands::IPC_COMMANDS {
        let needle = format!("ipc::commands::{}", command);
        assert!(
            handler.contains(&needle),
            "T0.5: ipc command '{command}' must be registered in generate_handler!"
        );
    }
}

#[test]
fn fr_0_5_3_events_and_commands_constant_counts_are_stable() {
    assert_eq!(commands::IPC_COMMANDS.len(), 42, "T0.5 §9.1 command count");
    assert_eq!(ipc::events::IPC_EVENTS.len(), 10, "T0.5 §9.2 event count");
}
