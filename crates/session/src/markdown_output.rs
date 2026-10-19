//! Supported Markdown output dialects share structure, escaping and link handling.
use super::{Kind, Node};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    CommonMark,
    Discord,
}

/// Escape dynamic UI text and parsed text nodes, including Discord-specific syntax.
pub fn literal(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\*_~|`>#[]()<.!+-".contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

pub(super) fn render(input: &[Node], dialect: Dialect) -> Option<String> {
    let mut result = String::new();
    for node in input {
        match node {
            Node::Text(text) => result.push_str(&literal(text)),
            Node::Code(text) => {
                // Unsupported Discord code syntax is shown literally by the caller.
                if dialect == Dialect::Discord && text.contains('`') {
                    return None;
                }
                let fence = "`".repeat(backticks(text).saturating_add(1));
                let pad = if (text.starts_with(['`', ' ']) || text.ends_with(['`', ' ']))
                    && !text.chars().all(|c| c == ' ')
                {
                    " "
                } else {
                    ""
                };
                result.push_str(&format!("{fence}{pad}{text}{pad}{fence}"));
            }
            Node::Break => result.push('\n'),
            Node::Rule => result.push_str("────────\n\n"),
            Node::Task(done) => result.push_str(if *done { "☑ " } else { "☐ " }),
            Node::Group(kind, children) => match kind {
                Kind::CodeBlock(language) => {
                    let code = super::plain(children);
                    if dialect == Dialect::Discord && code.contains("```") {
                        return None;
                    }
                    let fence = "`".repeat(backticks(&code).saturating_add(1).max(3));
                    result.push_str(&fence);
                    result.push_str(language.as_deref().unwrap_or_default());
                    result.push('\n');
                    result.push_str(&code);
                    if !code.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str(&fence);
                    result.push_str("\n\n");
                }
                Kind::Strong | Kind::Emphasis | Kind::Strike => {
                    let marker = match kind {
                        Kind::Strong => "**",
                        Kind::Emphasis => "_",
                        _ => "~~",
                    };
                    result.push_str(marker);
                    result.push_str(&render(children, dialect)?);
                    result.push_str(marker);
                }
                Kind::Heading(level) => {
                    let text = render(children, dialect)?;
                    if *level <= 3 || dialect == Dialect::CommonMark {
                        result.push_str(&"#".repeat(usize::from(*level)));
                        result.push(' ');
                        result.push_str(&text);
                    } else {
                        result.push_str(&format!("**{text}**"));
                    }
                    result.push_str("\n\n");
                }
                Kind::Link(Some(url)) | Kind::Image(Some(url)) => {
                    let label = super::plain(children);
                    let label = if label.is_empty() { url } else { &label };
                    result.push_str(&format!("[{}]({})", literal(label), link(url)));
                }
                Kind::Quote => {
                    let text = render(children, dialect)?;
                    for line in text.trim_end().lines() {
                        result.push_str("> ");
                        result.push_str(line);
                        result.push('\n');
                    }
                    result.push('\n');
                }
                Kind::List(start) => {
                    for (index, item) in children.iter().enumerate() {
                        let text = render(std::slice::from_ref(item), dialect)?;
                        let prefix = start.map_or_else(
                            || "- ".to_string(),
                            |n| format!("{}. ", n.saturating_add(index as u64)),
                        );
                        for (line, value) in text.trim_end().lines().enumerate() {
                            result.push_str(if line == 0 { &prefix } else { "  " });
                            result.push_str(value);
                            result.push('\n');
                        }
                    }
                    result.push('\n');
                }
                _ => {
                    result.push_str(&render(children, dialect)?);
                    if matches!(kind, Kind::Paragraph | Kind::Item) {
                        result.push_str("\n\n");
                    }
                }
            },
        }
    }
    Some(result)
}

fn link(url: &str) -> String {
    let mut result = String::new();
    for byte in url.bytes() {
        if b"()[]<>\\".contains(&byte) || byte.is_ascii_control() || byte == b' ' || byte >= 128 {
            result.push_str(&format!("%{byte:02X}"));
        } else {
            result.push(char::from(byte));
        }
    }
    result
}

fn backticks(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}
