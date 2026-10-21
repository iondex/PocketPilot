use super::*;

#[test]
fn common_markdown_becomes_discord_subset_without_activating_raw_markup() {
    let text = messages(
        "# 标题\n\n前文**中文。**后文 ~~old~~\n\n> **note**\n\n- [x] done\n- [ ] pending\n\n<@123> ||literal|| [safe](https://example.com/a(b))",
        Format::Markdown,
    )
    .unwrap().join("\n");
    assert!(text.starts_with("# 标题"));
    assert!(text.contains("**中文。**"));
    assert!(text.contains("~~old~~"));
    assert!(text.contains("> **note**"));
    assert!(text.contains("- ☑ done"));
    assert!(text.contains("- ☐ pending"));
    assert!(text.contains("\\<@123\\>"));
    assert!(text.contains("\\|\\|literal\\|\\|"));
    assert!(text.contains("[safe](https://example.com/a%28b%29)"));
}

#[test]
fn tables_links_and_code_keep_meaning_without_inventing_supported_syntax() {
    let text = messages(
        "| Name | Value |\n|---|---|\n| **Alpha** | [docs](https://example.com) |\n\n```rust\nlet a = \"<&>\";\n```\n\n[bad](javascript:unsafe)",
        Format::Markdown,
    )
    .unwrap().join("\n");
    assert!(text.contains("**Name**: **Alpha**"));
    assert!(text.contains("**Value**: [docs](https://example.com/)"));
    assert!(text.contains("```rust\nlet a = \"<&>\";\n```"));
    assert!(!text.contains("javascript:"));
    assert!(text.ends_with("bad"));
}

#[test]
fn oversized_and_complex_code_is_always_message_content() {
    for source in [
        "````text\na```b\n````".to_string(),
        "``a`b``".into(),
        "😀".repeat(901),
        "*".repeat(1000),
    ] {
        let parts = messages(&source, Format::Markdown).unwrap();
        assert!(!parts.is_empty());
        assert!(parts.iter().all(|s| session::markdown::fits(s, 1800)));
        assert!(parts.iter().all(|s| !s.contains("session.md")));
    }
    let source = "😀".repeat(901);
    assert!(messages(&source, Format::Markdown).unwrap().len() > 1);
    assert!(messages(&"a".repeat(session::markdown::MAX_BYTES + 1), Format::Plain).is_err());
}

#[test]
fn ui_titles_are_literal_and_emoji_stay_unicode() {
    let text = messages("**title** <#123> :custom: 👩‍💻\n1. name", Format::Plain)
        .unwrap()
        .join("\n");
    assert_eq!(
        text,
        "\\*\\*title\\*\\* \\<\\#123\\> :custom: 👩‍💻\n1\\. name"
    );
}

