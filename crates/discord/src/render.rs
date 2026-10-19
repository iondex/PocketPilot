//! Render parsed CommonMark into Discord's supported subset; keep exports unchanged.
pub use session::markdown::literal;
use session::markdown::{Document, Format, TooLarge};

pub fn messages(source: &str, format: Format) -> Result<Vec<String>, TooLarge> {
    if source.len() > session::markdown::MAX_BYTES {
        return Err(TooLarge);
    }
    if format == Format::Plain {
        return Ok(session::markdown::split_text(source, 800)
            .into_iter()
            .map(|s| literal(&s))
            .collect());
    }
    let mut result = Vec::new();
    for part in Document::parse(source)?.chunks(700, 64) {
        let text = part.markdown(session::markdown::Dialect::Discord);
        match text {
            Some(text) if session::markdown::fits(&text, 1800) => result.push(text),
            _ => result.extend(
                session::markdown::split_text(&part.plain(), 800)
                    .into_iter()
                    .map(|s| literal(&s)),
            ),
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
