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
            format!("```rust\n{}\n```", "let a = 1;\n".repeat(200)),
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
            .all(|p| session::markdown::fits(&p.text_content(), 1800))
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
