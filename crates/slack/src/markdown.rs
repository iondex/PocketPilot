//! Common Markdown nodes to ordinary Slack rich_text blocks, without executable mentions.
use session::markdown::{Document, Kind, Node};
use slack_morphism::prelude::*;

#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

impl Style {
    fn sdk(self) -> Option<SlackRichTextStyle> {
        (self.bold || self.italic || self.strike || self.code).then(|| {
            SlackRichTextStyle::new()
                .with_bold(self.bold)
                .with_italic(self.italic)
                .with_strike(self.strike)
                .with_code(self.code)
        })
    }
}

pub fn blocks(document: &Document) -> Option<Vec<SlackBlock>> {
    let elements = blocks_for(document.nodes(), 0);
    if elements.len() > 50 {
        return None;
    }
    if elements.is_empty() {
        return Some(Vec::new());
    }
    Some(vec![SlackRichTextBlock::new(elements).into()])
}

fn blocks_for(nodes: &[Node], indent: u64) -> Vec<SlackRichTextElement> {
    let mut blocks = Vec::new();
    for node in nodes {
        match node {
            Node::Group(Kind::CodeBlock(language), children) => {
                blocks.push(
                    SlackRichTextPreformatted::new(vec![
                        SlackRichTextText::new(session::markdown::plain(children)).into(),
                    ])
                    .opt_language(language.clone())
                    .into(),
                );
            }
            Node::Group(Kind::Quote, children) => {
                let mut elements = Vec::new();
                inline(children, Style::default(), &mut elements);
                if !elements.is_empty() {
                    blocks.push(SlackRichTextQuote::new(elements).into());
                }
            }
            Node::Group(Kind::List(start), items) => {
                for (index, item) in items.iter().enumerate() {
                    let children = if let Node::Group(Kind::Item, children) = item {
                        children.as_slice()
                    } else {
                        std::slice::from_ref(item)
                    };
                    let mut elements = Vec::new();
                    for child in children {
                        if matches!(child, Node::Group(Kind::List(_), _)) {
                            append_item(&mut blocks, &mut elements, *start, index, indent);
                            blocks.extend(blocks_for(std::slice::from_ref(child), indent + 1));
                        } else {
                            inline(std::slice::from_ref(child), Style::default(), &mut elements);
                        }
                    }
                    append_item(&mut blocks, &mut elements, *start, index, indent);
                }
            }
            Node::Group(Kind::Paragraph | Kind::Heading(_), children) => {
                let mut elements = Vec::new();
                inline(
                    children,
                    Style {
                        bold: matches!(node, Node::Group(Kind::Heading(_), _)),
                        ..Style::default()
                    },
                    &mut elements,
                );
                if !elements.is_empty() {
                    blocks.push(SlackRichTextSection::new(elements).into());
                }
            }
            _ => {
                let mut elements = Vec::new();
                inline(std::slice::from_ref(node), Style::default(), &mut elements);
                if !elements.is_empty() {
                    blocks.push(SlackRichTextSection::new(elements).into());
                }
            }
        }
    }
    blocks
}

fn append_item(
    blocks: &mut Vec<SlackRichTextElement>,
    elements: &mut Vec<SlackRichTextInlineElement>,
    start: Option<u64>,
    index: usize,
    indent: u64,
) {
    if elements.is_empty() {
        return;
    }
    blocks.push(
        SlackRichTextList::new(
            if start.is_some() {
                SlackRichTextListStyle::Ordered
            } else {
                SlackRichTextListStyle::Bullet
            },
            vec![SlackRichTextSection::new(std::mem::take(elements)).into()],
        )
        .with_indent(indent.min(7))
        .opt_offset(start.map(|n| n.saturating_sub(1).saturating_add(index as u64)))
        .into(),
    );
}

fn text(out: &mut Vec<SlackRichTextInlineElement>, value: &str, style: Style) {
    if value.is_empty() {
        return;
    }
    let style = style.sdk();
    if let Some(SlackRichTextInlineElement::Text(previous)) = out.last_mut()
        && previous.style == style
    {
        previous.text.push_str(value);
    } else {
        out.push(SlackRichTextText::new(value.into()).opt_style(style).into());
    }
}

fn inline(nodes: &[Node], style: Style, out: &mut Vec<SlackRichTextInlineElement>) {
    for node in nodes {
        match node {
            Node::Text(value) => text(out, value, style),
            Node::Code(value) => text(
                out,
                value,
                Style {
                    code: true,
                    ..style
                },
            ),
            Node::Break => text(out, "\n", style),
            Node::Rule => text(out, "────────", style),
            Node::Task(done) => text(out, if *done { "☑ " } else { "☐ " }, style),
            Node::Group(kind, children) => match kind {
                Kind::Strong => inline(
                    children,
                    Style {
                        bold: true,
                        ..style
                    },
                    out,
                ),
                Kind::Emphasis => inline(
                    children,
                    Style {
                        italic: true,
                        ..style
                    },
                    out,
                ),
                Kind::Strike => inline(
                    children,
                    Style {
                        strike: true,
                        ..style
                    },
                    out,
                ),
                Kind::CodeBlock(_) => inline(
                    children,
                    Style {
                        code: true,
                        ..style
                    },
                    out,
                ),
                Kind::Link(Some(url)) | Kind::Image(Some(url)) => {
                    let mut label = Vec::new();
                    inline(children, style, &mut label);
                    if label.is_empty() {
                        text(&mut label, url, style);
                    }
                    for element in label {
                        if let SlackRichTextInlineElement::Text(label) = element {
                            out.push(
                                SlackRichTextLink::new(SlackRelaxedUrl(url.clone()))
                                    .with_text(label.text.replace('\n', " "))
                                    .opt_style(label.style)
                                    .into(),
                            );
                        }
                    }
                }
                Kind::List(_) => text(
                    out,
                    &session::markdown::plain(std::slice::from_ref(node)),
                    style,
                ),
                _ => {
                    inline(children, style, out);
                    if matches!(kind, Kind::Paragraph | Kind::Item) {
                        text(out, "\n", style);
                    }
                }
            },
        }
    }
}
