//! Platform-independent Markdown structure, safe links and plain-text previews.
use pulldown_cmark::{CodeBlockKind, Event, Tag};

#[path = "markdown_input.rs"]
pub mod input;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;

const MAX_DEPTH: usize = 64;

const MAX_NODES: usize = 8192;

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Plain,
    Markdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    Strong,
    Emphasis,
    Strike,
    Quote,
    List(Option<u64>),
    Item,
    CodeBlock(Option<String>),
    Link(Option<String>),
    Image(Option<String>),
    Table,
    TableHead,
    TableRow,
    TableCell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Text(String),
    Code(String),
    Break,
    Rule,
    Task(bool),
    Group(Kind, Vec<Node>),
}

#[derive(Debug, thiserror::Error)]
#[error("Markdown source exceeds the message size budget")]
pub struct TooLarge;

pub struct Document {
    nodes: Vec<Node>,
}

impl Document {
    pub fn parse(markdown: &str) -> Result<Self, TooLarge> {
        if markdown.len() > MAX_BYTES {
            return Err(TooLarge);
        }
        let source = input::source(markdown);
        let Some(events) = input::bounded_events(&source, MAX_NODES) else {
            return Ok(Self {
                nodes: vec![Node::Text(markdown.into())],
            });
        };
        let mut root = Vec::new();
        let mut stack: Vec<(Kind, Vec<Node>)> = Vec::new();
        let mut count = 0;
        for event in events {
            count += 1;
            if count > MAX_NODES || stack.len() >= MAX_DEPTH {
                // Preserve content literally when structure is excessive; never recurse unboundedly.
                return Ok(Self {
                    nodes: vec![Node::Text(markdown.into())],
                });
            }
            let mut completed = Vec::new();
            match event {
                Event::Start(tag) => {
                    let kind = match tag {
                        Tag::Paragraph | Tag::HtmlBlock => Kind::Paragraph,
                        Tag::Heading { level, .. } => Kind::Heading(level as u8),
                        Tag::Strong => Kind::Strong,
                        Tag::Emphasis => Kind::Emphasis,
                        Tag::Strikethrough => Kind::Strike,
                        Tag::BlockQuote(_) => Kind::Quote,
                        Tag::List(start) => Kind::List(start),
                        Tag::Item => Kind::Item,
                        Tag::CodeBlock(kind) => Kind::CodeBlock(match kind {
                            CodeBlockKind::Indented => None,
                            CodeBlockKind::Fenced(info) => language(&info),
                        }),
                        Tag::Link { dest_url, .. } => Kind::Link(safe_link(&dest_url)),
                        Tag::Image { dest_url, .. } => Kind::Image(safe_link(&dest_url)),
                        Tag::Table(_) => Kind::Table,
                        Tag::TableHead => Kind::TableHead,
                        Tag::TableRow => Kind::TableRow,
                        Tag::TableCell => Kind::TableCell,
                        _ => Kind::Paragraph,
                    };
                    stack.push((kind, Vec::new()));
                    continue;
                }
                Event::End(_) => {
                    if let Some((kind, children)) = stack.pop() {
                        if kind == Kind::Table {
                            let Some(rows) = table(children, MAX_NODES - count) else {
                                return Ok(Self {
                                    nodes: vec![Node::Text(markdown.into())],
                                });
                            };
                            count += size(&rows);
                            completed = rows;
                        } else {
                            completed.push(Node::Group(kind, children));
                        }
                    }
                }
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                    completed.push(Node::Text(text.into_string()))
                }
                Event::Code(text) => completed.push(Node::Code(text.into_string())),
                Event::SoftBreak | Event::HardBreak => completed.push(Node::Break),
                Event::Rule => completed.push(Node::Rule),
                Event::TaskListMarker(done) => completed.push(Node::Task(done)),
                Event::FootnoteReference(text) => completed.push(Node::Text(format!("[{text}]"))),
                _ => {}
            }
            if let Some((_, children)) = stack.last_mut() {
                children.extend(completed);
            } else {
                root.extend(completed);
            }
        }
        Ok(Self { nodes: root })
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn plain(&self) -> String {
        plain(&self.nodes).trim_end().to_owned()
    }

