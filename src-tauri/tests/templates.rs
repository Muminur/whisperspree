use whisperspree_lib::llm::{
    prompts::{assemble, LanguageRule, RewriteBlock, StyleId, TemplateId},
    ModelTier, ProcessRequest,
};

struct TemplateExpectation {
    id: &'static str,
    name: &'static str,
    block: &'static str,
}

const TEMPLATES: &[TemplateExpectation] = &[
    TemplateExpectation { id: "action_items", name: "Action items", block: "Transform: extract every task or commitment as a checklist. One line per item: \"- [ ] task — owner (if stated) — due (if stated)\". Nothing else." },
    TemplateExpectation { id: "meeting_summary", name: "Meeting summary", block: "Transform: output three sections titled TL;DR (2 sentences max), Decisions, Action items. Use \"- \" lines under the last two. Omit a section if empty." },
    TemplateExpectation { id: "followup_email", name: "Follow-up email", block: "Transform: a professional follow-up email based on the content. First line: \"Subject: …\". Then greeting, short body recapping key points and next steps, and a sign-off placeholder \"[Name]\"." },
    TemplateExpectation { id: "blog_draft", name: "Blog post draft", block: "Transform: a blog post draft. A compelling title on the first line, then an intro paragraph, 2–4 subheaded sections developing the spoken ideas (do not invent facts), and a one-paragraph conclusion." },
    TemplateExpectation { id: "bullet_summary", name: "Bullet summary", block: "Transform: 3–7 \"- \" bullets capturing the key points. No intro or outro text." },
    TemplateExpectation { id: "todo_list", name: "To-do list", block: "Transform: a to-do list, \"- [ ] \" per line, imperative mood, deduplicated." },
    TemplateExpectation { id: "slack_update", name: "Slack update", block: "Transform: a Slack status update, 3–5 short lines, plain language, emoji allowed where natural, no greeting." },
    TemplateExpectation { id: "standup", name: "Standup update", block: "Transform: three sections: \"Yesterday:\", \"Today:\", \"Blockers:\" each with \"- \" lines. Write \"Blockers: none\" if none were mentioned." },
    TemplateExpectation { id: "commit_message", name: "Git commit message", block: "Transform: a Conventional Commits message. First line \"type(scope): summary\" ≤ 72 chars; blank line; body wrapped at 72 chars explaining what and why. Infer type from content (feat/fix/chore/docs/refactor)." },
    TemplateExpectation { id: "github_issue", name: "GitHub issue", block: "Transform: a GitHub issue. \"Title: …\" then sections \"## Description\", \"## Steps to reproduce\" (numbered), \"## Expected\", \"## Actual\". Omit repro sections if not a bug." },
    TemplateExpectation { id: "tweet", name: "Tweet / X post", block: "Transform: a single post ≤ 280 characters. Hook first. No hashtags unless one is clearly central." },
    TemplateExpectation { id: "x_thread", name: "X thread", block: "Transform: a numbered thread (\"1/\", \"2/\", …), each part ≤ 280 characters, first part is the hook, last part concludes or CTAs." },
    TemplateExpectation { id: "linkedin_post", name: "LinkedIn post", block: "Transform: a LinkedIn post. Strong first line, short paragraphs with line breaks, professional but human, end with a question or takeaway. ≤ 1300 characters." },
    TemplateExpectation { id: "pros_cons", name: "Pros & cons", block: "Transform: two sections \"Pros:\" and \"Cons:\" with \"- \" lines drawn only from what was said." },
    TemplateExpectation { id: "journal", name: "Journal entry", block: "Transform: a tidy first-person journal entry. Keep the personal voice and chronology; fix rambling; add a date line placeholder \"{{today}}\" on top." },
    TemplateExpectation { id: "exec_summary", name: "Executive summary", block: "Transform: an executive summary ≤ 120 words, one paragraph, leading with the outcome or recommendation." },
    TemplateExpectation { id: "text_reply", name: "Text message reply", block: "Transform: a casual SMS-length reply (≤ 2 sentences) conveying the core message." },
];

#[test]
fn ids_expose_every_prd_7_5_template_with_exact_metadata_and_blocks() {
    assert_eq!(TemplateId::all().len(), TEMPLATES.len());

    for expected in TEMPLATES {
        let template = TemplateId::from_id(expected.id)
            .unwrap_or_else(|| panic!("missing template id {}", expected.id));
        assert_eq!(template.id(), expected.id);
        assert_eq!(template.name(), expected.name);
        assert_eq!(template.block(), expected.block);
    }

    assert_eq!(TemplateId::from_id("not_a_template"), None);
}

#[test]
fn transforms_assemble_without_style_and_use_quality_model() {
    for template in TemplateId::all() {
        let request = ProcessRequest {
            raw_text: "Sam will ship it Friday".into(),
            glossary: vec![],
            style: StyleId::Email,
            rewrite: RewriteBlock::Template(*template),
            language_rule: LanguageRule::KeepSpeaker,
            app_name: "Mail".into(),
            model_tier: ModelTier::Quality,
        };

        let prompt = assemble(&request);
        assert_eq!(request.model_tier, ModelTier::Quality);
        assert!(prompt.system.contains(template.block()));
        assert!(!prompt
            .system
            .contains("Style: professional email body text."));
    }
}
