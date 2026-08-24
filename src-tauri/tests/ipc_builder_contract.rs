//! T0.5 — production/shared IPC builder and P-9 registry contracts.

use std::{collections::BTreeSet, fs, path::PathBuf};

use whisperspree_lib::ipc::commands;

fn source(path: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect("required production IPC source must exist")
}

#[test]
fn fr_0_5_production_builder_uses_real_state_dependencies_and_preserves_single_instance_first() {
    let lib = source("src/lib.rs");
    let common_harness = source("tests/common/mod.rs");
    let builder = lib
        .find("let builder = tauri::Builder::default()")
        .expect("run() must retain an explicit production builder");
    let plugin = lib
        .find(".plugin(tauri_plugin_single_instance::init")
        .expect("single-instance must remain the first plugin");
    let state = lib
        .find("let state = ipc::commands::IpcState::new(")
        .expect("run() must construct one managed IPC state");
    let configure = lib
        .find("ipc::configure_ipc(builder, state)")
        .expect("run() must pass that state to the shared IPC helper");
    assert_eq!(
        lib.matches("let state = ipc::commands::IpcState::new(")
            .count(),
        1,
        "run() must construct exactly one state object from real dependencies"
    );
    assert_eq!(
        lib.matches("ipc::configure_ipc(builder, state)").count(),
        1,
        "run() must configure the production builder through the shared helper exactly once"
    );
    let state_statement = &lib[state..configure];
    let compact_state_statement = state_statement
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact_state_statement.contains(
            "letstate=ipc::commands::IpcState::new(SettingsStore::new(store::app_data_dir()),Arc::new(KeyringStore));"
        ),
        "the sole production IpcState must be constructed directly from the real settings and keychain dependencies"
    );
    assert!(builder < plugin && lib[..plugin].matches(".plugin(").count() == 0);
    assert!(plugin < configure && state < configure);
    assert!(!lib.contains(".invoke_handler("));

    assert_eq!(
        common_harness
            .matches("whisperspree_lib::ipc::configure_ipc(")
            .count(),
        1,
        "MockRuntime must configure its app through the same shared helper"
    );
    assert!(
        !common_harness.contains(".invoke_handler("),
        "the canonical MockRuntime harness must not define a test-only command registry"
    );
}

#[test]
fn fr_0_5_shared_builder_manages_one_ipc_state_before_single_handler_registry() {
    let ipc_mod = source("src/ipc/mod.rs");
    let manage = ipc_mod
        .find(".manage(state)")
        .expect("the shared builder must manage its supplied IpcState");
    let invoke_handler = ipc_mod
        .find(".invoke_handler(")
        .expect("the shared builder must install the canonical handler registry");
    assert_eq!(ipc_mod.matches(".manage(state)").count(), 1);
    assert_eq!(ipc_mod.matches(".invoke_handler(").count(), 1);
    assert!(
        manage < invoke_handler,
        "managed IpcState must be installed before Tauri receives the command registry"
    );
}

#[test]
fn fr_1_1_production_state_selects_platform_runtime_factory() {
    let commands = source("src/ipc/commands.rs");
    let pipeline = source("src/pipeline/session.rs");
    assert!(
        commands.contains("fn default_dictation_runtime(")
            && pipeline.contains("CoordinatorRuntime::new")
            && pipeline.contains("CpalMicrophone::new"),
        "production IPC state must select the real capture/coordinator runtime on supported platforms"
    );
}

#[test]
fn fr_1_1_production_session_runtime_drains_and_installs_event_sink() {
    let session = source("src/pipeline/session.rs");
    let shell = source("src/lib.rs");
    assert!(
        session.contains("drain_events(&mut current, event_sink.as_ref())")
            && session.contains("sink.emit_task(&active.session_id, event)")
            && shell.contains("state.install_event_sink(Arc::new(TauriSessionEventSink"),
        "production session events must be drained through the installed Tauri sink"
    );
}

#[test]
fn fr_1_1_toggle_stop_publishes_lifecycle_and_drains_session_events() {
    let session = source("src/pipeline/session.rs");
    let toggle = session
        .find("Command::Toggle(reply)")
        .expect("production runtime must retain the toggle command");
    let branch = &session[toggle..];
    let stop = branch
        .find("if active.is_some()")
        .expect("toggle must have an active-session stop branch");
    let start = &branch[stop..];
    let next_command = start
        .find("Command::SetEventSink")
        .expect("toggle branch must end before the next command arm");
    let stop_branch = &start[..next_command];
    assert!(stop_branch.contains("SessionState::Finalizing"));
    assert!(stop_branch.contains("drain_events"));
    assert!(stop_branch.contains("finalized_control"));
    assert!(stop_branch.contains("SessionState::PostProcessing"));
    assert!(stop_branch.contains("SessionState::Idle"));
}

#[test]
fn ec_1_1_poll_failure_aborts_session_and_publishes_error_then_idle() {
    let session = source("src/pipeline/session.rs");
    assert!(session.contains("Err(error) => {") && session.contains("current.session.fail(error)"));
    let failure = session
        .find("current.session.fail(error)")
        .expect("poll failure must abort the active session task");
    let tail = &session[failure..];
    assert!(tail.contains("SessionState::Error"));
    assert!(tail.contains("SessionState::Idle"));
    assert!(tail.contains("coordinator.reset_control()"));
}

