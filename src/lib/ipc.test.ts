// T0.5 — TypeScript IPC contract tests (PRD §9.1–§9.2).
//
// These tests intentionally mock only Tauri's native IPC boundary. They prove
// that the thin frontend mirror sends the exact Rust command name and argument
// object, then returns the boundary promise unchanged for both outcomes.

import { describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import * as ipc from "./ipc";

type WrapperCase = {
  requirement: string;
  call: () => Promise<unknown>;
  invokeArgs: readonly unknown[];
};

const entry = { spoken: "whisper spree", written: "WhisperSpree" };
const snippet = { trigger: "standup", body: "Yesterday: {{yesterday}}" };
const prompt = { name: "Concise", body: "Keep answers concise." };
const rule = { bundleId: "com.apple.TextEdit", styleId: "default" };

const WRAPPER_CASES: readonly WrapperCase[] = [
  { requirement: "fr_0_5_get_settings", call: () => ipc.getSettings(), invokeArgs: ["get_settings"] },
  {
    requirement: "fr_0_5_update_settings_deep_merge_patch",
    call: () => ipc.updateSettings({ mode: "local" }),
    invokeArgs: ["update_settings", { patch: { mode: "local" } }],
  },
  {
    requirement: "fr_0_5_set_api_key_provider_and_value",
    call: () => ipc.setApiKey("anthropic", "stored-by-native-keychain"),
    invokeArgs: ["set_api_key", { provider: "anthropic", value: "stored-by-native-keychain" }],
  },
  {
    requirement: "fr_0_5_has_api_key_provider",
    call: () => ipc.hasApiKey("deepgram"),
    invokeArgs: ["has_api_key", { provider: "deepgram" }],
  },
  {
    requirement: "fr_0_5_delete_api_key_provider",
    call: () => ipc.deleteApiKey("anthropic"),
    invokeArgs: ["delete_api_key", { provider: "anthropic" }],
  },
  { requirement: "fr_0_5_start_dictation", call: () => ipc.startDictation(), invokeArgs: ["start_dictation"] },
  { requirement: "fr_0_5_stop_dictation", call: () => ipc.stopDictation(), invokeArgs: ["stop_dictation"] },
  { requirement: "fr_0_5_cancel_dictation", call: () => ipc.cancelDictation(), invokeArgs: ["cancel_dictation"] },
  { requirement: "fr_0_5_list_models", call: () => ipc.listModels(), invokeArgs: ["list_models"] },
  {
    requirement: "fr_0_5_download_model_id",
    call: () => ipc.downloadModel("tiny"),
    invokeArgs: ["download_model", { id: "tiny" }],
  },
  {
    requirement: "fr_0_5_cancel_download_id",
    call: () => ipc.cancelDownload("tiny"),
    invokeArgs: ["cancel_download", { id: "tiny" }],
  },
  {
    requirement: "fr_0_5_delete_model_id",
    call: () => ipc.deleteModel("tiny"),
    invokeArgs: ["delete_model", { id: "tiny" }],
  },
  {
    requirement: "fr_0_5_list_dictations_optional_query",
    call: () => ipc.listDictations({ q: "meeting", limit: 25, beforeId: "older-row" }),
    invokeArgs: ["list_dictations", { query: { q: "meeting", limit: 25, beforeId: "older-row" } }],
  },
  {
    requirement: "fr_0_5_get_dictation_id",
    call: () => ipc.getDictation("dictation-1"),
    invokeArgs: ["get_dictation", { id: "dictation-1" }],
  },
  {
    requirement: "fr_0_5_delete_dictation_id",
    call: () => ipc.deleteDictation("dictation-1"),
    invokeArgs: ["delete_dictation", { id: "dictation-1" }],
  },
  { requirement: "fr_0_5_clear_history", call: () => ipc.clearHistory(), invokeArgs: ["clear_history"] },
  {
    requirement: "fr_0_5_reprocess_dictation_input",
    call: () => ipc.reprocessDictation("dictation-1", "template", "email"),
    invokeArgs: ["reprocess_dictation", { input: { id: "dictation-1", kind: "template", refId: "email" } }],
  },
  {
    requirement: "fr_0_5_get_audio_url_id",
    call: () => ipc.getAudioUrl("dictation-1"),
    invokeArgs: ["get_audio_url", { id: "dictation-1" }],
  },
  {
    requirement: "fr_0_5_list_dictionary_entry_undefined_query",
    call: () => ipc.listDictionaryEntry(),
    invokeArgs: ["list_dictionary_entry", { query: undefined }],
  },
  {
    requirement: "fr_0_5_add_dictionary_entry",
    call: () => ipc.addDictionaryEntry(entry),
    invokeArgs: ["add_dictionary_entry", { entry }],
  },
  {
    requirement: "fr_0_5_update_dictionary_entry",
    call: () => ipc.updateDictionaryEntry("entry-1", entry),
    invokeArgs: ["update_dictionary_entry", { id: "entry-1", entry }],
  },
  {
    requirement: "fr_0_5_delete_dictionary_entry",
    call: () => ipc.deleteDictionaryEntry("entry-1"),
    invokeArgs: ["delete_dictionary_entry", { id: "entry-1" }],
  },
  {
    requirement: "fr_0_5_list_snippet_undefined_query",
    call: () => ipc.listSnippet(),
    invokeArgs: ["list_snippet", { query: undefined }],
  },
  {
    requirement: "fr_0_5_add_snippet",
    call: () => ipc.addSnippet(snippet),
    invokeArgs: ["add_snippet", { snippet }],
  },
  {
    requirement: "fr_0_5_update_snippet",
    call: () => ipc.updateSnippet("snippet-1", snippet),
    invokeArgs: ["update_snippet", { id: "snippet-1", snippet }],
  },
  {
    requirement: "fr_0_5_delete_snippet",
    call: () => ipc.deleteSnippet("snippet-1"),
    invokeArgs: ["delete_snippet", { id: "snippet-1" }],
  },
  {
    requirement: "fr_0_5_list_custom_prompt_undefined_query",
    call: () => ipc.listCustomPrompt(),
    invokeArgs: ["list_custom_prompt", { query: undefined }],
  },
  {
    requirement: "fr_0_5_add_custom_prompt",
    call: () => ipc.addCustomPrompt(prompt),
    invokeArgs: ["add_custom_prompt", { prompt }],
  },
  {
    requirement: "fr_0_5_update_custom_prompt",
    call: () => ipc.updateCustomPrompt("prompt-1", prompt),
    invokeArgs: ["update_custom_prompt", { id: "prompt-1", prompt }],
  },
  {
    requirement: "fr_0_5_delete_custom_prompt",
    call: () => ipc.deleteCustomPrompt("prompt-1"),
    invokeArgs: ["delete_custom_prompt", { id: "prompt-1" }],
  },
  {
    requirement: "fr_0_5_list_app_rule_undefined_query",
    call: () => ipc.listAppRule(),
    invokeArgs: ["list_app_rule", { query: undefined }],
  },
  {
    requirement: "fr_0_5_add_app_rule",
    call: () => ipc.addAppRule(rule),
    invokeArgs: ["add_app_rule", { rule }],
  },
  {
    requirement: "fr_0_5_update_app_rule",
    call: () => ipc.updateAppRule("rule-1", rule),
    invokeArgs: ["update_app_rule", { id: "rule-1", rule }],
  },
  {
    requirement: "fr_0_5_delete_app_rule",
    call: () => ipc.deleteAppRule("rule-1"),
    invokeArgs: ["delete_app_rule", { id: "rule-1" }],
  },
  { requirement: "fr_0_5_list_personas", call: () => ipc.listPersonas(), invokeArgs: ["list_personas"] },
  { requirement: "fr_0_5_list_templates", call: () => ipc.listTemplates(), invokeArgs: ["list_templates"] },
  { requirement: "fr_0_5_list_input_devices", call: () => ipc.listInputDevices(), invokeArgs: ["list_input_devices"] },
  {
    requirement: "fr_0_5_test_injection_sample",
    call: () => ipc.testInjection("test injection"),
    invokeArgs: ["test_injection", { sample: "test injection" }],
  },
  { requirement: "fr_0_5_check_permissions", call: () => ipc.checkPermissions(), invokeArgs: ["check_permissions"] },
  {
    requirement: "fr_0_5_open_permission_pane_kind",
    call: () => ipc.openPermissionPane("microphone"),
    invokeArgs: ["open_permission_pane", { kind: "microphone" }],
  },
  {
    requirement: "fr_0_5_export_history_dest_path",
    call: () => ipc.exportHistory("/tmp/export.jsonl"),
    invokeArgs: ["export_history", { destPath: "/tmp/export.jsonl" }],
  },
  { requirement: "fr_0_5_get_app_version", call: () => ipc.getAppVersion(), invokeArgs: ["get_app_version"] },
];

describe("IPC mirror (PRD §9.1–§9.2)", () => {
  for (const wrapper of WRAPPER_CASES) {
    it(`${wrapper.requirement}_forwards_tauri_result_and_error`, async () => {
      const response = { accepted: wrapper.requirement };
      invoke.mockReset();
      invoke.mockResolvedValueOnce(response);

      await expect(wrapper.call()).resolves.toBe(response);
      expect(invoke).toHaveBeenCalledExactlyOnceWith(...wrapper.invokeArgs);

      const failure = new Error("native IPC rejected");
      invoke.mockReset();
      invoke.mockRejectedValueOnce(failure);

      await expect(wrapper.call()).rejects.toBe(failure);
      expect(invoke).toHaveBeenCalledExactlyOnceWith(...wrapper.invokeArgs);
    });
  }

  it("fr_0_5_command_inventory_matches_the_registered_frontend_contract", () => {
    expect(ipc.IPC_COMMANDS).toEqual(WRAPPER_CASES.map(({ invokeArgs }) => invokeArgs[0]));
  });

  it("fr_0_5_event_inventory_matches_prd_9_2", () => {
    expect(ipc.IPC_EVENTS).toEqual([
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
    ]);
  });
});
