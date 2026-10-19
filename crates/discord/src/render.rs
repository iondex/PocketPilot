//! Render parsed CommonMark into Discord's supported subset; keep exports unchanged.
use session::markdown::{Document, Format, Kind, Node, TooLarge};

pub struct Content {
    pub text: String,
    pub attachment: bool,
}

pub fn content(source: &str, format: Format) -> Result<Content, TooLarge> {
    if source.len() > session::markdown::MAX_BYTES {
        return Err(TooLarge);
    }
    let rendered = match format {
        Format::Plain => Some(literal(source)),
        Format::Markdown => nodes(Document::parse(source)?.nodes()).map(|s| s.trim_end().into()),
    };
    Ok(match rendered {
        Some(text) if session::markdown::fits(&text, 1800) => Content {
            text,
            attachment: false,
        },
        _ => Content {
            text: String::new(),
            attachment: true,
        },
    })
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

fn nodes(input: &[Node]) -> Option<String> {
    let mut result = String::new();
    for node in input {
        match node {
            Node::Text(text) => result.push_str(&literal(text)),
            Node::Code(text) => {
                // Discord's safe builder changes embedded fences. Export instead of altering code.
                if text.contains('`') {
                    return None;
                }
                result.push_str(&format!("`{text}`"));
            }
            Node::Break => result.push('\n'),
            Node::Rule => result.push_str("────────\n\n"),
            Node::Task(done) => result.push_str(if *done { "☑ " } else { "☐ " }),
            Node::Group(kind, children) => match kind {
                Kind::CodeBlock(language) => {
                    let code = session::markdown::plain(children);
                    if code.contains("```") {
                        return None;
                    }
                    result.push_str("```");
                    result.push_str(language.as_deref().unwrap_or_default());
                    result.push('\n');
                    result.push_str(&code);
                    if !code.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str("```\n\n");
                }
                Kind::Strong | Kind::Emphasis | Kind::Strike => {
                    let marker = match kind {
                        Kind::Strong => "**",
                        Kind::Emphasis => "_",
                        _ => "~~",
                    };
                    result.push_str(marker);
                    result.push_str(&nodes(children)?);
                    result.push_str(marker);
                }
                Kind::Heading(level) => {
                    let text = nodes(children)?;
                    if *level <= 3 {
                        result.push_str(&"#".repeat(usize::from(*level)));
                        result.push(' ');
                        result.push_str(&text);
                    } else {
                        result.push_str(&format!("**{text}**"));
                    }
                    result.push_str("\n\n");
                }
                Kind::Link(Some(url)) | Kind::Image(Some(url)) => {
                    let label = session::markdown::plain(children);
                    let label = if label.is_empty() { url } else { &label };
                    result.push_str(&format!("[{}]({})", literal(label), link(url)));
                }
                Kind::Quote => {
                    let text = nodes(children)?;
                    for line in text.trim_end().lines() {
                        result.push_str("> ");
                        result.push_str(line);
                        result.push('\n');
                    }
                    result.push('\n');
                }
                Kind::List(start) => {
                    for (index, item) in children.iter().enumerate() {
                        let text = nodes(std::slice::from_ref(item))?;
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
                    result.push_str(&nodes(children)?);
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

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