#[test]
fn p3_all_production_error_warnings_include_stable_error_codes() {
    let hotkey = source("src/hotkey/mod.rs");
    let session = source("src/pipeline/session.rs");
    assert!(hotkey.contains("code = %error.code()"));
    assert!(session.matches("code = %error.code()").count() >= 4);
}

#[test]
fn fr_1_1_runtime_uses_one_coordinator_owned_session_id_for_events() {
    let session = source("src/pipeline/session.rs");
    assert!(session.contains("start_control_with_id"));
    assert!(!session.contains("local-{next_session_id}"));
}

#[test]
fn p3_bootstrap_and_background_error_paths_log_stable_codes() {
    let lib = source("src/lib.rs");
    let commands = source("src/ipc/commands.rs");
    assert!(lib.contains("code = %error.code"));
    assert!(commands.contains("code = %error.code()"));
}

#[test]
fn p3_os_boundary_failures_are_coded_and_observable() {
    let hotkey = source("src/hotkey/mod.rs");
    let inject = source("src/inject/mod.rs");
    let history = source("src/store/history.rs");
    assert!(hotkey.contains("code = \"HK-PERM\""));
    assert!(inject.contains("code = \"INJ-FAIL\""));
    assert!(history.contains("code = \"DB-IO\""));
}

#[test]
fn fr_1_3_production_state_starts_hotkey_listener_and_action_bridge() {
    let state = source("src/state.rs");
    assert!(
        state.contains("spawn_action_bridge"),
        "production state construction must start the shared hotkey action bridge"
    );
    assert!(
        state.contains("spawn_rdev_listener_with_control"),
        "production state construction must start the rdev listener/control manager"
    );
}

#[test]
fn fr_1_3_production_hotkey_listener_starts_optimistically_and_degrades_on_probe_failure() {
    let state = source("src/state.rs");
    let hotkey = source("src/hotkey/mod.rs");
    assert!(
        state.contains("spawn_rdev_listener_with_control(")
            && state.contains("PermissionState::Granted")
            && hotkey.contains("HotkeyControl::SetPermission(PermissionState::Denied)"),
        "the listener must accept granted Input Monitoring on startup and retain the denial path when rdev reports failure"
    );
}

#[test]
fn fr_1_3_toggle_registration_reads_persisted_hotkey_combo() {
    let lib = source("src/lib.rs");
    assert!(
        lib.contains("settings.hotkey.toggle_combo") && lib.contains("on_shortcut(")
            && !lib.contains("on_shortcut(\n                \"CTRL+ALT+SPACE\""),
        "global shortcut registration must use the persisted toggle combo rather than a hard-coded accelerator"
    );
}

#[test]
fn fr_1_3_global_shortcut_routes_toggle_through_hotkey_policy_manager() {
    let lib = source("src/lib.rs");
    assert!(
        lib.contains("HotkeyControl::Toggle(Instant::now())")
            && lib.contains("hotkey_controller.clone()"),
        "the global shortcut callback must feed the hotkey policy manager rather than calling the runtime directly"
    );
    assert!(
        !lib.contains("state_for_shortcut.toggle_dictation()"),
        "the global shortcut callback must not bypass debounce and mode policy"
    );
}

#[test]
fn fr_1_3_session_event_bridge_reports_idle_and_speech_to_hotkey_policy() {
    let session = source("src/pipeline/session.rs");
    let shell = source("src/lib.rs");
    assert!(session.contains("fn session_became_idle") && session.contains("fn speech_observed"));
    assert!(
        shell.contains("HotkeyControl::SessionBecameIdle")
            && shell.contains("mark_speech_observed"),
        "production event sink must feed coordinator lifecycle signals back to hotkey policy"
    );
}

#[test]
fn fr_1_3_live_rebind_callbacks_use_hotkey_controller_and_runtime_silent_cancel_exists() {
    let commands = source("src/ipc/commands.rs");
    let session = source("src/pipeline/session.rs");
    assert!(
        commands.contains("HotkeyControl::Toggle(Instant::now())")
            && commands.matches("runtime.toggle_dictation()").count() == 1,
        "live shortcut registrations must retain the policy-manager path"
    );
    assert!(
        session.contains("fn cancel_silent_dictation(&self)")
            && session.matches("fn cancel_silent_dictation(&self)").count() >= 2,
        "the production session runtime must implement silent accidental-tap cancellation"
    );
    assert!(
        session.matches("task.set_speech_observer").count() >= 3,
        "explicit Start, Toggle start, AND the EC-1.1 local-fallback continuation \
         must each install synchronous VAD feedback"
    );
}

#[test]
fn fr_1_3_toggle_registration_failure_rolls_back_persisted_combo() {
    let commands = source("src/ipc/commands.rs");
    assert!(
        commands.contains("could not unregister previous toggle shortcut")
            && commands.matches("\"toggleCombo\": previous_combo").count() >= 2,
        "OS registration failures must restore the previous persisted toggle combo"
    );
}

