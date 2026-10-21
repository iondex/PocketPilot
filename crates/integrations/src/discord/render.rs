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
        let limit = self.capabilities().text_budget;
        let fragments = fragments(request.source, request.format, &header, limit)?;
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut images = Vec::new();
        for fragment in fragments {
            let separator = if text.is_empty() {
                ""
            } else {
                fragment.separator
            };
            if !text.is_empty()
                && (header.encode_utf16().count()
                    + text.encode_utf16().count()
                    + separator.len()
                    + fragment.text.encode_utf16().count()
                    > limit
                    || images.len() + fragment.images.len() > 10)
            {
                parts.push(Self::Part::with_images(
                    display_text(&header, &text),
                    std::mem::take(&mut images),
                ));
                text.clear();
            }
            if !text.is_empty() {
                text.push_str(fragment.separator);
            }
            text.push_str(&fragment.text);
            // One gallery per message keeps both media and component budgets explicit.
            for gallery in fragment.images.chunks(10) {
                if images.len() + gallery.len() > 10 {
                    parts.push(Self::Part::with_images(
                        display_text(&header, &text),
                        std::mem::take(&mut images),
                    ));
                    text.clear();
                }
                images.extend_from_slice(gallery);
            }
        }
        if !text.is_empty() || !images.is_empty() {
            parts.push(Self::Part::with_images(
                display_text(&header, &text),
                images,
            ));
        }
        Ok(RenderBatch {
            format: self.capabilities().format,
            parts,
        })
    }
}

fn display_text(header: &str, text: &str) -> String {
    if header.is_empty() && text.is_empty() {
        "🖼️".into()
    } else {
        format!("{header}{text}")
    }
}

struct Fragment {
    text: String,
    images: Vec<(String, String)>,
    separator: &'static str,
}

/// Split only when the final escaped Markdown exceeds the native text budget.
/// Parser nodes describe formatting, not Discord components, so they get a separate generous bound.
fn fragments(
    source: &str,
    format: Format,
    header: &str,
    limit: usize,
) -> Result<Vec<Fragment>, TooLarge> {
    if source.len() > session::markdown::MAX_BYTES
        || header.encode_utf16().count().saturating_add(32) > limit
    {
        return Err(TooLarge);
    }
    let budget = limit - header.encode_utf16().count();
    let mut result = Vec::new();
    if format == Format::Plain {
        literal_fragments(source, budget, Vec::new(), &mut result);
        return Ok(result);
    }
    let document = Document::parse(source)?;
    let mut queue = document
        .chunks(budget, 2048)
        .into_iter()
        .map(|document| (document, 0))
        .collect::<std::collections::VecDeque<_>>();
    while let Some((document, depth)) = queue.pop_front() {
        let rendered = document.markdown(session::markdown::Dialect::Discord);
        if let Some(text) = &rendered {
            if session::markdown::fits(text, budget) {
                let mut images = Vec::new();
                collect_images(document.nodes(), &mut images);
                result.push(Fragment {
                    text: text.clone(),
                    images,
                    separator: "\n\n",
                });
                continue;
            }
            let plain = document.plain().encode_utf16().count();
            if depth < 8 && plain > 32 {
                let target = (budget.saturating_mul(plain) / text.encode_utf16().count())
                    .saturating_sub(32)
                    .clamp(32, plain - 1);
                for fragment in document.chunks(target, 2048).into_iter().rev() {
                    queue.push_front((fragment, depth + 1));
                }
                continue;
            }
        }
        let mut images = Vec::new();
        collect_images(document.nodes(), &mut images);
        literal_fragments(&document.plain(), budget, images, &mut result);
    }
    Ok(result)
}

fn literal_fragments(
    source: &str,
    budget: usize,
    mut images: Vec<(String, String)>,
    output: &mut Vec<Fragment>,
) {
    for (index, text) in session::markdown::split_text(source, (budget / 2).max(2))
        .into_iter()
        .enumerate()
    {
        output.push(Fragment {
            text: literal(&text),
            images: std::mem::take(&mut images),
            separator: if index == 0 { "\n\n" } else { "" },
        });
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
    let mut parts = Vec::new();
    let mut current = String::new();
    for fragment in fragments(source, format, header, 1800)? {
        let separator = if current.is_empty() {
            ""
        } else {
            fragment.separator
        };
        if !current.is_empty()
            && !session::markdown::fits(
                &format!("{header}{current}{separator}{}", fragment.text),
                1800,
            )
        {
            parts.push(format!("{header}{}", std::mem::take(&mut current)));
        }
        if !current.is_empty() {
            current.push_str(fragment.separator);
        }
        current.push_str(&fragment.text);
    }
    if !current.is_empty() {
        parts.push(format!("{header}{current}"));
    }
    Ok(parts)
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
        let mut sections: Vec<session::presentation::Section> = Vec::new();
        let mut previous = None;
        for item in items {
            let key = (item.turn_id.clone(), item.phase.clone());
            let section = session::presentation::Section::item(item, locale);
            if item.role == session::Role::Agent && previous.as_ref() == Some(&key) {
                let current = sections.last_mut().expect("previous agent section");
                current.body.push_str("\n\n");
                current.body.push_str(&section.body);
            } else {
                sections.push(section);
            }
            previous = (item.role == session::Role::Agent).then_some(key);
        }
        Self::sections(sections)
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
