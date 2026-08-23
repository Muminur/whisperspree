//! `settings.json` store (§8.3): serde model, deep-merge `update_settings`
//! logic, validation (clamp/reject-by-ignore), and atomic persistence.
//!
//! PRD refs:
//! - §8.3 — the exact schema + defaults (quoted field-by-field below) and the
//!   "unknown fields preserved on rewrite" tolerance.
//! - §9.1 `update_settings` — deep-merge, validate, persist, return the result.
//! - §4.4 / §12 P-3 — settings.json never carries API keys/secrets.
//! - §14 / OPEN_QUESTIONS Q8 — write failures map to `DB-IO`; an invalid patch
//!   is not an error (validate clamps/drops, keeping the prior valid value).

use crate::error::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// §8.3 schema (camelCase on the wire; `#[serde(default)]` throughout so unknown
// fields never fail deserialization).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub mode: String,
    pub hotkey: Hotkey,
    pub audio: Audio,
    pub asr: Asr,
    pub postprocess: Postprocess,
    pub translation: Translation,
    pub context: Toggle,
    pub commands: Toggle,
    pub injection: Injection,
    pub history: History,
    pub hud: Hud,
    pub launch_at_login: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Hotkey {
    pub mode: String,
    pub push_to_talk_key: String,
    pub toggle_combo: String,
    pub esc_cancels: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Audio {
    pub input_device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Asr {
    pub local_model: String,
    pub effective_local_model: Option<String>,
    pub cloud_provider: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Postprocess {
    pub enabled: bool,
    pub persona_id: String,
    pub llm_provider: String,
    pub model_fast: String,
    pub model_quality: String,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Translation {
    pub enabled: bool,
    pub target_language: String,
}

/// Shared shape for `context` and `commands` (§8.3: both are `{ "enabled": bool }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Toggle {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Injection {
    pub type_threshold_chars: u32,
    pub restore_clipboard_delay_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct History {
    pub save_text: bool,
    pub retention_days: u32,
    pub save_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Hud {
    pub show_partials: bool,
}

impl Default for Settings {
    /// The exact §8.3 literal defaults. Pinned field-by-field by
    /// `tests::settings_defaults_match_prd_8_3` — see that test for the quoted
    /// PRD source.
    fn default() -> Self {
        Settings {
            version: 1,
            mode: "auto".to_string(),
            hotkey: Hotkey {
                mode: "push_to_talk".to_string(),
                push_to_talk_key: "AltRight".to_string(),
                toggle_combo: "Ctrl+Alt+Space".to_string(),
                esc_cancels: true,
            },
            audio: Audio {
                input_device_id: None,
            },
            asr: Asr {
                local_model: "small".to_string(),
                effective_local_model: None,
                cloud_provider: "deepgram".to_string(),
                language: "auto".to_string(),
            },
            postprocess: Postprocess {
                enabled: true,
                persona_id: "clean".to_string(),
                llm_provider: "anthropic".to_string(),
                model_fast: "claude-haiku-4-5".to_string(),
                model_quality: "claude-sonnet-4-6".to_string(),
                timeout_ms: 6000,
            },
            translation: Translation {
                enabled: false,
                target_language: "en".to_string(),
            },
            context: Toggle { enabled: true },
            commands: Toggle { enabled: true },
            injection: Injection {
                type_threshold_chars: 200,
                restore_clipboard_delay_ms: 600,
            },
            history: History {
                save_text: true,
                retention_days: 30,
                save_audio: false,
            },
            hud: Hud {
                show_partials: true,
            },
            launch_at_login: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Reads/writes `<dir>/settings.json`. `dir` is injected so tests use a
/// `tempfile` tempdir instead of the real `~/Library/Application Support` path
/// (that real path only comes from `store::app_data_dir()` at call sites).
#[derive(Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    /// Read the raw persisted value, or the §8.3 default value if the file is
    /// absent. A parse failure maps to `DB-IO` (Q8-a).
    fn read_raw(&self) -> Result<Value, Error> {
        let path = self.path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| crate::store::db_io("parsing settings.json", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::to_value(Settings::default())
                    .map_err(|e| crate::store::db_io("serializing default settings", e))
            }
            Err(e) => Err(crate::store::db_io("reading settings.json", e)),
        }
    }

    /// §9.1 `get_settings`: read `settings.json`; if absent, the §8.3 defaults
    /// (without writing them). A parse failure maps to `DB-IO` (Q8-a).
    pub fn get(&self) -> Result<Settings, Error> {
        let mut raw = self.read_raw()?;
        validate(&mut raw);
        serde_json::from_value(raw).map_err(|e| crate::store::db_io("deserializing settings", e))
    }

    /// §9.1 `update_settings`: deep-merge `patch` onto the current raw value
    /// (preserving unknown fields), validate (clamp/reject-by-ignore, Q8-c),
    /// persist atomically, and return the resulting `Settings`. A write
    /// failure maps to `DB-IO` (Q8-a).
    pub fn update(&self, patch: Value) -> Result<Settings, Error> {
        let mut base = self.read_raw()?;
        // Sanitize the patch BEFORE merging: an invalid enum/accelerator field is
        // dropped from the patch itself so the merge treats it as an absent key,
        // which `deep_merge` leaves untouched — this is how the prior valid value
        // in `base` survives (OPEN_QUESTIONS Q8-c "reject-by-ignore").
        let mut patch = patch;
        // Unknown settings are forward-compatible, but settings.json is never a
        // secret store (§12 P-3). Strip secret-bearing unknown fields at every
        // depth before merge so a nested object/array cannot smuggle credentials
        // to disk while benign future settings remain intact.
        strip_secret_bearing_fields(&mut patch);
        validate(&mut patch);
        deep_merge(&mut base, &patch);
        // Defense in depth: sanitize the fully merged value too, including any
        // legacy unknown field already present on disk, before it is persisted.
        strip_secret_bearing_fields(&mut base);
        // Clamp/drop pass over the fully merged value (idempotent once the patch
        // above is already sanitized).
        validate(&mut base);
        // Prove the round-trip BEFORE writing anything to disk: a type-invalid
        // patch (e.g. a string where a number belongs) passes `validate` (which
        // only guards enums/numeric floors, not field types) but must never reach
        // `atomic_write` — otherwise a bad patch corrupts the persisted file and
        // every later `get()`/`update()` re-reads the poisoned value forever.
        let settings: Settings = serde_json::from_value(base.clone())
            .map_err(|e| crate::store::db_io("deserializing settings", e))?;
        atomic_write(&self.dir, &self.path(), &base)?;
        Ok(settings)
    }
}

/// Write `value` to `path` atomically: serialize pretty → open a uniquely
/// named `.tmp` file in the SAME directory (so `rename` is atomic on APFS)
/// with `0600` perms applied at creation time (unix), refusing to follow an
/// existing file/symlink at that path → write → `sync_all` → `rename` over
/// `path`. Any I/O failure maps to `DB-IO` (Q8-a); the tmp file is best-effort
/// removed on any failure so a failed write leaves no debris.
fn atomic_write(dir: &std::path::Path, path: &std::path::Path, value: &Value) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|e| crate::store::db_io("creating settings dir", e))?;

    let tmp_path = unique_tmp_path(dir);
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| crate::store::db_io("serializing settings", e))?;

    let write_result: Result<(), Error> = (|| {
        let mut file = open_tmp_file(&tmp_path)?;
        use std::io::Write;
        file.write_all(text.as_bytes())
            .map_err(|e| crate::store::db_io("writing settings.json.tmp", e))?;
        file.sync_all()
            .map_err(|e| crate::store::db_io("fsync settings.json.tmp", e))
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(crate::store::db_io("renaming settings.json.tmp", e));
    }

    Ok(())
}

/// A unique tmp-file path in `dir` (process id + a monotonic per-process
/// counter, so concurrent writers/writes never collide on the same name) —
/// same directory as the target so the final `rename` stays atomic (APFS).
fn unique_tmp_path(dir: &std::path::Path) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("settings.json.{}.{}.tmp", std::process::id(), n))
}

/// Open `tmp_path` for writing: `create_new` so a pre-existing file or symlink
/// at that path is refused (never followed) rather than silently written
/// through, with the final `0600` permission applied atomically at creation
/// time on unix (no window where the content is world-readable).
fn open_tmp_file(tmp_path: &std::path::Path) -> Result<std::fs::File, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(tmp_path)
            .map_err(|e| crate::store::db_io("creating settings.json.tmp", e))
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(tmp_path)
            .map_err(|e| crate::store::db_io("creating settings.json.tmp", e))
    }
}

