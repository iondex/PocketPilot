use crate::model::Content;
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
    status.text_in(locale)
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
            .and_then(|doc| crate::markdown::blocks(&doc))
            .unwrap_or_default();
    } else {
        content.blocks.clear();
    }
    content
}

pub fn parts(content: &Content) -> crate::Result<Vec<Content>> {
    use session::markdown::{Document, Format, split_text};
    if content.edit || (!content.blocks.is_empty() && session::markdown::fits(&content.text, 2800))
    {
        return Ok(vec![content.clone()]);
    }
    if content.format == Format::Plain {
        return Ok(split_text(&content.text, 2800)
            .into_iter()
            .map(message)
            .collect());
    }
    let doc =
        Document::parse(&content.text).map_err(|_| crate::Error::Ui("slack.error.history"))?;
    Ok(doc
        .chunks(1400, 64)
        .into_iter()
        .map(|part| Content {
            text: part.plain(),
            blocks: crate::markdown::blocks(&part).unwrap_or_default(),
            ..Content::default()
        })
        .collect())
}
