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
