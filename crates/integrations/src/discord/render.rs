//! Render Discord conversation roles and parsed CommonMark into bounded message content.
use crate::rendering::{Capabilities, RenderBatch, RenderRequest, Renderer};
pub use session::markdown::literal;
use session::markdown::{Document, Format, TooLarge};

pub struct ComponentRenderer;

impl Renderer for ComponentRenderer {
    type Part = crate::discord::native::Message;

    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(crate::Platform::Discord)
    }

    fn render(&self, request: RenderRequest<'_>) -> Result<RenderBatch<Self::Part>, TooLarge> {
        let header = request
            .heading
            .map(|heading| format!("{heading}\n\n"))
            .unwrap_or_default();
        let mut parts = Vec::new();
        if request.format == Format::Plain {
            parts.extend(
                with_header(request.source, Format::Plain, &header)?
                    .into_iter()
                    .map(Self::Part::text),
            );
        } else {
            for document in Document::parse(request.source)?.chunks(700, 24) {
                let mut images = Vec::new();
                collect_images(document.nodes(), &mut images);
                let text = document.markdown(session::markdown::Dialect::Discord);
                match text {
                    Some(text)
                        if session::markdown::fits(
                            &text,
                            1800_usize.saturating_sub(header.encode_utf16().count()),
                        ) =>
                    {
                        parts.push(Self::Part::with_images(format!("{header}{text}"), images));
                    }
                    _ => {
                        for text in session::markdown::split_text(&document.plain(), 800) {
                            parts.push(Self::Part::with_images(
                                format!("{header}{}", literal(&text)),
                                std::mem::take(&mut images),
                            ));
                        }
                    }
                }
            }
        }
        Ok(RenderBatch {
            format: self.capabilities().format,
            parts,
        })
    }
}

fn collect_images(nodes: &[session::markdown::Node], output: &mut Vec<(String, String)>) {
    use session::markdown::{Kind, Node};
    for node in nodes {
        if let Node::Group(kind, children) = node {
            if let Kind::Image(Some(url)) = kind
                && url.starts_with("https://")
            {
                output.push((url.clone(), session::markdown::plain(children)));
            } else {
                collect_images(children, output);
            }
        }
    }
}

pub fn messages(source: &str, format: Format) -> Result<Vec<String>, TooLarge> {
    with_header(source, format, "")
}

fn with_header(source: &str, format: Format, header: &str) -> Result<Vec<String>, TooLarge> {
    if source.len() > session::markdown::MAX_BYTES {
        return Err(TooLarge);
    }
    if format == Format::Plain {
        return Ok(session::markdown::split_text(source, 800)
            .into_iter()
            .map(|s| format!("{header}{}", literal(&s)))
            .collect());
    }
    let mut result = Vec::new();
    for part in Document::parse(source)?.chunks(700, 64) {
        let text = part.markdown(session::markdown::Dialect::Discord);
        match text {
            Some(text) if session::markdown::fits(&text, 1800 - header.encode_utf16().count()) => {
                result.push(text)
            }
            _ => result.extend(
                session::markdown::split_text(&part.plain(), 800)
                    .into_iter()
                    .map(|s| literal(&s)),
            ),
        }
    }
    Ok(result
        .into_iter()
        .map(|text| format!("{header}{text}"))
        .collect())
}

/// Freeze each role's rendered parts together with its readable outbox payload.
#[derive(Default)]
pub(crate) struct Conversation {
    pub text: String,
    pub parts: Vec<crate::discord::native::Message>,
}

impl Conversation {
    pub(crate) fn latest(history: &session::History, locale: &str) -> Result<Self, TooLarge> {
        Self::sections(session::presentation::latest(history, locale))
    }

    pub(crate) fn changes<'a>(
        items: impl Iterator<Item = &'a session::TextItem>,
        locale: &str,
    ) -> Result<Self, TooLarge> {
        Self::sections(items.map(|item| session::presentation::Section::item(item, locale)))
    }

    fn sections(
        sections: impl IntoIterator<Item = session::presentation::Section>,
    ) -> Result<Self, TooLarge> {
        let mut output = Self::default();
        for section in sections {
            let header = format!("{}\n\n", section.heading);
            if output.text.len() + header.len() + section.body.len() + 2
                > session::markdown::MAX_BYTES
            {
                return Err(TooLarge);
            }
            let parts = ComponentRenderer
                .render(RenderRequest {
                    source: &section.body,
                    format: Format::Markdown,
                    heading: Some(&section.heading),
                })?
                .parts;
            if !output.text.is_empty() {
                output.text.push_str("\n\n");
            }
            output.text.push_str(&header);
            output.text.push_str(&section.body);
            output.parts.extend(parts);
        }
        Ok(output)
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
