//! T0.5 — IPC skeleton contract tests (RED first, then GREEN stubs).
//!
//! AC-9.1 for milestones M0: every command exists, is registered, and currently
//! returns TODO until the feature implementation lands.

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
fn fr_0_5_1_all_commands_return_todo() {
    use serde_json::json;
    assert_todo_error(commands::get_settings(), commands::COMMAND_GET_SETTINGS);
    assert_todo_error(
        commands::update_settings(json!({})),
        commands::COMMAND_UPDATE_SETTINGS,
    );
    assert_todo_error(
        commands::set_api_key("anthropic".into(), "abc".into()),
        commands::COMMAND_SET_API_KEY,
    );
    assert_todo_error(
        commands::has_api_key("anthropic".into()),
        commands::COMMAND_HAS_API_KEY,
    );
    assert_todo_error(
        commands::delete_api_key("anthropic".into()),
        commands::COMMAND_DELETE_API_KEY,
    );
    assert_todo_error(
        commands::start_dictation(),
        commands::COMMAND_START_DICTATION,
    );
    assert_todo_error(commands::stop_dictation(), commands::COMMAND_STOP_DICTATION);
    assert_todo_error(
        commands::cancel_dictation(),
        commands::COMMAND_CANCEL_DICTATION,
    );
    assert_todo_error(commands::list_models(), commands::COMMAND_LIST_MODELS);
    assert_todo_error(
        commands::download_model("tiny".into()),
        commands::COMMAND_DOWNLOAD_MODEL,
    );
    assert_todo_error(
        commands::cancel_download("tiny".into()),
        commands::COMMAND_CANCEL_DOWNLOAD,
    );
    assert_todo_error(
        commands::delete_model("tiny".into()),
        commands::COMMAND_DELETE_MODEL,
    );
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
            kind: "template".into(),
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
    assert_todo_error(commands::list_templates(), commands::COMMAND_LIST_TEMPLATES);
    assert_todo_error(
        commands::list_input_devices(),
        commands::COMMAND_LIST_INPUT_DEVICES,
    );
    assert_todo_error(
        commands::test_injection("sample".into()),
        commands::COMMAND_TEST_INJECTION,
    );
    assert_todo_error(
        commands::check_permissions(),
        commands::COMMAND_CHECK_PERMISSIONS,
    );
    assert_todo_error(
        commands::open_permission_pane("microphone".into()),
        commands::COMMAND_OPEN_PERMISSION_PANE,
    );
    assert_todo_error(
        commands::export_history("/tmp/out.jsonl".into()),
        commands::COMMAND_EXPORT_HISTORY,
    );
    assert_todo_error(
        commands::get_app_version(),
        commands::COMMAND_GET_APP_VERSION,
    );
}

#[test]
fn fr_0_5_2_all_commands_are_registered() {
    use std::fs;
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib = fs::read_to_string(manifest_dir.join("src/lib.rs")).unwrap();
    let handler = lib
        .split("generate_handler!")
        .nth(1)
        .expect("T0.5: tauri::Builder must have invoke_handler(generate_handler![...])");

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