    pub fn preview(&self, limit: usize) -> String {
        preview(&self.plain(), limit)
    }
}

pub fn safe_link(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https" | "mailto")
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

/// Conservative UTF-16 units also bound clients which count surrogate pairs separately.
pub fn fits(text: &str, limit: usize) -> bool {
    text.encode_utf16().take(limit.saturating_add(1)).count() <= limit
}

pub fn preview(text: &str, limit: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if fits(text, limit) {
        return text.into();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let size = grapheme.encode_utf16().count();
        if used + size + 1 > limit {
            break;
        }
        used += size;
        result.push_str(grapheme);
    }
    if limit > 0 {
        result.push('…');
    }
    result
}

pub fn plain(nodes: &[Node]) -> String {
    let mut result = String::new();
    for node in nodes {
        match node {
            Node::Text(text) | Node::Code(text) => result.push_str(text),
            Node::Break => result.push('\n'),
            Node::Rule => result.push_str("────────\n"),
            Node::Task(done) => result.push_str(if *done { "☑ " } else { "☐ " }),
            Node::Group(Kind::List(start), items) => {
                let mut index = start.unwrap_or(1);
                for item in items {
                    if start.is_some() {
                        result.push_str(&format!("{index}. "));
                    } else {
                        result.push_str("• ");
                    }
                    result.push_str(plain(std::slice::from_ref(item)).trim_end());
                    result.push('\n');
                    index = index.saturating_add(1);
                }
            }
            Node::Group(kind, children) => {
                let text = plain(children);
                result.push_str(&text);
                if let Kind::Link(Some(url)) | Kind::Image(Some(url)) = kind
                    && text != *url
                {
                    result.push_str(&format!(" ({url})"));
                }
                if matches!(
                    kind,
                    Kind::Paragraph
                        | Kind::Heading(_)
                        | Kind::Quote
                        | Kind::CodeBlock(_)
                        | Kind::Item
                ) {
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                    if matches!(kind, Kind::Paragraph | Kind::Heading(_)) {
                        result.push('\n');
                    }
                }
            }
        }
    }
    result
}

fn language(info: &str) -> Option<String> {
    let language = info.split_whitespace().next()?;
    (!language.is_empty()
        && language.len() <= 32
        && language
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_+-.".contains(&c)))
    .then(|| language.into())
}

/// Both targets display tables as labeled rows, preserving formatting and links in cells.
fn table(children: Vec<Node>, budget: usize) -> Option<Vec<Node>> {
    let mut headers = Vec::new();
    let mut rows = Vec::new();
    for node in children {
        if let Node::Group(kind, cells) = node {
            let cells = cells
                .into_iter()
                .filter_map(|node| {
                    if let Node::Group(Kind::TableCell, nodes) = node {
                        Some(nodes)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if kind == Kind::TableHead {
                headers = cells;
            } else {
                rows.push(cells);
            }
        }
    }
    if rows.is_empty() {
        rows.push(headers.clone());
        headers.clear();
    }
    let mut used = 0;
    let mut result = Vec::new();
    for row in rows {
        let mut nodes = Vec::new();
        used += 1;
        for (column, cell) in row.into_iter().enumerate() {
            // Repeated headers must fit the same budget as the parsed structure.
            used += size(&cell) + usize::from(column > 0);
            let header = headers.get(column).filter(|h| !plain(h).trim().is_empty());
            if let Some(header) = header {
                used += size(header) + 2;
            }
            if used > budget {
                return None;
            }
            if column > 0 {
                nodes.push(Node::Break);
            }
            if let Some(header) = header {
                nodes.push(Node::Group(Kind::Strong, header.clone()));
                nodes.push(Node::Text(": ".into()));
            }
            nodes.extend(cell);
        }
        result.push(Node::Group(Kind::Paragraph, nodes));
    }
    Some(result)
}

fn size(nodes: &[Node]) -> usize {
    nodes.iter().fold(0, |total, node| {
        total
            + 1
            + match node {
                Node::Group(_, children) => size(children),
                _ => 0,
            }
    })
}
