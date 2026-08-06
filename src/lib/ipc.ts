import { invoke } from "@tauri-apps/api/core";

export type PermissionState = "granted" | "denied" | "undetermined";

export interface ApiErrorLike {
  code: string;
  message: string;
}

export interface Settings {
  version: number;
  mode: string;
  hotkey: {
    mode: string;
    pushToTalkKey: string;
    toggleCombo: string;
    escCancels: boolean;
  };
  audio: {
    inputDeviceId: string | null;
  };
  asr: {
    localModel: string;
    effectiveLocalModel: string | null;
    cloudProvider: string;
    language: string;
  };
  postprocess: {
    enabled: boolean;
    personaId: string;
    llmProvider: string;
    modelFast: string;
    modelQuality: string;
    timeoutMs: number;
  };
  translation: {
    enabled: boolean;
    targetLanguage: string;
  };
  context: {
    enabled: boolean;
  };
  commands: {
    enabled: boolean;
  };
  injection: {
    typeThresholdChars: number;
    restoreClipboardDelayMs: number;
  };
  history: {
    saveText: boolean;
    retentionDays: number;
    saveAudio: boolean;
  };
  hud: {
    showPartials: boolean;
  };
  launchAtLogin: boolean;
}

export interface DictationRow {
  id: string;
  [key: string]: unknown;
}

export interface ReprocessOptions {
  id: string;
  kind: "template" | "persona";
  refId: string;
}

export interface PermissionSnapshot {
  microphone: PermissionState;
  accessibility: PermissionState;
  inputMonitoring: PermissionState;
}

export interface ModelInfo {
  id: string;
  label: string;
  sizeBytes: number;
  path?: string;
}

export interface InjectionTestResult {
  method: string;
}

export interface InputDevice {
  id: string;
  name: string;
  default: boolean;
}

export interface PersonaSummary {
  id: string;
  name: string;
}

export interface TemplateSummary {
  id: string;
  name: string;
}

export interface ListDictationsQuery {
  q?: string;
  limit?: number;
  beforeId?: string;
}

export const IPC_COMMANDS = [
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
] as const;

export const IPC_EVENTS = [
  "session:state",
  "transcript:partial",
  "transcript:segment",
  "transcript:final",
  "postprocess:done",
  "inject:done",
  "audio:level",
  "language:detected",
  "model:download:progress",
  "app:error",
] as const;

export type IpcCommand = (typeof IPC_COMMANDS)[number];
export type IpcEvent = (typeof IPC_EVENTS)[number];

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export function updateSettings(patch: Partial<Settings>): Promise<Settings> {
  return invoke("update_settings", { patch });
}

export function setApiKey(provider: "anthropic" | "deepgram", value: string): Promise<void> {
  return invoke("set_api_key", { provider, value });
}

export function hasApiKey(provider: "anthropic" | "deepgram"): Promise<boolean> {
  return invoke("has_api_key", { provider });
}

export function deleteApiKey(provider: "anthropic" | "deepgram"): Promise<void> {
  return invoke("delete_api_key", { provider });
}

export function startDictation(): Promise<void> {
  return invoke("start_dictation");
}

export function stopDictation(): Promise<void> {
  return invoke("stop_dictation");
}

export function cancelDictation(): Promise<void> {
  return invoke("cancel_dictation");
}

export function listModels(): Promise<ModelInfo[]> {
  return invoke("list_models");
}

export function downloadModel(id: string): Promise<void> {
  return invoke("download_model", { id });
}

export function cancelDownload(id: string): Promise<void> {
  return invoke("cancel_download", { id });
}

export function deleteModel(id: string): Promise<void> {
  return invoke("delete_model", { id });
}

export function listDictations(query: ListDictationsQuery): Promise<DictationRow[]> {
  return invoke("list_dictations", { query });
}

export function getDictation(id: string): Promise<DictationRow> {
  return invoke("get_dictation", { id });
}

export function deleteDictation(id: string): Promise<void> {
  return invoke("delete_dictation", { id });
}

export function clearHistory(): Promise<number> {
  return invoke("clear_history");
}

export function reprocessDictation(input: ReprocessOptions): Promise<{ text: string }> {
  return invoke("reprocess_dictation", { input });
}

export function getAudioUrl(id: string): Promise<string> {
  return invoke("get_audio_url", { id });
}

export function listDictionaryEntry(query?: string): Promise<unknown[]> {
  return invoke("list_dictionary_entry", { query });
}

export function addDictionaryEntry(entry: Record<string, unknown>): Promise<unknown> {
  return invoke("add_dictionary_entry", { entry });
}

export function updateDictionaryEntry(
  id: string,
  entry: Record<string, unknown>,
): Promise<unknown> {
  return invoke("update_dictionary_entry", { id, entry });
}

export function deleteDictionaryEntry(id: string): Promise<void> {
  return invoke("delete_dictionary_entry", { id });
}

export function listSnippet(query?: string): Promise<unknown[]> {
  return invoke("list_snippet", { query });
}

export function addSnippet(snippet: Record<string, unknown>): Promise<unknown> {
  return invoke("add_snippet", { snippet });
}

export function updateSnippet(id: string, snippet: Record<string, unknown>): Promise<unknown> {
  return invoke("update_snippet", { id, snippet });
}

export function deleteSnippet(id: string): Promise<void> {
  return invoke("delete_snippet", { id });
}

export function listCustomPrompt(query?: string): Promise<unknown[]> {
  return invoke("list_custom_prompt", { query });
}

export function addCustomPrompt(prompt: Record<string, unknown>): Promise<unknown> {
  return invoke("add_custom_prompt", { prompt });
}

export function updateCustomPrompt(id: string, prompt: Record<string, unknown>): Promise<unknown> {
  return invoke("update_custom_prompt", { id, prompt });
}

export function deleteCustomPrompt(id: string): Promise<void> {
  return invoke("delete_custom_prompt", { id });
}

export function listAppRule(query?: string): Promise<unknown[]> {
  return invoke("list_app_rule", { query });
}

export function addAppRule(rule: Record<string, unknown>): Promise<unknown> {
  return invoke("add_app_rule", { rule });
}

export function updateAppRule(id: string, rule: Record<string, unknown>): Promise<unknown> {
  return invoke("update_app_rule", { id, rule });
}

export function deleteAppRule(id: string): Promise<void> {
  return invoke("delete_app_rule", { id });
}

export function listPersonas(): Promise<PersonaSummary[]> {
  return invoke("list_personas");
}

export function listTemplates(): Promise<TemplateSummary[]> {
  return invoke("list_templates");
}

export function listInputDevices(): Promise<InputDevice[]> {
  return invoke("list_input_devices");
}

export function testInjection(sample: string): Promise<InjectionTestResult> {
  return invoke("test_injection", { sample });
}

export function checkPermissions(): Promise<PermissionSnapshot> {
  return invoke("check_permissions");
}

export function openPermissionPane(kind: string): Promise<void> {
  return invoke("open_permission_pane", { kind });
}

export function exportHistory(destPath: string): Promise<number> {
  return invoke("export_history", { destPath });
}

export function getAppVersion(): Promise<string> {
  return invoke("get_app_version");
}