/// Recursively merge `patch` onto `base`: matching object keys recurse; any
/// other present key (including an explicit `null`) overwrites `base[k]`
/// wholesale (arrays are replaced, never concatenated); absent keys are left
/// untouched (§8.3 "unknown fields preserved on rewrite").
fn deep_merge(base: &mut Value, patch: &Value) {
    let (Value::Object(base_map), Value::Object(patch_map)) = (base, patch) else {
        return;
    };
    for (key, patch_value) in patch_map {
        match base_map.get_mut(key) {
            Some(base_value) if base_value.is_object() && patch_value.is_object() => {
                deep_merge(base_value, patch_value);
            }
            _ => {
                base_map.insert(key.clone(), patch_value.clone());
            }
        }
    }
}

/// Remove keys and string values that are recognizably secret-bearing from a
/// user-supplied settings patch. Objects and arrays are walked recursively so
/// safe siblings of an offending nested field remain available to future app
/// versions.
fn strip_secret_bearing_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, child| {
                if is_secret_bearing_key(key) || contains_secret_value(child) {
                    return false;
                }
                strip_secret_bearing_fields(child);
                true
            });
        }
        Value::Array(values) => {
            values.retain(|child| !contains_secret_value(child));
            values.iter_mut().for_each(strip_secret_bearing_fields);
        }
        _ => {}
    }
}

