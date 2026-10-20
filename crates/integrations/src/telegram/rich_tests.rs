use super::*;

fn render(source: &str) -> RenderBatch<Part> {
    RichRenderer
        .render(RenderRequest {
            source,
            format: Format::Markdown,
            heading: None,
        })
        .unwrap()
}

#[test]
fn headings_cjk_styles_and_code_are_native_blocks() {
    let batch = render("# 结果\n\n**中文粗体** 和 *斜体*\n\n```rust\nlet x = 1;\n```\n");
    let message = &batch.parts[0].message;
    assert!(message.skip_entity_detection);
    assert!(
        matches!(&message.blocks[0], Block::Heading { size: 1, text } if text.plain() == "结果")
    );
    assert!(
        matches!(&message.blocks[2], Block::Pre { language: Some(language), text } if language == "rust" && text.plain() == "let x = 1;\n")
    );
    let payload = serde_json::to_string(message).unwrap();
    assert!(payload.contains("\"type\":\"bold\""));
    assert!(payload.contains("\"type\":\"italic\""));
    assert!(message.plain().contains("中文粗体 和 斜体"));
}

#[test]
fn native_tables_preserve_headers_alignment_and_links() {
    let batch =
        render("| **Name** | Value |\n| :---: | ---: |\n| [Docs](https://example.com) | 42 |\n");
    let Block::Table { cells, .. } = &batch.parts[0].message.blocks[0] else {
        panic!("expected a native table");
    };
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0][0].is_header, Some(true));
    assert_eq!(cells[0][0].align, "center");
    assert_eq!(cells[1][1].align, "right");
    assert_eq!(cells[1][0].text.plain(), "Docs (https://example.com/)");
    assert!(batch.parts[0].message.plain().contains("Name: Docs"));
}

#[test]
fn math_and_footnotes_keep_native_semantics() {
    let batch = render("Inline $x^2$ and note[^one].\n\n$$x + y$$\n\n[^one]: Footnote body.\n");
    let payload = serde_json::to_string(&batch.parts[0].message).unwrap();
    for kind in ["mathematical_expression", "reference_link", "reference"] {
        assert!(payload.contains(&format!("\"type\":\"{kind}\"")));
    }
    let plain = batch.parts[0].message.plain();
    assert!(plain.contains("x^2") && plain.contains("x + y"));
    assert!(plain.contains("[one]: Footnote body."));
}

#[test]
fn supported_html_styles_and_disclosures_become_typed_nodes() {
    let batch = render(
        "Text <u>under</u> <sub>low</sub> <sup>high</sup> <mark>marked</mark> <tg-spoiler>secret</tg-spoiler>.\n\n<details open><summary>Details</summary>\n\n**Body**\n\n</details>\n",
    );
    let message = &batch.parts[0].message;
    let payload = serde_json::to_string(message).unwrap();
    for kind in ["underline", "subscript", "superscript", "marked", "spoiler"] {
        assert!(payload.contains(&format!("\"type\":\"{kind}\"")));
    }
    assert!(message.blocks.iter().any(|block| matches!(block, Block::Details { summary, blocks, is_open: true } if summary.plain() == "Details" && blocks.iter().any(|block| block.plain() == "Body"))));
}

#[test]
fn untrusted_html_and_unsafe_urls_never_become_executable_styles() {
    let batch = render(
        "<script>alert(1)</script>\n\n[unsafe](javascript:alert%281%29) [safe](https://example.com) <u>unclosed\n",
    );
    let message = &batch.parts[0].message;
    assert!(message.plain().contains("<script>alert(1)</script>"));
    assert!(message.plain().contains("<u>unclosed"));
    let payload = serde_json::to_string(message).unwrap();
    assert!(!payload.contains("javascript:"));
    assert!(payload.contains("https://example.com/"));
}

#[test]
fn long_output_retains_content_with_a_heading_on_every_fragment() {
    let body = "中文🙂 ".repeat(4000);
    let batch = RichRenderer
        .render(RenderRequest {
            source: &body,
            format: Format::Plain,
            heading: Some("✅ Result"),
        })
        .unwrap();
    assert!(batch.parts.len() > 2);
    let mut recovered = String::new();
    for part in batch.parts {
        assert!(part.text.encode_utf16().count() <= 12_000);
        recovered.push_str(part.text.strip_prefix("✅ Result\n\n").unwrap());
        assert!(part.message.plain().starts_with("✅ Result"));
    }
    assert_eq!(recovered, body);
}

#[test]
fn deeply_nested_output_uses_literal_content_within_native_budget() {
    let source = format!("{}body\n", "> ".repeat(18));
    let batch = render(&source);
    assert!(
        matches!(&batch.parts[0].message.blocks[..], [Block::Paragraph { text }] if text.plain().contains("body"))
    );
}
