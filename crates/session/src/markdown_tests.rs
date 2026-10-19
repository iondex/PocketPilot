use super::*;

#[test]
fn common_structure_preserves_nested_styles_code_and_plain_fallback() {
    let doc = Document::parse("# Title\n\n**Bold _italic_** and `a < b`\n\n```rust\nlet x = \"<&>\";\n```\n\n- [x] done\n- [ ] pending").unwrap();
    assert!(matches!(&doc.nodes()[0], Node::Group(Kind::Heading(1), _)));
    let plain = doc.plain();
    assert!(plain.contains("Bold italic"));
    assert!(plain.contains("let x = \"<&>\";"));
    assert!(plain.contains("☑ done"));
    assert!(plain.contains("☐ pending"));
    assert!(!plain.contains("**"));
}

#[test]
fn table_rows_keep_headers_and_safe_links_while_rejecting_active_content() {
    let doc = Document::parse("| Name | Link |\n|---|---|\n| Alpha | [docs](https://example.com) |\n\n[bad](javascript:alert%281%29) <script>alert(1)</script>").unwrap();
    let text = doc.plain();
    assert!(text.contains("Name: Alpha"));
    assert!(text.contains("Link: docs (https://example.com/)"));
    assert!(text.contains("<script>"));
    assert!(!text.contains("javascript:"));
    assert!(safe_link("data:text/html,unsafe").is_none());
    assert!(safe_link("https://user:password@example.com").is_none());
}

#[test]
fn cjk_bold_and_literal_markers_share_the_existing_telegram_parser() {
    let doc = Document::parse("前文**中文。**后文\n\n`**literal**`\n\n\\*escaped\\*").unwrap();
    assert!(doc.plain().contains("前文中文。后文"));
    assert!(doc.plain().contains("**literal**"));
    assert!(doc.plain().contains("*escaped*"));
    let Node::Group(_, children) = &doc.nodes()[0] else {
        panic!("paragraph")
    };
    assert!(
        children
            .iter()
            .any(|n| matches!(n, Node::Group(Kind::Strong, _)))
    );
}

#[test]
fn previews_do_not_split_emoji_sequences_and_input_budgets_preserve_literal_content() {
    let family = "👩‍👩‍👧‍👦";
    assert_eq!(preview(&format!("a{family}b"), 8), "a…");
    assert!(!fits("😀😀", 3));
    assert_eq!(preview("abc", 0), "");
    let excessive = "*word* ".repeat(MAX_NODES);
    let parsed = Document::parse(&excessive).unwrap();
    assert_eq!(parsed.plain(), excessive.trim_end());
    assert!(Document::parse(&"x".repeat(MAX_BYTES + 1)).is_err());
}

#[test]
fn repeated_table_headers_cannot_expand_past_the_structure_budget() {
    let source = format!(
        "| {} |\n|---|\n{}",
        "**header** ".repeat(70),
        "| value |\n".repeat(100)
    );
    let parsed = Document::parse(&source).unwrap();
    assert_eq!(parsed.plain(), source.trim_end());
}

#[test]
fn message_chunks_preserve_all_text_styles_code_and_emoji_in_order() {
    fn text(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|n| match n {
                Node::Text(s) | Node::Code(s) => s.clone(),
                Node::Group(_, nodes) => text(nodes),
                _ => String::new(),
            })
            .collect()
    }
    let source = format!(
        "# Header\n\n**{}**\n\n```rust\n{}\n```\n\n3. item\n4. next",
        "👩‍💻 x ".repeat(300),
        "let n = 42;\n".repeat(300)
    );
    let document = Document::parse(&source).unwrap();
    let chunks = document.chunks(400, 32);
    assert!(chunks.len() > 10);
    assert_eq!(
        chunks.iter().map(|d| text(d.nodes())).collect::<String>(),
        text(document.nodes())
    );
    assert!(
        chunks
            .iter()
            .all(|d| fits(&d.plain(), 400) && size(d.nodes()) <= 32)
    );
    assert!(
        chunks
            .iter()
            .any(|d| matches!(d.nodes().first(), Some(Node::Group(Kind::CodeBlock(_), _))))
    );
}
