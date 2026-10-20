//! Typed Telegram RichMessage output; agent HTML is never forwarded as trusted markup.
use crate::rendering::{Capabilities, RenderBatch, RenderRequest, Renderer};
use serde::{Deserialize, Serialize};
use session::markdown::{Alignment, Document, Format, Kind, Node, TooLarge};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Sequence(Vec<Text>),
    Styled(Box<Style>),
}

impl Text {
    pub fn plain(&self) -> String {
        match self {
            Self::Plain(text) => text.clone(),
            Self::Sequence(parts) => parts.iter().map(Text::plain).collect(),
            Self::Styled(style) => match style.as_ref() {
                Style::Bold { text }
                | Style::Italic { text }
                | Style::Strikethrough { text }
                | Style::Code { text }
                | Style::Underline { text }
                | Style::Subscript { text }
                | Style::Superscript { text }
                | Style::Marked { text }
                | Style::Spoiler { text }
                | Style::ReferenceLink { text, .. } => text.plain(),
                Style::Url { text, url } => {
                    let text = text.plain();
                    if text == *url {
                        text
                    } else {
                        format!("{text} ({url})")
                    }
                }
                Style::MathematicalExpression { expression } => expression.clone(),
                Style::Reference { text, name } => format!("[{name}]: {}", text.plain()),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Style {
    Bold { text: Text },
    Italic { text: Text },
    Underline { text: Text },
    Subscript { text: Text },
    Superscript { text: Text },
    Marked { text: Text },
    Spoiler { text: Text },
    Strikethrough { text: Text },
    Code { text: Text },
    Url { text: Text, url: String },
    MathematicalExpression { expression: String },
    Reference { text: Text, name: String },
    ReferenceLink { text: Text, reference_name: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub text: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_header: Option<bool>,
    pub align: String,
    pub valign: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListItem {
    pub blocks: Vec<Block>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_checkbox: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub label_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        text: Text,
    },
    Heading {
        text: Text,
        size: u8,
    },
    Pre {
        text: Text,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    Blockquote {
        blocks: Vec<Block>,
    },
    Details {
        summary: Text,
        blocks: Vec<Block>,
        is_open: bool,
    },
    List {
        items: Vec<ListItem>,
    },
    Table {
        cells: Vec<Vec<Cell>>,
        is_bordered: bool,
        is_striped: bool,
    },
    MathematicalExpression {
        expression: String,
    },
    Divider,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub blocks: Vec<Block>,
    pub skip_entity_detection: bool,
}

impl Message {
    pub fn plain(&self) -> String {
        self.blocks
            .iter()
            .map(Block::plain)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Block {
    fn plain(&self) -> String {
        match self {
            Self::Paragraph { text } | Self::Heading { text, .. } | Self::Pre { text, .. } => {
                text.plain()
            }
            Self::Blockquote { blocks } => blocks
                .iter()
                .map(Block::plain)
                .collect::<Vec<_>>()
                .join("\n\n"),
            Self::Details {
                summary, blocks, ..
            } => format!(
                "{}\n\n{}",
                summary.plain(),
                blocks
                    .iter()
                    .map(Block::plain)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            ),
            Self::List { items } => items
                .iter()
                .map(|item| {
                    let marker = if item.has_checkbox == Some(true) {
                        if item.is_checked == Some(true) {
                            "☑ ".into()
                        } else {
                            "☐ ".into()
                        }
                    } else {
                        item.value
                            .map_or_else(|| "• ".into(), |value| format!("{value}. "))
                    };
                    format!(
                        "{marker}{}",
                        item.blocks
                            .iter()
                            .map(Block::plain)
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Table { cells, .. } => {
                let headers = cells
                    .first()
                    .filter(|row| row.iter().any(|cell| cell.is_header == Some(true)));
                cells
                    .iter()
                    .skip(usize::from(headers.is_some()))
                    .map(|row| {
                        row.iter()
                            .enumerate()
                            .map(|(column, cell)| {
                                headers.and_then(|headers| headers.get(column)).map_or_else(
                                    || cell.text.plain(),
                                    |header| {
                                        format!("{}: {}", header.text.plain(), cell.text.plain())
                                    },
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            }
            Self::MathematicalExpression { expression } => expression.clone(),
            Self::Divider => "────────".into(),
        }
    }

    fn cost(&self) -> usize {
        1 + match self {
            Self::Blockquote { blocks } | Self::Details { blocks, .. } => {
                blocks.iter().map(Block::cost).sum()
            }
            Self::List { items } => items
                .iter()
                .map(|item| 1 + item.blocks.iter().map(Block::cost).sum::<usize>())
                .sum(),
            Self::Table { cells, .. } => cells.len(),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub message: Message,
    pub text: String,
}

pub struct RichRenderer;

impl Renderer for RichRenderer {
    type Part = Part;

    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(crate::Platform::Telegram)
    }

    fn render(&self, request: RenderRequest<'_>) -> Result<RenderBatch<Part>, TooLarge> {
        let mut parts = Vec::new();
        if request.format == Format::Plain {
            if request.source.len() > session::markdown::MAX_BYTES {
                return Err(TooLarge);
            }
            for text in session::markdown::split_text(request.source, 6000) {
                parts.push(part(
                    vec![Block::Paragraph {
                        text: Text::Plain(text.clone()),
                    }],
                    text,
                    request.heading,
                )?);
            }
        } else {
            let source = Document::parse(request.source)?;
            let definitions = source
                .nodes()
                .iter()
                .filter_map(|node| {
                    if let Node::Group(Kind::FootnoteDefinition(name), _) = node {
                        Some((name, node))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for document in source.chunks(6000, 240) {
                let text = document
                    .markdown(session::markdown::Dialect::CommonMark)
                    .unwrap_or_else(|| document.plain());
                // RichMessage has a stricter nesting budget than the common parser.
                let mut blocks = if depth(document.nodes()) > 12 {
                    vec![Block::Paragraph {
                        text: Text::Plain(document.plain()),
                    }]
                } else {
                    blocks(document.nodes())
                };
                let mut references = Vec::new();
                references_in(document.nodes(), &mut references);
                for (name, definition) in &definitions {
                    if references.contains(&name.as_str()) && !document.nodes().iter().any(|node| matches!(node, Node::Group(Kind::FootnoteDefinition(saved), _) if saved == *name)) {
                        blocks.extend(self::blocks(std::slice::from_ref(*definition)));
                    }
                }
                if blocks.iter().map(Block::cost).sum::<usize>() > 390
                    || blocks
                        .iter()
                        .map(|block| block.plain().encode_utf16().count())
                        .sum::<usize>()
                        > 11_000
                {
                    blocks = vec![Block::Paragraph {
                        text: Text::Plain(document.plain()),
                    }];
                }
                parts.push(part(blocks, text, request.heading)?);
            }
        }
        Ok(RenderBatch {
            format: self.capabilities().format,
            parts,
        })
    }
}

fn part(mut blocks: Vec<Block>, mut text: String, heading: Option<&str>) -> Result<Part, TooLarge> {
    if let Some(heading) = heading {
        let title = Document::parse(heading)?;
        // Phase labels may be separate parser paragraphs; the repeated title stays one line.
        let mut title_parts = Vec::new();
        for node in title.nodes() {
            if !title_parts.is_empty() {
                title_parts.push(Text::Plain(" ".into()));
            }
            title_parts.push(inline(std::slice::from_ref(node)));
        }
        blocks.insert(
            0,
            Block::Paragraph {
                text: Text::Sequence(title_parts),
            },
        );
        text = format!("{heading}\n\n{text}");
    }
    if text.encode_utf16().count() > 12_000 {
        return Err(TooLarge);
    }
    Ok(Part {
        message: Message {
            blocks,
            skip_entity_detection: true,
        },
        text,
    })
}

fn depth(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| match node {
            Node::Group(_, children) => 1 + depth(children),
            _ => 1,
        })
        .max()
        .unwrap_or(0)
}

fn blocks(nodes: &[Node]) -> Vec<Block> {
    let mut result = Vec::new();
    for node in nodes {
        match node {
            Node::Rule => result.push(Block::Divider),
            Node::Math {
                source,
                display: true,
            } => result.push(Block::MathematicalExpression {
                expression: source.clone(),
            }),
            Node::Group(Kind::Paragraph, children)
                if matches!(children.as_slice(), [Node::Math { display: true, .. }]) =>
            {
                result.extend(blocks(children))
            }
            Node::Group(Kind::Heading(size), children) => result.push(Block::Heading {
                text: inline(children),
                size: *size,
            }),
            Node::Group(Kind::CodeBlock(language), children) => result.push(Block::Pre {
                text: Text::Plain(session::markdown::plain(children)),
                language: language.clone(),
            }),
            Node::Group(Kind::Quote, children) => result.push(Block::Blockquote {
                blocks: blocks(children),
            }),
            Node::Group(Kind::Details { summary, open }, children) => result.push(Block::Details {
                summary: Text::Plain(summary.clone()),
                blocks: blocks(children),
                is_open: *open,
            }),
            Node::Group(Kind::List(start), items) => {
                let items = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let children = if let Node::Group(Kind::Item, children) = item {
                            children.as_slice()
                        } else {
                            std::slice::from_ref(item)
                        };
                        let task = task(children);
                        ListItem {
                            blocks: blocks(children),
                            has_checkbox: task.map(|_| true),
                            is_checked: task.filter(|checked| *checked),
                            value: start.map(|value| value.saturating_add(index as u64)),
                            label_type: start.map(|_| "1".into()),
                        }
                    })
                    .collect();
                result.push(Block::List { items });
            }
            Node::Group(Kind::Table(alignments), children) if alignments.len() <= 20 => {
                let cells = children
                    .iter()
                    .filter_map(|row| {
                        let Node::Group(kind, cells) = row else {
                            return None;
                        };
                        Some(
                            cells
                                .iter()
                                .enumerate()
                                .filter_map(|(column, cell)| {
                                    let Node::Group(Kind::TableCell, text) = cell else {
                                        return None;
                                    };
                                    Some(Cell {
                                        text: inline(text),
                                        is_header: (*kind == Kind::TableHead).then_some(true),
                                        align: match alignments.get(column) {
                                            Some(Alignment::Center) => "center",
                                            Some(Alignment::Right) => "right",
                                            _ => "left",
                                        }
                                        .into(),
                                        valign: "top".into(),
                                    })
                                })
                                .collect(),
                        )
                    })
                    .collect();
                result.push(Block::Table {
                    cells,
                    is_bordered: true,
                    is_striped: true,
                });
            }
            Node::Group(Kind::Table(_), children) => {
                result.extend(blocks(&session::markdown::table_rows(children)))
            }
            Node::Group(Kind::FootnoteDefinition(name), children) => {
                result.push(Block::Paragraph {
                    text: Text::Styled(Box::new(Style::Reference {
                        text: inline(children),
                        name: name.clone(),
                    })),
                })
            }
            Node::Group(Kind::Item, children) => result.extend(blocks(children)),
            _ => result.push(Block::Paragraph {
                text: inline(std::slice::from_ref(node)),
            }),
        }
    }
    result
}

fn task(nodes: &[Node]) -> Option<bool> {
    nodes.iter().find_map(|node| match node {
        Node::Task(checked) => Some(*checked),
        Node::Group(Kind::Paragraph | Kind::Item, children) => task(children),
        _ => None,
    })
}

fn references_in<'a>(nodes: &'a [Node], names: &mut Vec<&'a str>) {
    for node in nodes {
        match node {
            Node::FootnoteReference(name) => names.push(name),
            Node::Group(_, children) => references_in(children, names),
            _ => {}
        }
    }
}

fn inline(nodes: &[Node]) -> Text {
    Text::Sequence(
        nodes
            .iter()
            .filter_map(|node| {
                Some(match node {
                    Node::Text(text) | Node::RawHtml(text) => Text::Plain(text.clone()),
                    Node::Code(text) => Text::Styled(Box::new(Style::Code {
                        text: Text::Plain(text.clone()),
                    })),
                    Node::Break => Text::Plain("\n".into()),
                    Node::Rule => Text::Plain("────────".into()),
                    Node::Task(_) => return None,
                    Node::Math { source, .. } => {
                        Text::Styled(Box::new(Style::MathematicalExpression {
                            expression: source.clone(),
                        }))
                    }
                    Node::FootnoteReference(name) => Text::Styled(Box::new(Style::ReferenceLink {
                        text: Text::Plain(format!("[{name}]")),
                        reference_name: name.clone(),
                    })),
                    Node::Group(kind, children) => {
                        let text = inline(children);
                        let style = match kind {
                            Kind::Strong => Style::Bold { text },
                            Kind::Emphasis => Style::Italic { text },
                            Kind::Underline => Style::Underline { text },
                            Kind::Subscript => Style::Subscript { text },
                            Kind::Superscript => Style::Superscript { text },
                            Kind::Marked => Style::Marked { text },
                            Kind::Spoiler => Style::Spoiler { text },
                            Kind::Strike => Style::Strikethrough { text },
                            Kind::Link(Some(url)) | Kind::Image(Some(url)) => Style::Url {
                                text,
                                url: url.clone(),
                            },
                            _ => return Some(text),
                        };
                        Text::Styled(Box::new(style))
                    }
                })
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "rich_tests.rs"]
mod tests;
