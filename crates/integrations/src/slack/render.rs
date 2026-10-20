use crate::rendering::{Capabilities, RenderBatch, RenderRequest, Renderer};
use crate::slack::model::Content;

pub struct BlockRenderer;

impl Renderer for BlockRenderer {
    type Part = Content;

    fn capabilities(&self) -> Capabilities {
        Capabilities::for_platform(crate::Platform::Slack)
    }

    fn render(
        &self,
        request: RenderRequest<'_>,
    ) -> Result<RenderBatch<Content>, session::markdown::TooLarge> {
        let content = if request.format == session::markdown::Format::Plain {
            message(request.source.into())
        } else {
            markdown(request.source.into())
        };
        let heading_units = request
            .heading
            .map_or(0, |heading| heading.encode_utf16().count() + 2);
        let limit = self
            .capabilities()
            .text_budget
            .checked_sub(heading_units)
            .filter(|limit| *limit >= 64)
            .ok_or(session::markdown::TooLarge)?;
        let mut parts =
            parts_with_limit(&content, limit).map_err(|_| session::markdown::TooLarge)?;
        if let Some(heading) = request.heading {
            let header = markdown(heading.to_owned());
            let plain = session::markdown::Document::parse(heading)?.plain();
            for part in &mut parts {
                part.text = format!("{plain}\n\n{}", part.text);
                part.blocks.splice(0..0, header.blocks.clone());
            }
        }
        Ok(RenderBatch {
            format: self.capabilities().format,
            parts,
        })
    }
}
use slack_morphism::prelude::*;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn plain(text: &str) -> SlackBlock {
    SlackSectionBlock::new().with_text(pt!(text)).into()
}

pub fn header(text: &str) -> SlackBlock {
    SlackHeaderBlock::new(pt!(session::delivery::short_title(text, 150))).into()
}

pub fn context(text: &str) -> SlackBlock {
    SlackContextBlock::new(vec![SlackContextBlockElement::Plain(
        SlackBlockPlainText::new(session::delivery::short_title(text, 2000)),
    )])
    .into()
}

pub fn section_action(title: &str, detail: &str, label: &str, nonce: &str) -> SlackBlock {
    SlackSectionBlock::new()
        .with_text(pt!(format!(
            "{}\n{}",
            session::delivery::short_title(title, 100),
            detail
        )))
        .with_accessory(
            SlackBlockButtonElement::new(pt!(label))
                .with_action_id(format!("pp:{nonce}").into())
                .with_value(nonce.into())
                .into(),
        )
        .into()
}

pub fn project_select(
    placeholder: &str,
    groups: Vec<SlackBlockOptionGroup<SlackBlockPlainTextOnly>>,
) -> SlackBlock {
    SlackActionsBlock::new(vec![
        SlackBlockStaticSelectElement::new()
            .with_action_id("pp".into())
            .with_placeholder(pt!(placeholder))
            .with_option_groups(groups)
            .into(),
    ])
    .into()
}

/// Group related controls while letting Slack handle wrapping on narrow clients.
pub fn compact_actions(content: &mut Content) {
    let mut blocks = Vec::new();
    for block in std::mem::take(&mut content.blocks) {
        if let SlackBlock::Actions(actions) = block {
            if let Some(SlackBlock::Actions(previous)) = blocks.last_mut()
                && previous.elements.len() + actions.elements.len() <= 5
            {
                previous.elements.extend(actions.elements);
            } else {
                blocks.push(SlackBlock::Actions(actions));
            }
        } else {
            blocks.push(block);
        }
    }
    content.blocks = blocks;
}

pub fn state(status: &session::RuntimeStatus, locale: &str) -> String {
    session::presentation::status(status, locale)
}

pub fn button(label: &str, nonce: &str) -> SlackBlock {
    SlackActionsBlock::new(vec![
        SlackBlockButtonElement::new(pt!(label))
            .with_action_id(format!("pp:{nonce}").into())
            .with_value(nonce.into())
            .into(),
    ])
    .into()
}

pub fn message(text: String) -> Content {
    Content {
        blocks: if !session::markdown::fits(&text, 2800) || text.is_empty() {
            vec![]
        } else {
            vec![plain(&text)]
        },
        text,
        sections: Vec::new(),
        format: session::markdown::Format::Plain,
        document: false,
        edit: false,
    }
}

pub fn markdown(text: String) -> Content {
    let mut content = message(text);
    content.format = session::markdown::Format::Markdown;
    if session::markdown::fits(&content.text, 2800) {
        content.blocks = session::markdown::Document::parse(&content.text)
            .ok()
            .and_then(|doc| crate::slack::markdown::blocks(&doc))
            .unwrap_or_default();
    } else {
        content.blocks.clear();
    }
    content
}

pub fn parts(content: &Content) -> crate::slack::Result<Vec<Content>> {
    if !content.sections.is_empty() {
        if content.text.len() > session::markdown::MAX_BYTES {
            return Err(crate::slack::Error::Ui("slack.error.history"));
        }
        let mut result = Vec::new();
        for section in &content.sections {
            let batch = BlockRenderer
                .render(RenderRequest {
                    source: &section.body,
                    format: session::markdown::Format::Markdown,
                    heading: Some(&section.heading),
                })
                .map_err(|_| crate::slack::Error::Ui("slack.error.history"))?;
            result.extend(batch.parts);
        }
        return Ok(result);
    }
    parts_with_limit(content, 2800)
}

pub fn conversation(sections: Vec<session::presentation::Section>) -> Content {
    Content {
        text: sections
            .iter()
            .map(|s| s.markdown())
            .collect::<Vec<_>>()
            .join("\n\n"),
        sections,
        format: session::markdown::Format::Markdown,
        ..Content::default()
    }
}

fn parts_with_limit(content: &Content, limit: usize) -> crate::slack::Result<Vec<Content>> {
    use session::markdown::{Document, Format, split_text};
    if content.edit || (!content.blocks.is_empty() && session::markdown::fits(&content.text, limit))
    {
        return Ok(vec![content.clone()]);
    }
    if content.format == Format::Plain {
        return Ok(split_text(&content.text, limit)
            .into_iter()
            .map(message)
            .collect());
    }
    let doc = Document::parse(&content.text)
        .map_err(|_| crate::slack::Error::Ui("slack.error.history"))?;
    Ok(doc
        .chunks(limit / 2, 40)
        .into_iter()
        .map(|part| Content {
            text: part.plain(),
            blocks: crate::slack::markdown::blocks(&part).unwrap_or_default(),
            ..Content::default()
        })
        .collect())
}
