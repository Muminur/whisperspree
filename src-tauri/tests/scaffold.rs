//! T0.1 — Tauri scaffold contract (RED phase).
//!
//! These integration tests are written **before** the scaffold exists
//! (PRD §17.3, TDD). They read the config files the implementer must
//! create and assert on their content. Until those files exist, every test
//! must fail with a clear "…missing — scaffold not implemented (T0.1)"
//! panic (a missing artifact, NOT a compile error).
//!
//! PRD refs: §4.1 (app shell), §4.4 (app-data / Info.plist), FR-5.2 (§6,
//! HUD + tray windows), §11 (repo layout), §15.2 (GATE), §15.3 (T0.1 row),
//! §17.5 (CI). See OPEN_QUESTIONS.md Q2 (CI spec is §17.5) and Q3 (T0.1 sets
//! HUD *declarative* flags only — native click-through is T2.4).
//!
//! Real interfaces only: real files on disk parsed with real serde_json.
//! No mocks, no network, no new dependencies.

use std::fs;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// `src-tauri/` — the crate manifest directory.
fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Repository root (parent of `src-tauri/`).
fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("T0.1: src-tauri must live inside the repo root")
        .to_path_buf()
}

/// Read + parse `src-tauri/tauri.conf.json`, panicking with a requirement-named
/// message if it is missing or invalid.
fn load_tauri_conf() -> serde_json::Value {
    let path = manifest_dir().join("tauri.conf.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "src-tauri/tauri.conf.json missing — scaffold not implemented \
             (T0.1, FR-5.2 windows must be declared): {e}"
        )
    });
    serde_json::from_str(&raw)
        .expect("T0.1: src-tauri/tauri.conf.json exists but is not valid JSON")
}

/// Find a window entry by `label` in `app.windows[]`.
fn window<'a>(conf: &'a serde_json::Value, label: &str) -> &'a serde_json::Value {
    conf["app"]["windows"]
        .as_array()
        .expect("FR-5.2 / T0.1: tauri.conf.json `app.windows` must be an array")
        .iter()
        .find(|w| w["label"].as_str() == Some(label))
        .unwrap_or_else(|| panic!("FR-5.2 / T0.1: window '{label}' missing from app.windows"))
}

/// Assert a window boolean flag equals `expected`.
fn assert_bool_flag(w: &serde_json::Value, label: &str, key: &str, expected: bool, req: &str) {
    let actual = &w[key];
    assert_eq!(
        actual,
        &serde_json::Value::Bool(expected),
        "{req} / T0.1: window '{label}' must set {key}={expected} (got {actual})"
    );
}

// ---------------------------------------------------------------------------
// tauri.conf.json — windows (FR-5.2)
// ---------------------------------------------------------------------------

/// FR-5.2 / §11: the four v1 windows are declared.
#[test]
fn scaffold_tauri_conf_declares_four_windows() {
    let conf = load_tauri_conf();
    let windows = conf["app"]["windows"]
        .as_array()
        .expect("FR-5.2 / T0.1: tauri.conf.json `app.windows` must be an array");
    let labels: Vec<&str> = windows.iter().filter_map(|w| w["label"].as_str()).collect();
    for expected in ["hud", "settings", "history", "onboarding"] {
        assert!(
            labels.contains(&expected),
            "FR-5.2 / T0.1: app.windows must declare a '{expected}' window (found {labels:?})"
        );
    }
}

/// FR-5.2: HUD is transparent, non-activating, always-on-top, all-Spaces,
/// task-bar-hidden and starts hidden; macOS transparency needs the private API.
/// (Q3: declarative flags only in T0.1; native click-through is T2.4.)
#[test]
fn scaffold_hud_window_flags_per_fr_5_2() {
    let conf = load_tauri_conf();
    let hud = window(&conf, "hud");
    assert_bool_flag(hud, "hud", "transparent", true, "FR-5.2");
    assert_bool_flag(hud, "hud", "decorations", false, "FR-5.2");
    assert_bool_flag(hud, "hud", "focusable", false, "FR-5.2");
    assert_bool_flag(hud, "hud", "alwaysOnTop", true, "FR-5.2");
    assert_bool_flag(hud, "hud", "visibleOnAllWorkspaces", true, "FR-5.2");
    assert_bool_flag(hud, "hud", "skipTaskbar", true, "FR-5.2");
    assert_bool_flag(hud, "hud", "visible", false, "FR-5.2");

    assert_eq!(
        &conf["app"]["macOSPrivateApi"],
        &serde_json::Value::Bool(true),
        "FR-5.2 / T0.1: app.macOSPrivateApi must be true for HUD transparency on macOS"
    );
}

