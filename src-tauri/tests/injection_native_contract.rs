//! T2.2 — native macOS adapter contracts for FR-1.4 clipboard preservation.
//!
//! The real NSPasteboard/enigo calls are OS-permission surfaces that headless
//! tests must not exercise (CLAUDE.md §3); these source contracts pin the
//! wiring decisions the manual QA-2 sheet then validates on hardware.

use std::{fs, path::PathBuf};

fn source(path: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect("required injection adapter source must exist")
}

#[test]
fn production_test_injection_uses_the_managed_injector_and_macos_adapters() {
    let commands = source("src/ipc/commands.rs");
    // Whitespace-normalized so rustfmt reflow cannot mask the wiring.
    let flat = commands.split_whitespace().collect::<String>();
    assert!(
        flat.contains("state.injector.inject("),
        "test_injection must dispatch through the managed §9.3 Injector"
    );
    // Platform default factories live beside `default_dictation_runtime` in
    // the command adapter, matching the established house pattern.
    for adapter in [
        "MacFrontmostSnapshot",
        "InjectionService::new",
        "MacClipboard",
        "MacTypist",
        "MacPasteVerifier",
    ] {
        assert!(
            commands.contains(adapter),
            "the macOS default injection stack must wire {adapter}"
        );
    }
}

#[test]
fn macos_frontmost_composes_carbon_and_best_effort_ax_secure_probes() {
    let macos = source("src/context/macos.rs");
    assert!(
        macos.contains("IsSecureEventInputEnabled"),
        "the Carbon secure-event probe is permission-free and must be consulted"
    );
    assert!(
        macos.contains("AXUIElementCopyAttributeValue"),
        "the focused-element AX role check must exist as a best-effort probe"
    );
    assert!(
        !macos.contains("secure_input: true"),
        "the hardcoded conservative secure flag must be replaced by real probes"
    );
    assert!(
        macos.contains("inject_context(Some(&snapshot)"),
        "the pure FR-1.4 decision function must drive adapter composition"
    );
}

#[test]
fn macos_typist_sets_inter_key_delay_and_always_releases_meta() {
    let macos = source("src/inject/macos.rs");
    assert!(
        macos.contains("set_delay("),
        "FR-1.4 requires the configured 3 ms inter-key delay to reach enigo"
    );
    assert!(
        !macos.contains("and_then(|_| enigo.key(Key::Meta, Direction::Release))"),
        "⌘ release must never be chained after the V click: a failed synthesis \
         would otherwise leave the modifier stuck"
    );
    assert!(
        macos.contains("let released ="),
        "the paste shortcut must unconditionally synthesize the ⌘ release"
    );
}

#[test]
fn macos_clipboard_revision_tracks_the_native_change_count() {
    let macos = source("src/inject/macos.rs");
    assert!(
        macos.contains("changeCount()"),
        "MacClipboard::snapshot must use NSPasteboard changeCount as its revision"
    );
    assert!(
        !macos.contains("DefaultHasher"),
        "content hashes must not replace the native change count (R-4)"
    );
}

#[test]
fn macos_clipboard_reads_rtf_and_classifies_types_before_capture() {
    let policy = source("src/inject/mod.rs");
    assert!(
        policy.contains("\"public.rtf\"") && policy.contains("\"public.utf8-plain-text\""),
        "the shared classifier must own the canonical UTI constants"
    );
    let macos = source("src/inject/macos.rs");
    assert!(
        macos.contains("pasteboard_carries_restorable_text"),
        "non-text detection must go through the shared FR-1.4 classifier"
    );
    assert!(
        macos.contains("PASTEBOARD_TYPE_RTF"),
        "the RTF flavor must be captured alongside plain text (FR-1.4)"
    );
}

#[test]
fn macos_restore_clears_empty_snapshots_via_clear_contents() {
    let macos = source("src/inject/macos.rs");
    assert!(
        macos.contains("restore_action("),
        "restore must be driven by the pure RestoreAction decision"
    );
    assert!(
        macos.contains("clearContents()"),
        "an empty pre-injection clipboard is restored with a true clearContents, never set_text(\"\")"
    );
}
