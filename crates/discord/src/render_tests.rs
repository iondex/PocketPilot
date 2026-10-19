use super::*;

#[test]
fn common_markdown_becomes_discord_subset_without_activating_raw_markup() {
    let text = content(
        "# 标题\n\n前文**中文。**后文 ~~old~~\n\n> **note**\n\n- [x] done\n- [ ] pending\n\n<@123> ||literal|| [safe](https://example.com/a(b))",
        Format::Markdown,
    )
    .unwrap();
    assert!(!text.attachment);
    assert!(text.text.starts_with("# 标题"));
    assert!(text.text.contains("**中文。**"));
    assert!(text.text.contains("~~old~~"));
    assert!(text.text.contains("> **note**"));
    assert!(text.text.contains("- ☑ done"));
    assert!(text.text.contains("- ☐ pending"));
    assert!(text.text.contains("\\<@123\\>"));
    assert!(text.text.contains("\\|\\|literal\\|\\|"));
    assert!(text.text.contains("[safe](https://example.com/a%28b%29)"));
}

#[test]
fn tables_links_and_code_keep_meaning_without_inventing_supported_syntax() {
    let text = content(
        "| Name | Value |\n|---|---|\n| **Alpha** | [docs](https://example.com) |\n\n```rust\nlet a = \"<&>\";\n```\n\n[bad](javascript:unsafe)",
        Format::Markdown,
    )
    .unwrap();
    assert!(!text.attachment);
    assert!(text.text.contains("**Name**: **Alpha**"));
    assert!(
        text.text
            .contains("**Value**: [docs](https://example.com/)")
    );
    assert!(text.text.contains("```rust\nlet a = \"<&>\";\n```"));
    assert!(!text.text.contains("javascript:"));
    assert!(text.text.ends_with("bad"));
}

#[test]
fn unsupported_fences_and_rendered_utf16_overflow_choose_original_attachment() {
    assert!(
        content("````text\na```b\n````", Format::Markdown)
            .unwrap()
            .attachment
    );
    assert!(content("``a`b``", Format::Markdown).unwrap().attachment);
    assert!(
        content(&"😀".repeat(901), Format::Markdown)
            .unwrap()
            .attachment
    );
    assert!(
        content(&"*".repeat(1000), Format::Plain)
            .unwrap()
            .attachment
    );
    assert!(content(&"a".repeat(session::markdown::MAX_BYTES + 1), Format::Plain).is_err());
}

#[test]
fn ui_titles_are_literal_and_emoji_stay_unicode() {
    let text = content("**title** <#123> :custom: 👩‍💻\n1. name", Format::Plain).unwrap();
    assert_eq!(
        text.text,
        "\\*\\*title\\*\\* \\<\\#123\\> :custom: 👩‍💻\n1\\. name"
    );
    assert!(!text.attachment);
}
