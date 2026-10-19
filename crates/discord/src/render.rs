//! Render Discord conversation roles and parsed CommonMark into bounded message content.
pub use session::markdown::literal;
use session::markdown::{Document, Format, TooLarge};

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
    pub parts: Vec<String>,
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
            let parts = with_header(&section.body, Format::Markdown, &header)?;
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