#[test]
fn each_conversation_part_starts_with_its_role_and_keeps_markdown_and_length_limits() {
    let item = |role, text: String| session::TextItem {
        turn_id: "turn".into(),
        item_id: "item".into(),
        role,
        phase: None,
        text,
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let items = [
        item(
            session::Role::User,
            "# Request\n\n**Question** <@123>".into(),
        ),
        item(
            session::Role::Agent,
            format!("```rust\n{}\n```", "let a = 1;\n".repeat(800)),
        ),
    ];
    let output = Conversation::changes(items.iter(), "en").unwrap();
    assert!(
        output.parts[0]
            .text_content()
            .starts_with("👤 **User**\n\n# Request")
    );
    assert!(
        output.parts[0]
            .text_content()
            .contains("**Question** \\<@123\\>")
    );
    assert!(output.parts.len() > 2);
    for part in &output.parts[1..] {
        assert!(
            part.text_content()
                .starts_with("🤖 **Agent** · 💬\n\n```rust\n")
        );
        assert!(part.text_content().ends_with("```"));
    }
    assert!(
        output
            .parts
            .iter()
            .all(|p| session::markdown::fits(&p.text_content(), 3600))
    );
}

#[test]
fn recent_interaction_uses_role_icons_and_only_confirmed_final_output() {
    let item = |role, phase: Option<&str>, text: &str| session::TextItem {
        turn_id: "turn".into(),
        item_id: "item".into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let mut history = session::History {
        items: vec![
            item(session::Role::User, None, "question"),
            item(session::Role::Agent, Some("commentary"), "working"),
            item(session::Role::Agent, Some("final_answer"), "**answer**"),
        ],
        all_pages_read: true,
    };
    let output = Conversation::latest(&history, "zh-CN").unwrap();
    assert_eq!(
        output
            .parts
            .iter()
            .map(|part| part.text_content())
            .collect::<Vec<_>>(),
        ["👤 **用户**\n\nquestion", "🤖 **Agent** · ✅\n\n**answer**"]
    );
    assert!(!output.text.contains("working"));
    history.all_pages_read = false;
    let output = Conversation::latest(&history, "en").unwrap();
    assert!(output.parts[0].text_content().starts_with("ℹ️\n\n"));
    assert!(!output.text.contains("question") && !output.text.contains("answer"));
}

fn components(source: &str, heading: Option<&str>) -> Vec<crate::discord::native::Message> {
    ComponentRenderer
        .render(RenderRequest {
            source,
            format: Format::Markdown,
            heading,
        })
        .unwrap()
        .parts
}

#[test]
fn dense_markdown_and_short_paragraphs_pack_by_rendered_length() {
    let mut source = String::from("| Name | Value |\n|---|---|\n");
    for index in 0..12 {
        source.push_str(&format!("| **Row {index}** | `value {index}` |\n"));
    }
    source.push_str("\n## Notes\n\n");
    source.push_str(&"A **short** paragraph with *formatting*.\n\n".repeat(12));
    let parts = components(&source, Some("🤖 **Agent**"));
    assert_eq!(
        parts.len(),
        1,
        "AST nodes are not native Discord components"
    );
    let text = parts[0].text_content();
    assert!(text.contains("**Name**: **Row 11**"));
    assert!(text.contains("## Notes"));
    assert_eq!(text.matches("A **short** paragraph").count(), 12);
    assert!(text.encode_utf16().count() > 700);
    assert!(session::markdown::fits(&text, 3600));
}

#[test]
fn rendered_budgets_cover_escape_expansion_unicode_links_and_code() {
    for source in [
        "\\*\\|\\<\\>".repeat(1500),
        "👩‍💻中文**strong** ".repeat(900),
        format!("```rust\n{}\n```", "let a = 1;\n".repeat(1500)),
        format!("[label](https://example.com/{})", "path".repeat(950)),
    ] {
        let parts = components(&source, Some("🤖 **Agent** · ✅"));
        assert!(!parts.is_empty());
        assert!(
            parts
                .iter()
                .all(|part| session::markdown::fits(&part.text_content(), 3600))
        );
        assert!(
            parts
                .iter()
                .all(|part| part.allowed_mentions.parse.is_empty())
        );
    }
    assert!(
        ComponentRenderer
            .render(RenderRequest {
                source: "text",
                format: Format::Plain,
                heading: Some(&"a".repeat(3590))
            })
            .is_err()
    );
}

#[test]
fn galleries_are_bounded_and_every_text_display_is_nonempty() {
    use twilight_model::channel::message::Component;
    let source = (0..27)
        .map(|index| format!("![image {index}](https://example.com/{index}.png)"))
        .collect::<String>();
    let parts = components(&source, None);
    let mut images = 0;
    for part in &parts {
        assert!(!part.text_content().is_empty());
        assert!(session::markdown::fits(&part.text_content(), 3600));
        let Component::Container(container) = &part.components[0] else {
            panic!("expected container");
        };
        assert!(container.components.len() <= 2);
        for component in &container.components {
            if let Component::MediaGallery(gallery) = component {
                assert!((1..=10).contains(&gallery.items.len()));
                images += gallery.items.len();
            }
        }
    }
    assert_eq!(images, 27);
}

#[test]
fn adjacent_agent_items_merge_only_within_the_same_turn_and_phase() {
    let item = |role, turn: &str, phase: Option<&str>, text: &str| session::TextItem {
        turn_id: turn.into(),
        item_id: text.into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let items = [
        item(session::Role::Agent, "one", Some("commentary"), "first"),
        item(session::Role::Agent, "one", Some("commentary"), "second"),
        item(session::Role::Agent, "one", Some("final_answer"), "final"),
        item(session::Role::Agent, "two", Some("final_answer"), "next"),
        item(session::Role::User, "two", None, "user one"),
        item(session::Role::User, "two", None, "user two"),
        item(
            session::Role::Agent,
            "two",
            Some("final_answer"),
            "after user",
        ),
    ];
    let output = Conversation::changes(items.iter(), "en").unwrap();
    assert_eq!(output.parts.len(), 6);
    assert!(output.parts[0].text_content().ends_with("first\n\nsecond"));
    assert_eq!(output.parts[0].text_content().matches("🤖").count(), 1);
    assert!(output.parts[4].text_content().ends_with("user two"));
    assert!(output.parts[5].text_content().ends_with("after user"));
}

#[test]
fn literal_packing_preserves_every_character_across_message_boundaries() {
    let source = "😀 中文 * _ ~ < > & | ` end\n".repeat(700);
    let parts = ComponentRenderer
        .render(RenderRequest {
            source: &source,
            format: Format::Plain,
            heading: None,
        })
        .unwrap()
        .parts;
    assert!(parts.len() > 1);
    assert!(
        parts
            .iter()
            .all(|part| session::markdown::fits(&part.text_content(), 3600))
    );
    assert_eq!(
        parts
            .iter()
            .map(|part| part.text_content())
            .collect::<String>(),
        literal(&source)
    );
}