/// Match complete credential terms after normalizing conventional camelCase
/// and snake_case spellings. This intentionally does not reject unrelated
/// names such as `tokenizerModel`, `secretaryMode`, or `passwordlessMode`.
fn is_secret_bearing_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();

    let mut tokens = Vec::new();
    let chars: Vec<char> = key.chars().collect();
    let mut current = String::new();
    for (index, character) in chars.iter().copied().enumerate() {
        let separator = !character.is_ascii_alphanumeric();
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let camel_boundary = character.is_ascii_uppercase()
            && previous.is_some_and(|p| p.is_ascii_lowercase())
            || character.is_ascii_uppercase()
                && previous.is_some_and(|p| p.is_ascii_uppercase())
                && next.is_some_and(|n| n.is_ascii_lowercase());
        if separator || camel_boundary {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if !separator {
                current.push(character.to_ascii_lowercase());
            }
        } else {
            current.push(character.to_ascii_lowercase());
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    if matches!(
        normalized.as_str(),
        "apikey"
            | "anthropicapikey"
            | "deepgramapikey"
            | "apicredential"
            | "apicredentials"
            | "accesstoken"
            | "authtoken"
            | "bearertoken"
            | "clientsecret"
            | "credential"
            | "credentials"
            | "password"
            | "privatekey"
            | "secret"
            | "token"
            | "authorization"
            | "authorizationheader"
            | "authheader"
    ) {
        return true;
    }

    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "token" | "secret" | "password" | "credential" | "credentials" | "authorization"
        )
    }) || tokens
        .windows(2)
        .any(|pair| matches!(pair, [first, second] if (first == "api" && second == "key") || (first == "pass" && second == "word")))
}

/// Reuse the one P-3 redaction predicate at the persistence boundary. A changed
/// result means this string contains an `sk-ant-…` key or a `Token <value>`
/// credential; ordinary text such as "token usage" remains untouched.
fn contains_secret_value(value: &Value) -> bool {
    let Value::String(text) = value else {
        return false;
    };
    crate::error::redact(text) != *text
}