/// FR-5.2 / §5.4: Settings, History and Onboarding windows start hidden and are
/// shown on demand.
#[test]
fn scaffold_secondary_windows_hidden_at_start() {
    let conf = load_tauri_conf();
    for label in ["settings", "history", "onboarding"] {
        let w = window(&conf, label);
        assert_bool_flag(w, label, "visible", false, "FR-5.2");
    }
}

// ---------------------------------------------------------------------------
// tauri.conf.json — security & bundle
// ---------------------------------------------------------------------------

/// §12 privacy: the app CSP must lock the default source to 'self'.
#[test]
fn scaffold_csp_default_src_self() {
    let conf = load_tauri_conf();
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("§12 / T0.1: app.security.csp must be a string");
    assert!(
        csp.contains("default-src 'self'"),
        "§12 / T0.1: CSP must contain `default-src 'self'` (got: {csp})"
    );
}

/// §3 / §4.1: v1 targets macOS 13+.
#[test]
fn scaffold_bundle_targets_macos_13() {
    let conf = load_tauri_conf();
    let v = &conf["bundle"]["macOS"]["minimumSystemVersion"];
    assert_eq!(
        v,
        &serde_json::Value::String("13.0".to_string()),
        "§3/§4.1 / T0.1: bundle.macOS.minimumSystemVersion must be \"13.0\" (got {v})"
    );
}

// ---------------------------------------------------------------------------
// Info.plist — §4.4 / FR-5.2
// ---------------------------------------------------------------------------

/// §4.4 / FR-5.2: menu-bar-only app (`LSUIElement` = true) and a non-empty
/// microphone usage string. Whitespace-tolerant, regex-free string scanning.
#[test]
fn scaffold_info_plist_has_lsuielement_and_mic_usage() {
    let plist = fs::read_to_string(manifest_dir().join("Info.plist")).unwrap_or_else(|e| {
        panic!(
            "src-tauri/Info.plist missing — scaffold not implemented \
             (T0.1, §4.4 / FR-5.2): {e}"
        )
    });

    // LSUIElement -> <true/> (allowing arbitrary whitespace between the tags).
    let key = "<key>LSUIElement</key>";
    let idx = plist.find(key).unwrap_or_else(|| {
        panic!("§4.4/FR-5.2 / T0.1: Info.plist must set {key} so the app has no Dock icon")
    });
    let after = plist[idx + key.len()..].trim_start();
    assert!(
        after.starts_with("<true/>"),
        "§4.4/FR-5.2 / T0.1: LSUIElement must be followed by <true/> (menu-bar-only app)"
    );

    // NSMicrophoneUsageDescription -> non-empty <string>…</string> value.
    let mic = "<key>NSMicrophoneUsageDescription</key>";
    let midx = plist.find(mic).unwrap_or_else(|| {
        panic!("§4.4/FR-5.2 / T0.1: Info.plist must declare {mic} for the mic permission prompt")
    });
    let after_mic = plist[midx + mic.len()..].trim_start();
    let open = "<string>";
    assert!(
        after_mic.starts_with(open),
        "§4.4 / T0.1: NSMicrophoneUsageDescription must be followed by a <string> value"
    );
    let rest = &after_mic[open.len()..];
    let close = rest
        .find("</string>")
        .expect("§4.4 / T0.1: NSMicrophoneUsageDescription <string> value is not closed");
    assert!(
        !rest[..close].trim().is_empty(),
        "§4.4 / T0.1: NSMicrophoneUsageDescription must be a non-empty usage string"
    );
}

// ---------------------------------------------------------------------------
// single-instance plugin — §4.1
// ---------------------------------------------------------------------------

