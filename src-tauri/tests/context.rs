//! T2.3 acceptance coverage for FR-2.3 context resolution.

use whisperspree_lib::{
    context::{
        inject_context, AppStyle, ContextResolver, FrontmostSnapshot, ResolvedContext, StyleId,
    },
    error::Error,
    pipeline::AppContext,
    store::app_rules::AppRuleRow,
};

#[derive(Debug)]
struct SnapshotFixture {
    context: AppContext,
    calls: usize,
}

impl SnapshotFixture {
    fn new(bundle_id: &str, title: &str, secure_input: bool) -> Self {
        Self {
            context: AppContext {
                bundle_id: bundle_id.into(),
                title: title.into(),
                secure_input,
            },
            calls: 0,
        }
    }
}

impl FrontmostSnapshot for SnapshotFixture {
    fn frontmost(&mut self) -> Result<AppContext, Error> {
        self.calls += 1;
        Ok(self.context.clone())
    }
}

fn rule(
    id: &str,
    bundle_id: &str,
    title_regex: Option<&str>,
    style_id: Option<&str>,
    priority: i64,
) -> AppRuleRow {
    AppRuleRow {
        id: id.into(),
        bundle_id: bundle_id.into(),
        title_regex: title_regex.map(str::to_owned),
        style_id: style_id.map(str::to_owned),
        persona_id: None,
        custom_prompt_id: None,
        priority,
    }
}

mod rules {
    use super::*;

    /// AC-2.3 / FR-2.3: defaults are table-driven, including the title-regex
    /// Gmail-in-Chrome case and the `com.jetbrains.*` code family.
    #[test]
    fn fr_2_3_default_rules_resolve_expected_style() {
        let cases = [
            ("com.tinyspeck.slackmacgap", "#general", StyleId::Chat),
            ("com.apple.mail", "New Message", StyleId::Email),
            ("com.google.Chrome", "Inbox (12) - Gmail", StyleId::Email),
            ("com.microsoft.VSCode", "main.rs", StyleId::Code),
            ("com.jetbrains.intellij", "project", StyleId::Code),
            ("notion.id", "Project notes", StyleId::Notes),
            ("com.example.unknown", "Anything", StyleId::Default),
        ];

        for (bundle_id, title, expected_style) in cases {
            let resolved = ContextResolver::new(
                SnapshotFixture::new(bundle_id, title, false),
                Vec::new(),
                true,
            )
            .snapshot()
            .expect("fixture snapshot resolves");

            assert_eq!(
                resolved.style.style_id, expected_style,
                "{bundle_id}: {title}"
            );
        }
    }

    /// FR-2.3: user rules take precedence over defaults, and the database's
    /// documented priority ordering chooses the highest matching rule.
    #[test]
    fn fr_2_3_user_rules_override_defaults_by_highest_priority() {
        let mut highest = rule(
            "highest",
            "com.tinyspeck.slackmacgap",
            Some("(?i)^#engineering"),
            Some("email"),
            20,
        );
        highest.persona_id = Some("formal".into());
        highest.custom_prompt_id = Some("prompt-1".into());

        let resolved = ContextResolver::new(
            SnapshotFixture::new("com.tinyspeck.slackmacgap", "#engineering", false),
            vec![
                rule(
                    "lower",
                    "com.tinyspeck.slackmacgap",
                    None,
                    Some("notes"),
                    10,
                ),
                highest,
            ],
            true,
        )
        .snapshot()
        .expect("snapshot resolves");

        assert_eq!(resolved.style.style_id, StyleId::Email);
        assert_eq!(resolved.style.persona_id.as_deref(), Some("formal"));
        assert_eq!(resolved.style.custom_prompt_id.as_deref(), Some("prompt-1"));
    }

    /// FR-2.3: an optional user title regex is a further constraint, not a
    /// blanket match for every window owned by that bundle id.
    #[test]
    fn fr_2_3_user_title_regex_must_match_before_rule_applies() {
        let resolved = ContextResolver::new(
            SnapshotFixture::new("com.google.Chrome", "Docs - Project", false),
            vec![rule(
                "gmail-only",
                "com.google.Chrome",
                Some("(?i)gmail"),
                Some("code"),
                100,
            )],
            true,
        )
        .snapshot()
        .expect("snapshot resolves");

        assert_eq!(resolved.style.style_id, StyleId::Default);
    }