#[test]
fn fr_5_2_tray_exposes_dictation_modes_windows_pause_and_quit() {
    let lib = source("src/lib.rs");
    for id in [
        "start_dictation",
        "stop_dictation",
        "mode_auto",
        "mode_local",
        "mode_cloud",
        "pause_hotkeys",
        "open_settings",
        "open_history",
        "open_onboarding",
        "quit",
    ] {
        assert!(lib.contains(&format!("\"{id}\"")), "tray is missing {id}");
    }
    assert!(lib.contains("TrayIconBuilder"));
    assert!(lib.contains("on_menu_event"));
}

#[test]
fn fr_1_1_production_local_runtime_reads_and_persists_effective_model() {
    let session = source("src/pipeline/session.rs");
    assert!(session.contains("effective_local_model"));
    assert!(session.contains("set_effective_model_persistence"));
    assert!(session.contains("local_model"));
}

#[test]
fn fr_1_1_production_runtime_shares_settings_owner_and_applies_input_device() {
    let commands = source("src/ipc/commands.rs");
    let session = source("src/pipeline/session.rs");
    let microphone = source("src/pipeline/microphone.rs");
    assert!(commands.contains("let settings = Arc::new(Mutex::new(settings));"));
    // T3.1: the runtime now also receives the managed KeyStore so the cloud
    // selector can probe Deepgram key presence.
    assert!(commands
        .contains("default_dictation_runtime(Arc::clone(&settings), Arc::clone(&key_store))"));
    assert!(session.contains("settings_snapshot.audio.input_device_id.clone()"));
    assert!(session.contains("set_requested_device_id"));
    assert!(microphone.contains("pub fn set_requested_device_id"));
}

#[test]
fn fr_1_1_failure_and_silence_paths_publish_visible_lifecycle_events() {
    let session = source("src/pipeline/session.rs");
    let task = source("src/pipeline/session_task.rs");
    let events = source("src/ipc/events.rs");
    assert!(session.contains("emit_start_failure"));
    assert!(session.contains("SessionState::Error"));
    assert!(task.contains("SessionTaskEvent::SilenceOnly"));
    assert!(events.contains("notice: Some(\"Didn't catch anything\".into())"));
    assert!(session.matches("start_control_with_id()?").count() >= 2);
    assert!(session
        .contains("emit_start_failure(event_sink.as_ref(), start_session_id.as_deref(), error)"));
    assert!(session.contains("frontmost context unavailable; using unknown context"));
}

#[test]
fn fr_1_3_duplicate_runtime_start_does_not_teardown_active_session() {
    let session = source("src/pipeline/session.rs");
    assert!(
        session.contains("let was_active = active.is_some();")
            && session.contains("if result.is_err() && !was_active"),
        "a duplicate start must report the illegal transition without running start-failure cleanup against the live session"
    );
}

#[test]
fn p9_configure_ipc_registers_exact_42_unique_command_paths() {
    let ipc_mod = source("src/ipc/mod.rs");
    assert!(ipc_mod.matches("generate_handler![").count() == 1);

    let entries = ipc_mod
        .split("generate_handler![")
        .nth(1)
        .expect("shared registry must contain generate_handler")
        .split("])")
        .next()
        .expect("shared handler list must close")
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let actual = entries.iter().cloned().collect::<BTreeSet<_>>();
    let expected = commands::IPC_COMMANDS
        .iter()
        .map(|command| format!("ipc::commands::{command}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        commands::IPC_COMMANDS.len(),
        42,
        "P-9 fixes the command allowlist size"
    );
    assert_eq!(
        entries.len(),
        42,
        "shared registry must expose every P-9 command"
    );
    assert_eq!(actual, expected);
}

#[test]
fn t3_1_runtime_start_selects_local_or_cloud_engine_from_settings_and_keychain() {
    let session = source("src/pipeline/session.rs");
    // Both production start paths (explicit Start + toggle-start) must route
    // engine construction through one shared selector: 1 definition + 2 calls.
    let sites = session.matches("select_engine_for_session(").count();
    assert_eq!(
        sites - 1,
        2,
        "exactly the two start paths may construct engines via the shared selector"
    );
    assert!(
        session.contains("engine_from_store(") && session.contains("resolve_mode("),
        "the selector must combine keychain presence with mode resolution"
    );
    let commands = source("src/ipc/commands.rs");
    assert!(
        commands.contains("key_store"),
        "default_dictation_runtime must hand the managed KeyStore to the runtime"
    );
}

#[test]
fn t3_2_session_state_carries_the_selected_engine_to_the_hud_badge() {
    let session = source("src/pipeline/session.rs");
    assert!(
        !session.contains("engine: Some(\"local\""),
        "state emission must report the ACTUAL engine, not a hardcoded local"
    );
    assert!(
        session.contains("engine_label()"),
        "ActiveSession must expose which engine is running"
    );
    let lib = source("src/lib.rs");
    assert!(
        lib.contains("engine: Some(engine.to_string())"),
        "the Tauri sink must forward the runtime-provided engine label"
    );
}