/// §4.1: the single-instance plugin is depended on and registered.
#[test]
fn scaffold_single_instance_plugin_registered() {
    let lib = fs::read_to_string(manifest_dir().join("src/lib.rs"))
        .expect("T0.1: src-tauri/src/lib.rs must exist");
    let cargo = fs::read_to_string(manifest_dir().join("Cargo.toml"))
        .expect("T0.1: src-tauri/Cargo.toml must exist");
    assert!(
        lib.contains("tauri_plugin_single_instance::init"),
        "§4.1 / T0.1: lib.rs must register the single-instance plugin \
         (tauri_plugin_single_instance::init)"
    );
    assert!(
        cargo.contains("tauri-plugin-single-instance"),
        "§4.1 / T0.1: Cargo.toml must depend on tauri-plugin-single-instance"
    );
}

/// PRD §4.1 / FR-1.3: toggle accelerators must use the official global-shortcut
/// plugin in the same Tauri builder as the single-instance guard.
#[test]
fn scaffold_global_shortcut_plugin_registered() {
    let lib = fs::read_to_string(manifest_dir().join("src/lib.rs"))
        .expect("T2.1: src-tauri/src/lib.rs must exist");
    let cargo = fs::read_to_string(manifest_dir().join("Cargo.toml"))
        .expect("T2.1: src-tauri/Cargo.toml must exist");
    assert!(
        lib.contains("tauri_plugin_global_shortcut"),
        "PRD §4.1 / T2.1: lib.rs must register the global-shortcut plugin"
    );
    assert!(
        cargo.contains("tauri-plugin-global-shortcut"),
        "PRD §4.1 / T2.1: Cargo.toml must depend on tauri-plugin-global-shortcut"
    );
}

// ---------------------------------------------------------------------------
// CI workflow — §17.5 (Q2) / GATE §15.2
// ---------------------------------------------------------------------------

/// §17.5 (Q2): CI runs the full §15.2 GATE on a macos-14 runner, fail-fast off.
#[test]
fn scaffold_ci_workflow_runs_gate_on_macos14() {
    let path = repo_root().join(".github/workflows/ci.yml");
    let yml = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            ".github/workflows/ci.yml missing — scaffold not implemented \
             (T0.1, §17.5): {e}"
        )
    });
    for needle in [
        "macos-14",
        "pnpm typecheck",
        "pnpm test",
        "cargo fmt",
        "--check",
        "cargo clippy",
        "-D warnings",
        "cargo test",
        "fail-fast: false",
    ] {
        assert!(
            yml.contains(needle),
            "§17.5/§15.2 GATE / T0.1: ci.yml must contain `{needle}`"
        );
    }
}

// ---------------------------------------------------------------------------
// Vite dev server — §4.1
// ---------------------------------------------------------------------------

/// §4.1: Vite serves the frontend on the fixed Tauri dev port 1420 with
/// strictPort so the shell's devUrl always matches.
#[test]
fn scaffold_vite_dev_server_port_1420() {
    let path = repo_root().join("vite.config.ts");
    let cfg = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("vite.config.ts missing — scaffold not implemented (T0.1, §4.1): {e}")
    });
    assert!(
        cfg.contains("1420"),
        "§4.1 / T0.1: Vite must serve Tauri on port 1420"
    );
    assert!(
        cfg.contains("strictPort"),
        "§4.1 / T0.1: Vite must set strictPort so Tauri's fixed devUrl matches"
    );
}

#[test]
fn scaffold_autostart_plugin_registered() {
    let lib = fs::read_to_string(manifest_dir().join("src/lib.rs"))
        .expect("T2.6: src-tauri/src/lib.rs must exist");
    let cargo = fs::read_to_string(manifest_dir().join("Cargo.toml"))
        .expect("T2.6: src-tauri/Cargo.toml must exist");
    assert!(
        lib.contains("tauri_plugin_autostart"),
        "PRD §4.1 / T2.6: lib.rs must register the autostart plugin"
    );
    assert!(
        cargo.contains("tauri-plugin-autostart"),
        "PRD §4.1 / T2.6: Cargo.toml must declare tauri-plugin-autostart"
    );
}