/// Clamp/reject-by-ignore pass applied to the merged raw value before persist
/// (OPEN_QUESTIONS Q8-c): out-of-range numerics are clamped to their floor;
/// malformed enums/accelerators are dropped, leaving the prior (already-merged)
/// value in place. Unknown keys are left intact.
///
// PRD-QUESTION(Q8): `update_settings` receiving an invalid value is not one of
// the closed §14 codes — it is validation, not an error (see OPEN_QUESTIONS Q8
// option B).
fn validate(v: &mut Value) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };

    reject_enum_field(obj, "mode", &["auto", "local", "cloud"]);

    if let Some(hotkey) = obj.get_mut("hotkey").and_then(|h| h.as_object_mut()) {
        reject_enum_field(hotkey, "mode", &["push_to_talk", "toggle"]);
        reject_non_string_field(hotkey, "pushToTalkKey");
        reject_non_string_field(hotkey, "toggleCombo");
    }

    if let Some(postprocess) = obj.get_mut("postprocess").and_then(|p| p.as_object_mut()) {
        clamp_min_field(postprocess, "timeoutMs", 1);
    }

    if let Some(history) = obj.get_mut("history").and_then(|h| h.as_object_mut()) {
        clamp_min_field(history, "retentionDays", 0);
    }

    if let Some(injection) = obj.get_mut("injection").and_then(|i| i.as_object_mut()) {
        clamp_min_field(injection, "typeThresholdChars", 0);
        clamp_min_field(injection, "restoreClipboardDelayMs", 0);
    }
}

/// Drop `key` from `obj` if present and not one of `allowed` (reject-by-ignore,
/// Q8-c) — a wrong-type value is dropped too. Absent keys are left untouched.
fn reject_enum_field(obj: &mut serde_json::Map<String, Value>, key: &str, allowed: &[&str]) {
    let invalid = match obj.get(key) {
        Some(Value::String(s)) => !allowed.contains(&s.as_str()),
        Some(_) => true,
        None => false,
    };
    if invalid {
        obj.remove(key);
    }
}

/// Drop `key` from `obj` if present and not a JSON string.
fn reject_non_string_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    let invalid = matches!(obj.get(key), Some(value) if !value.is_string());
    if invalid {
        obj.remove(key);
    }
}