    /// FR-2.3: a rule that only selects a custom prompt retains the matching
    /// built-in style, preserving code safety for code applications.
    #[test]
    fn fr_2_3_custom_prompt_only_rule_retains_default_style() {
        let mut prompt_only = rule("prompt-only", "com.microsoft.VSCode", None, None, 1);
        prompt_only.custom_prompt_id = Some("technical-team".into());

        let resolved = ContextResolver::new(
            SnapshotFixture::new("com.microsoft.VSCode", "lib.rs", false),
            vec![prompt_only],
            true,
        )
        .snapshot()
        .expect("snapshot resolves");

        assert_eq!(resolved.style.style_id, StyleId::Code);
        assert_eq!(
            resolved.style.custom_prompt_id.as_deref(),
            Some("technical-team")
        );
    }

    /// FR-2.3: disabling context forces neutral/default style regardless of
    /// the frontmost app or stored rules, while preserving the secure snapshot.
    #[test]
    fn fr_2_3_disabled_context_returns_default_and_preserves_snapshot() {
        let resolved = ContextResolver::new(
            SnapshotFixture::new("com.microsoft.VSCode", "secrets.rs", true),
            vec![rule(
                "override",
                "com.microsoft.VSCode",
                None,
                Some("chat"),
                1,
            )],
            false,
        )
        .snapshot()
        .expect("snapshot resolves");

        assert_eq!(resolved.style, AppStyle::default());
        assert_eq!(
            resolved,
            ResolvedContext {
                app: AppContext {
                    bundle_id: "com.microsoft.VSCode".into(),
                    title: "secrets.rs".into(),
                    secure_input: true,
                },
                style: AppStyle::default(),
            }
        );
    }

    /// Stored regexes are user-configurable. A malformed one must be unable to
    /// destabilize session start; it behaves as a non-match.
    #[test]
    fn fr_2_3_invalid_user_title_regex_is_ignored() {
        let resolved = ContextResolver::new(
            SnapshotFixture::new("com.apple.Notes", "Today", false),
            vec![rule(
                "bad-regex",
                "com.apple.Notes",
                Some("[unclosed"),
                Some("email"),
                100,
            )],
            true,
        )
        .snapshot()
        .expect("invalid stored regex must not fail a session");

        assert_eq!(resolved.style.style_id, StyleId::Notes);
    }

    /// The OS-facing trait makes snapshots deterministic in tests and ensures
    /// each resolver snapshot reads the frontmost app exactly once.
    #[test]
    fn fr_2_3_snapshot_delegates_once_to_frontmost_provider() {
        let provider = SnapshotFixture::new("com.apple.Terminal", "zsh", false);
        let mut resolver = ContextResolver::new(provider, Vec::new(), true);

        let resolved = resolver.snapshot().expect("snapshot resolves");

        assert_eq!(resolved.style.style_id, StyleId::Code);
        assert_eq!(resolver.provider().calls, 1);
    }
}

fn frontmost(bundle_id: &str, secure: bool) -> AppContext {
    AppContext {
        bundle_id: bundle_id.into(),
        title: "Window".into(),
        secure_input: secure,
    }
}

#[test]
fn fr_1_4_resolved_context_types_and_pastes_for_a_normal_target() {
    let ctx = inject_context(Some(&frontmost("com.example.editor", false)), true);
    assert!(!ctx.secure_input);
    assert!(ctx.target_is_valid);
    assert!(ctx.typing_available);
}

#[test]
fn fr_1_4_secure_context_forces_clipboard_only_flags() {
    let ctx = inject_context(Some(&frontmost("com.example.passwords", true)), true);
    assert!(ctx.secure_input);
}

#[test]
fn fr_1_4_own_window_is_never_a_valid_target() {
    let ctx = inject_context(Some(&frontmost("com.whisperspree.app", false)), true);
    assert!(!ctx.target_is_valid);
}

#[test]
fn fr_1_4_unknown_bundle_ids_are_not_valid_targets() {
    let ctx = inject_context(Some(&frontmost("unknown", false)), true);
    assert!(!ctx.target_is_valid);
}

#[test]
fn fr_1_4_missing_frontmost_is_not_a_valid_target() {
    let ctx = inject_context(None, true);
    assert!(!ctx.target_is_valid);
}

#[test]
fn fr_1_4_untrusted_accessibility_disables_typing_but_keeps_paste() {
    let ctx = inject_context(Some(&frontmost("com.example.editor", false)), false);
    assert!(ctx.target_is_valid);
    assert!(!ctx.typing_available);
}

#[test]
fn ec_1_4_ax_probe_absence_cannot_fail_resolution() {
    // Resolution is total: no Result, no panic path for missing AX data.
    let ctx = inject_context(Some(&frontmost("com.example.editor", false)), false);
    let _ = ctx.secure_input;
}
