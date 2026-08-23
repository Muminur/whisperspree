//! Deterministic FR-2.3 app-style rule resolution.

use super::FrontmostSnapshot;
use crate::{error::Error, pipeline::AppContext, store::app_rules::AppRuleRow};

/// The fixed §7.2 style blocks addressable by FR-2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleId {
    Default,
    Chat,
    Email,
    Code,
    Notes,
}

impl StyleId {
    /// Parse persisted §7.2 style ids without accepting arbitrary prompt text.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "default" => Some(Self::Default),
            "chat" => Some(Self::Chat),
            "email" => Some(Self::Email),
            "code" => Some(Self::Code),
            "notes" => Some(Self::Notes),
            _ => None,
        }
    }
}

/// Fully resolved style inputs retained with an app snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppStyle {
    pub style_id: StyleId,
    pub persona_id: Option<String>,
    pub custom_prompt_id: Option<String>,
}

impl Default for AppStyle {
    fn default() -> Self {
        Self {
            style_id: StyleId::Default,
            persona_id: None,
            custom_prompt_id: None,
        }
    }
}

/// Frontmost app snapshot plus the style selected for that app at session start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContext {
    pub app: AppContext,
    pub style: AppStyle,
}

/// Resolves app rules in one pass over a deterministic frontmost snapshot.
pub struct ContextResolver<P> {
    provider: P,
    rules: Vec<AppRuleRow>,
    enabled: bool,
}

impl<P> ContextResolver<P> {
    pub fn new(provider: P, rules: Vec<AppRuleRow>, enabled: bool) -> Self {
        Self {
            provider,
            rules,
            enabled,
        }
    }

    /// Exposes the provider for deterministic fixture assertions.
    pub fn provider(&self) -> &P {
        &self.provider
    }
}

impl<P: FrontmostSnapshot> ContextResolver<P> {
    /// Capture the frontmost app once and resolve its style. `enabled = false`
    /// always returns neutral/default style as required by FR-2.3.
    pub fn snapshot(&mut self) -> Result<ResolvedContext, Error> {
        let app = self.provider.frontmost()?;
        let style = if self.enabled {
            resolve_style(&app, &self.rules)
        } else {
            AppStyle::default()
        };
        Ok(ResolvedContext { app, style })
    }
}

/// Lets the existing session coordinator consume this detector at both SM-5
/// snapshot points. The coordinator carries `AppContext`; callers that also
/// need the resolved style use the inherent [`ContextResolver::snapshot`]
/// method above at session start.
impl<P: FrontmostSnapshot> crate::pipeline::ContextDetector for ContextResolver<P> {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        Ok(ContextResolver::snapshot(self)?.app)
    }
}

/// Resolve user `app_rules` before the built-in FR-2.3 defaults.
///
/// Matching user rules are ordered by descending `priority`, then ascending
/// stable rule id so a malformed or unordered SQLite result never changes a
/// session outcome. A custom-prompt-only rule intentionally inherits the built-
/// in style, preserving the code-style safety guarantees in FR-2.3.a.
pub fn resolve_style(app: &AppContext, rules: &[AppRuleRow]) -> AppStyle {
    let default_style = default_style(app);
    let selected_rule = rules
        .iter()
        .filter(|rule| user_rule_matches(rule, app))
        .max_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| right.id.cmp(&left.id))
        });

    match selected_rule {
        Some(rule) => AppStyle {
            style_id: rule
                .style_id
                .as_deref()
                .and_then(StyleId::parse)
                .unwrap_or(default_style),
            persona_id: rule.persona_id.clone(),
            custom_prompt_id: rule.custom_prompt_id.clone(),
        },
        None => AppStyle {
            style_id: default_style,
            ..AppStyle::default()
        },
    }
}

fn user_rule_matches(rule: &AppRuleRow, app: &AppContext) -> bool {
    rule.bundle_id == app.bundle_id
        && rule.title_regex.as_ref().map_or(true, |pattern| {
            regex::Regex::new(pattern)
                .map(|regex| regex.is_match(&app.title))
                // A persisted invalid expression cannot prevent dictation from
                // starting. The Rules task owns creation-time validation.
                .unwrap_or(false)
        })
}

fn default_style(app: &AppContext) -> StyleId {
    let bundle_id = app.bundle_id.as_str();
    if matches_bundle(
        bundle_id,
        &[
            "com.tinyspeck.slackmacgap",
            "com.hnc.Discord",
            "com.apple.MobileSMS",
            "WhatsApp",
            "Telegram",
        ],
    ) {
        StyleId::Chat
    } else if matches_bundle(bundle_id, &["com.apple.mail", "com.microsoft.Outlook"])
        || title_is_email(&app.title)
    {
        StyleId::Email
    } else if matches_bundle(
        bundle_id,
        &[
            "com.microsoft.VSCode",
            "com.todesktop.230313mzl4w4u92",
            "dev.zed.Zed",
            "com.googlecode.iterm2",
            "com.apple.Terminal",
        ],
    ) || bundle_id.starts_with("com.jetbrains.")
    {
        StyleId::Code
    } else if matches_bundle(
        bundle_id,
        &[
            "notion.id",
            "com.apple.Notes",
            "md.obsidian",
            "com.culturedcode.ThingsMac",
        ],
    ) {
        StyleId::Notes
    } else {
        StyleId::Default
    }
}

fn matches_bundle(bundle_id: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| bundle_id.contains(pattern))
}

fn title_is_email(title: &str) -> bool {
    // Static, audited expression from the FR-2.3 defaults table.
    regex::Regex::new("(?i)gmail|outlook|mail")
        .expect("FR-2.3 email title regex is valid")
        .is_match(title)
}