/// Clamp a present numeric `key` in `obj` to a floor of `min`.
fn clamp_min_field(obj: &mut serde_json::Map<String, Value>, key: &str, min: i64) {
    let Some(value) = obj.get_mut(key) else {
        return;
    };
    if let Some(n) = value.as_i64() {
        if n < min {
            *value = serde_json::json!(min);
        }
    } else if let Some(f) = value.as_f64() {
        if f < min as f64 {
            *value = serde_json::json!(min);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (colocated so `cargo test settings::` filters to this module).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The §8.3 `settings.json` schema, transcribed verbatim from the PRD JSON
    /// block. Both `settings_defaults_match_prd_8_3` and
    /// `settings_serde_camelcase_matches_prd_8_3` build off THIS single literal
    /// so the pin is against the PRD text, not a second hand-duplicated Rust
    /// struct literal (review NIT fix — the old `expected_defaults()` was
    /// near-tautological, comparing two copies of the same Rust values).
    fn prd_8_3_json() -> Value {
        serde_json::json!({
            "version": 1,
            "mode": "auto",
            "hotkey": { "mode": "push_to_talk", "pushToTalkKey": "AltRight",
                        "toggleCombo": "Ctrl+Alt+Space", "escCancels": true },
            "audio": { "inputDeviceId": null },
            "asr": { "localModel": "small", "effectiveLocalModel": null,
                     "cloudProvider": "deepgram", "language": "auto" },
            "postprocess": { "enabled": true, "personaId": "clean",
                             "llmProvider": "anthropic",
                             "modelFast": "claude-haiku-4-5",
                             "modelQuality": "claude-sonnet-4-6",
                             "timeoutMs": 6000 },
            "translation": { "enabled": false, "targetLanguage": "en" },
            "context": { "enabled": true },
            "commands": { "enabled": true },
            "injection": { "typeThresholdChars": 200, "restoreClipboardDelayMs": 600 },
            "history": { "saveText": true, "retentionDays": 30, "saveAudio": false },
            "hud": { "showPartials": true },
            "launchAtLogin": false
        })
    }

    /// AC: §8.3 — `Settings::default()` equals the literal defaults, exhaustively,
    /// pinned against the §8.3 PRD JSON TEXT itself: `expected` is built by
    /// deserializing `prd_8_3_json()`, not by re-typing the same values as a
    /// second Rust struct literal (review NIT — a struct-vs-struct compare
    /// proves nothing about matching the PRD).
    #[test]
    fn settings_defaults_match_prd_8_3() {
        let expected: Settings = serde_json::from_value(prd_8_3_json())
            .expect("§8.3 literal JSON must deserialize into Settings");
        assert_eq!(Settings::default(), expected);
    }

    /// AC: §8.3 — the default `Settings` serializes to *exactly* the §8.3 JSON
    /// wire shape (camelCase, including explicit `null`s) and round-trips back.
    #[test]
    fn settings_serde_camelcase_matches_prd_8_3() {
        let expected = prd_8_3_json();

        let actual = serde_json::to_value(Settings::default()).expect("serialize default Settings");
        assert_eq!(
            actual, expected,
            "Settings default must serialize to the exact §8.3 wire shape"
        );

        let round_tripped: Settings = serde_json::from_value(expected)
            .expect("§8.3 literal JSON must deserialize into Settings");
        assert_eq!(round_tripped, Settings::default());
    }

    /// AC: §8.3 "unknown fields preserved on rewrite" — a field the current
    /// schema doesn't know about survives an `update_settings` round-trip.
    #[test]
    fn update_preserves_unknown_fields() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store.update(serde_json::json!({})).expect("seed defaults");

        let raw_path = tmp.path().join("settings.json");
        let mut on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&raw_path).unwrap()).unwrap();
        on_disk["experimentalFeatureFlag"] = serde_json::json!(true);
        std::fs::write(&raw_path, serde_json::to_string_pretty(&on_disk).unwrap()).unwrap();

        store
            .update(serde_json::json!({ "mode": "local" }))
            .expect("update");

        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(&raw_path).unwrap()).unwrap();
        assert_eq!(
            after["experimentalFeatureFlag"],
            serde_json::json!(true),
            "unknown fields must survive rewrite (§8.3)"
        );
        assert_eq!(after["mode"], serde_json::json!("local"));
    }

    /// AC: §9.1 `update_settings` deep-merge — a partial nested-object patch
    /// only overwrites the keys it names; sibling keys are untouched.
    #[test]
    fn deep_merge_nested_object_partial() {
        let mut base = serde_json::json!({
            "postprocess": { "enabled": true, "timeoutMs": 6000, "personaId": "clean" }
        });
        let patch = serde_json::json!({ "postprocess": { "timeoutMs": 3000 } });
        deep_merge(&mut base, &patch);
        assert_eq!(
            base,
            serde_json::json!({
                "postprocess": { "enabled": true, "timeoutMs": 3000, "personaId": "clean" }
            })
        );
    }

    /// EC: §9.1 deep-merge — an explicit `null` in the patch sets the field to
    /// `null` (distinct from the key being absent, which leaves it untouched).
    #[test]
    fn deep_merge_null_sets_field() {
        let mut base = serde_json::json!({ "audio": { "inputDeviceId": "device-123" } });
        let patch = serde_json::json!({ "audio": { "inputDeviceId": null } });
        deep_merge(&mut base, &patch);
        assert_eq!(base["audio"]["inputDeviceId"], Value::Null);
    }

    /// EC: §9.1 deep-merge — arrays are replaced wholesale, never element-merged.
    #[test]
    fn deep_merge_array_replaced() {
        let mut base = serde_json::json!({ "asr": { "keywords": ["alpha", "beta", "gamma"] } });
        let patch = serde_json::json!({ "asr": { "keywords": ["delta"] } });
        deep_merge(&mut base, &patch);
        assert_eq!(base["asr"]["keywords"], serde_json::json!(["delta"]));
    }

    /// AC: atomic write — after a successful `update`, only `settings.json`
    /// exists (no leftover `.tmp`) and it parses as valid JSON.
    #[test]
    fn atomic_write_leaves_no_tmp_and_valid_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store
            .update(serde_json::json!({ "mode": "cloud" }))
            .expect("update");

        let entries: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(entries.contains(&"settings.json".to_string()));
        assert!(
            !entries.iter().any(|n| n.ends_with(".tmp")),
            "no .tmp file must survive a successful atomic write: {entries:?}"
        );

        let text = std::fs::read_to_string(tmp.path().join("settings.json")).unwrap();
        let _: Value = serde_json::from_str(&text).expect("settings.json must be valid JSON");
    }

    /// EC (defense-in-depth, plan §"0600 perms"): `settings.json` is written
    /// `0600` — consistency with the rest of the app-data dir, not a P-3
    /// requirement (the file holds no secrets).
    #[cfg(unix)]
    #[test]
    fn settings_file_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store.update(serde_json::json!({})).expect("update");

        let meta = std::fs::metadata(tmp.path().join("settings.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    /// EC: §8.3 validate — `postprocess.timeoutMs` is clamped to a floor of 1
    /// (never zero/negative) rather than rejected as an error.
    #[test]
    fn validate_clamps_timeout_ms() {
        let mut v = serde_json::json!({ "postprocess": { "timeoutMs": 0 } });
        validate(&mut v);
        assert_eq!(v["postprocess"]["timeoutMs"], serde_json::json!(1));

        let mut v2 = serde_json::json!({ "postprocess": { "timeoutMs": -500 } });
        validate(&mut v2);
        assert_eq!(v2["postprocess"]["timeoutMs"], serde_json::json!(1));
    }

    /// EC / OPEN_QUESTIONS Q8-c: an out-of-enum `hotkey.mode` patch is dropped
    /// (reject-by-ignore) rather than erroring; the prior valid value survives
    /// both the returned `Settings` and the persisted file.
    #[test]
    fn validate_rejects_invalid_hotkey_keeps_previous() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());

        let seeded = store.update(serde_json::json!({})).expect("seed defaults");
        assert_eq!(seeded.hotkey.mode, "push_to_talk");

        let after = store
            .update(serde_json::json!({ "hotkey": { "mode": "not_a_real_mode" } }))
            .expect("an invalid enum patch must not error, only be ignored");
        assert_eq!(
            after.hotkey.mode, "push_to_talk",
            "invalid hotkey.mode must be dropped, keeping the prior valid value"
        );

        let on_disk: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(on_disk["hotkey"]["mode"], serde_json::json!("push_to_talk"));
    }

    /// AC: §14 `DB-IO` / OPEN_QUESTIONS Q8-a — a genuine settings-write I/O
    /// failure (the settings dir can never be created because a path component
    /// is a regular file, not a mock) maps to `DB-IO`.
    #[test]
    fn settings_write_io_failure_maps_db_io() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let unwritable_dir = blocker.join("settings-subdir");

        let store = SettingsStore::new(unwritable_dir);
        let err = store
            .update(serde_json::json!({}))
            .expect_err("writing under a file-blocked path must fail");
        assert_eq!(
            err.code(),
            "DB-IO",
            "settings I/O failure must map to DB-IO (Q8)"
        );
    }

    #[test]
    fn secret_key_compounds_are_removed_but_benign_words_remain() {
        let mut value = serde_json::json!({
            "refreshToken": "private",
            "dbPassword": "private",
            "serviceCredential": "private",
            "customSecret": "private",
            "backupApiKey": "private",
            "myAuthorization": "private",
            "tokenizerModel": "safe",
            "secretaryMode": "safe",
            "passwordlessMode": true,
            "toKen_value": "safe"
        });
        strip_secret_bearing_fields(&mut value);
        assert!(value.get("refreshToken").is_none());
        assert!(value.get("dbPassword").is_none());
        assert!(value.get("serviceCredential").is_none());
        assert!(value.get("customSecret").is_none());
        assert!(value.get("backupApiKey").is_none());
        assert!(value.get("myAuthorization").is_none());
        assert_eq!(value["tokenizerModel"], "safe");
        assert_eq!(value["secretaryMode"], "safe");
        assert_eq!(value["passwordlessMode"], true);
        assert_eq!(value["toKen_value"], "safe");
    }

    #[test]
    fn update_scrubs_array_secrets_and_preexisting_unknowns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store.update(serde_json::json!({})).expect("seed defaults");
        let path = tmp.path().join("settings.json");
        let mut raw: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        raw["legacy"] = serde_json::json!({
            "refreshToken": "opaque-legacy-secret",
            "safe": true,
            "values": ["Token legacy-array-secret", "retain me"]
        });
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        store
            .update(serde_json::json!({ "mode": "local" }))
            .expect("sanitized update");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(after["legacy"].get("refreshToken").is_none());
        assert_eq!(after["legacy"]["safe"], true);
        assert_eq!(after["legacy"]["values"], serde_json::json!(["retain me"]));
    }

    /// AC: §12 P-3 / §4.4 — `settings.json` never carries an API key/secret field.
    #[test]
    fn p3_api_key_absent_from_settings_json() {
        let json = serde_json::to_string(&Settings::default()).expect("serialize");
        for needle in ["apiKey", "api_key", "sk-ant-", "secret"] {
            assert!(
                !json.to_lowercase().contains(&needle.to_lowercase()),
                "settings.json must never carry a key/secret field (§12 P-3): found {needle:?} in {json}"
            );
        }
    }

    /// AC: §9.1 `get_settings` / plan algorithm step 1 — on a fresh dir with no
    /// `settings.json`, `get()` returns the §8.3 defaults AND does NOT write the
    /// file (defaults are only persisted by a later `update`).
    #[test]
    fn get_returns_defaults_when_file_absent_without_writing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());

        let settings = store.get().expect("get on an absent settings.json");
        assert_eq!(settings, Settings::default());
        assert!(
            !tmp.path().join("settings.json").exists(),
            "get() must not write settings.json when it is absent"
        );
    }

    /// AC: §9.1 `get_settings` — reads back a value persisted by a prior
    /// `update()`, through a brand-new `SettingsStore` instance pointed at the
    /// same dir (proves a real disk round-trip, not in-memory state).
    #[test]
    fn get_returns_persisted_values() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let seed_store = SettingsStore::new(tmp.path().to_path_buf());
        seed_store
            .update(serde_json::json!({ "postprocess": { "timeoutMs": 4242 } }))
            .expect("seed a non-default value");

        let fresh_store = SettingsStore::new(tmp.path().to_path_buf());
        let settings = fresh_store.get().expect("get from a fresh store instance");
        assert_eq!(settings.postprocess.timeout_ms, 4242);
    }

    /// AC: §14 `DB-IO` / OPEN_QUESTIONS Q8-a — malformed JSON already on disk
    /// makes `get()` fail, mapped to `DB-IO`.
    #[test]
    fn get_parse_failure_maps_db_io() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("settings.json"), b"{not valid").unwrap();

        let store = SettingsStore::new(tmp.path().to_path_buf());
        let err = store.get().expect_err("malformed settings.json must error");
        assert_eq!(
            err.code(),
            "DB-IO",
            "settings.json parse failure must map to DB-IO (Q8)"
        );
    }

    /// EC / OPEN_QUESTIONS Q8-c: an out-of-range value already persisted on
    /// disk (not via `update`) is still clamped on read — `get()` runs
    /// `validate()` too, not only `update()`.
    #[test]
    fn get_validates_on_read_clamping_out_of_range_persisted_value() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut raw = serde_json::to_value(Settings::default()).unwrap();
        raw["postprocess"]["timeoutMs"] = serde_json::json!(0);
        std::fs::write(
            tmp.path().join("settings.json"),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();

        let store = SettingsStore::new(tmp.path().to_path_buf());
        let settings = store.get().expect("get");
        assert_eq!(
            settings.postprocess.timeout_ms, 1,
            "get() must clamp an out-of-range persisted value on read, same as update()"
        );
    }

    /// EC: §8.3 validate — a floating-point out-of-range numeric in a patch
    /// clamps the same way an integer one does (the `as_f64` branch of
    /// `clamp_min_field`, distinct from the `as_i64` branch already covered by
    /// `validate_clamps_timeout_ms`).
    #[test]
    fn validate_clamps_float_out_of_range_timeout_ms() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());

        let after = store
            .update(serde_json::json!({ "postprocess": { "timeoutMs": -1.5 } }))
            .expect("update with a float out-of-range value");
        assert_eq!(
            after.postprocess.timeout_ms, 1,
            "a float out-of-range timeoutMs must clamp the same way an integer one does"
        );
    }

    /// REGRESSION (code-review BLOCKING finding, Q8): `validate()` only guards
    /// enums/numeric floors, not field TYPES. A top-level type-invalid patch
    /// (e.g. `version` as a string) must never reach disk — `update()` may
    /// return `Err`, but settings.json must stay byte-identical to the last
    /// good state, and a subsequent `get()` on a FRESH `SettingsStore` must
    /// still succeed with the original values (the store must not be bricked).
    /// This is the exact Q8 invariant: "the validated/clamped value is what
    /// gets persisted, so a bad patch never corrupts the file."
    #[test]
    fn update_type_invalid_patch_does_not_corrupt_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store
            .update(serde_json::json!({ "mode": "cloud" }))
            .expect("seed a known-good state");

        let path = tmp.path().join("settings.json");
        let good_bytes = std::fs::read(&path).expect("read seeded settings.json");

        // A type-invalid patch (`version` must be a number). It is acceptable
        // for this call to return `Err`; what matters is the FILE and later
        // reads, asserted below.
        let _ = store.update(serde_json::json!({ "version": "notanumber" }));

        let after_bytes =
            std::fs::read(&path).expect("read settings.json after the type-invalid patch");
        assert_eq!(
            after_bytes, good_bytes,
            "a type-invalid patch must never be persisted (OPEN_QUESTIONS Q8 invariant)"
        );

        let fresh_store = SettingsStore::new(tmp.path().to_path_buf());
        let settings = fresh_store
            .get()
            .expect("a subsequent get() on a fresh store must still succeed — not bricked");
        assert_eq!(settings.mode, "cloud");
    }

    /// REGRESSION (code-review BLOCKING finding, Q8): the nested-field variant
    /// of the same bug — a type mismatch inside a nested object (`postprocess.enabled`
    /// must be a bool) must not corrupt settings.json or brick later reads either.
    #[test]
    fn update_type_invalid_nested_patch_does_not_corrupt_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SettingsStore::new(tmp.path().to_path_buf());
        store
            .update(serde_json::json!({ "postprocess": { "timeoutMs": 4242 } }))
            .expect("seed a known-good state");

        let path = tmp.path().join("settings.json");
        let good_bytes = std::fs::read(&path).expect("read seeded settings.json");

        let _ = store.update(serde_json::json!({ "postprocess": { "enabled": "yes" } }));

        let after_bytes =
            std::fs::read(&path).expect("read settings.json after the type-invalid nested patch");
        assert_eq!(
            after_bytes, good_bytes,
            "a nested type-invalid patch must never be persisted (OPEN_QUESTIONS Q8 invariant)"
        );

        let fresh_store = SettingsStore::new(tmp.path().to_path_buf());
        let settings = fresh_store
            .get()
            .expect("a subsequent get() on a fresh store must still succeed — not bricked");
        assert_eq!(settings.postprocess.timeout_ms, 4242);
    }
}
