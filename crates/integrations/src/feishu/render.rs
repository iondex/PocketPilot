//! Card JSON 2.0. Agent Markdown is parsed before native components are serialized.
use crate::rendering::{Capabilities, RenderBatch, RenderRequest, Renderer};
use serde::{Deserialize, Serialize};
use session::markdown::{Dialect, Document, Format, TooLarge};

#[derive(Clone, Serialize, Deserialize)]
pub struct Card {
    schema: String,
    config: CardConfig,
    header: Header,
    body: Body,
}

#[derive(Clone, Serialize, Deserialize)]
struct CardConfig {
    update_multi: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Header {
    title: Text,
    template: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Body {
    elements: Vec<Element>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Text {
    tag: String,
    content: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum Element {
    Markdown {
        content: String,
    },
    Button {
        name: String,
        text: Text,
        #[serde(rename = "type")]
        style: String,
        behaviors: Vec<Behavior>,
        #[serde(skip_serializing_if = "Option::is_none")]
        form_action_type: Option<String>,
    },
    Form {
        name: String,
        elements: Vec<Element>,
    },
    Input {
        name: String,
        input_type: String,
        placeholder: Text,
        required: bool,
        max_length: u32,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Behavior {
    #[serde(rename = "type")]
    kind: String,
    value: ActionValue,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActionValue {
    pub nonce: String,
}

impl Text {
    fn plain(content: &str) -> Self {
        Self {
            tag: "plain_text".into(),
            content: session::delivery::short_title(content, 100),
        }
    }
}

impl Card {
    pub fn new(title: &str) -> Self {
        Self {
            schema: "2.0".into(),
            config: CardConfig { update_multi: true },
            header: Header {
                title: Text::plain(title),
                template: "blue".into(),
            },
            body: Body { elements: vec![] },
        }
    }

    pub fn markdown(&mut self, content: String) {
        self.body.elements.push(Element::Markdown { content });
    }

    pub fn button(&mut self, label: &str, nonce: &str) {
        self.body.elements.push(button(label, nonce, false));
    }

    pub fn title_form(&mut self, label: &str, submit: &str, nonce: &str) {
        self.body.elements.push(Element::Form {
            name: "new_session".into(),
            elements: vec![
                Element::Input {
                    name: "title".into(),
                    input_type: "text".into(),
                    placeholder: Text::plain(label),
                    required: true,
                    max_length: 80,
                },
                button(submit, nonce, true),
            ],
        });
    }

    pub fn bounded(&self) -> bool {
        let Ok(content) = serde_json::to_string(self) else {
            return false;
        };
        self.body.elements.len() <= 128
            && serde_json::to_vec(&content).is_ok_and(|b| b.len() + 1024 <= 28_000)
    }
}

fn button(label: &str, nonce: &str, submit: bool) -> Element {
    Element::Button {
        name: format!("action_{}", nonce.replace('-', "_")),
        text: Text::plain(label),
        style: if submit { "primary" } else { "default" }.into(),
        behaviors: vec![Behavior {
            kind: "callback".into(),
            value: ActionValue {
                nonce: nonce.into(),
            },
        }],
        form_action_type: submit.then(|| "submit".into()),
    }
}

pub struct CardRenderer;

impl Renderer for CardRenderer {
    type Part = Card;

    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(crate::Platform::Feishu)
    }

    fn render(&self, request: RenderRequest<'_>) -> Result<RenderBatch<Card>, TooLarge> {
        let source = if request.format == Format::Plain {
            session::markdown::literal(request.source)
        } else {
            request.source.into()
        };
        let document = Document::parse(&source)?;
        let heading = request
            .heading
            .map(Document::parse)
            .transpose()?
            .map(|document| document.plain());
        let heading = heading
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or("PocketPilot");
        let mut parts = Vec::new();
        render_parts(
            &document,
            heading,
            self.capabilities().text_budget,
            &mut parts,
        )?;
        Ok(RenderBatch {
            format: self.capabilities().format,
            parts,
        })
    }
}

/// Prefer compact rich cards; reduce semantic chunk size only when final JSON
/// escaping or structural overhead actually exceeds the native payload budget.
fn render_parts(
    document: &Document,
    heading: &str,
    units: usize,
    parts: &mut Vec<Card>,
) -> Result<(), TooLarge> {
    for chunk in document.chunks(units, 100) {
        let mut card = Card::new(heading);
        for block in chunk.blocks() {
            let body = if tables(block.nodes()) > 4 {
                literal(&block.plain())
            } else {
                block
                    .markdown(Dialect::Feishu)
                    .unwrap_or_else(|| literal(&block.plain()))
            };
            card.markdown(body);
        }
        if card.bounded() {
            parts.push(card);
        } else if units > 64 {
            render_parts(&chunk, heading, units / 2, parts)?;
        } else {
            for text in session::markdown::split_text(&chunk.plain(), 32) {
                let mut card = Card::new(heading);
                card.markdown(literal(&text));
                if !card.bounded() || parts.len() >= 256 {
                    return Err(TooLarge);
                }
                parts.push(card);
            }
        }
        if parts.len() > 256 {
            return Err(TooLarge);
        }
    }
    Ok(())
}

/// UI values cannot introduce Feishu's native at/person tags.
pub fn literal(source: &str) -> String {
    session::markdown::literal(
        &source
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;"),
    )
}

fn tables(nodes: &[session::markdown::Node]) -> usize {
    use session::markdown::{Kind, Node};
    nodes
        .iter()
        .map(|node| match node {
            Node::Group(Kind::Table(_), children) => 1 + tables(children),
            Node::Group(_, children) => tables(children),
            _ => 0,
        })
        .sum()
}
