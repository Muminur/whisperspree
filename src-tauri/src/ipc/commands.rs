//! T0.5 — IPC command skeleton for §9.1.
//!
//! Every command is currently a minimal stub that returns `Err(ApiError {
//! code: "TODO" }` until later milestones implement them.
//! This file is intentionally dependency-light and keeps `serde_json::Value`
//! for many payloads to preserve shape flexibility while the upstream services are
//! added.

use crate::{error::ApiError, store::settings::Settings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size_bytes: u64,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationRow {
    pub id: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStateSet {
    pub microphone: String,
    pub accessibility: String,
    pub input_monitoring: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReprocessOptions {
    pub id: String,
    pub kind: String,
    pub ref_id: String,
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
pub fn get_settings() -> Result<Settings, ApiError> {
    Err(todo_error(COMMAND_GET_SETTINGS))
}

#[tauri::command]
pub fn update_settings(_patch: Value) -> Result<Settings, ApiError> {
    Err(todo_error(COMMAND_UPDATE_SETTINGS))
}

#[tauri::command]
pub fn set_api_key(_provider: String, _value: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_SET_API_KEY))
}

#[tauri::command]
pub fn has_api_key(_provider: String) -> Result<bool, ApiError> {
    Err(todo_error(COMMAND_HAS_API_KEY))
}

#[tauri::command]
pub fn delete_api_key(_provider: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_API_KEY))
}

#[tauri::command]
pub fn start_dictation() -> Result<(), ApiError> {
    Err(todo_error(COMMAND_START_DICTATION))
}

#[tauri::command]
pub fn stop_dictation() -> Result<(), ApiError> {
    Err(todo_error(COMMAND_STOP_DICTATION))
}

#[tauri::command]
pub fn cancel_dictation() -> Result<(), ApiError> {
    Err(todo_error(COMMAND_CANCEL_DICTATION))
}

#[tauri::command]
pub fn list_models() -> Result<Vec<ModelInfo>, ApiError> {
    Err(todo_error(COMMAND_LIST_MODELS))
}

#[tauri::command]
pub fn download_model(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DOWNLOAD_MODEL))
}

#[tauri::command]
pub fn cancel_download(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_CANCEL_DOWNLOAD))
}

#[tauri::command]
pub fn delete_model(_id: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_DELETE_MODEL))
}

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
    Err(todo_error(COMMAND_LIST_TEMPLATES))
}

#[tauri::command]
pub fn list_input_devices() -> Result<Vec<Value>, ApiError> {
    Err(todo_error(COMMAND_LIST_INPUT_DEVICES))
}

#[tauri::command]
pub fn test_injection(_sample: String) -> Result<InjectionTestResult, ApiError> {
    Err(todo_error(COMMAND_TEST_INJECTION))
}

#[tauri::command]
pub fn check_permissions() -> Result<PermissionStateSet, ApiError> {
    Err(todo_error(COMMAND_CHECK_PERMISSIONS))
}

#[tauri::command]
pub fn open_permission_pane(_kind: String) -> Result<(), ApiError> {
    Err(todo_error(COMMAND_OPEN_PERMISSION_PANE))
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
}
