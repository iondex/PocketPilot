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
    let document = !session::markdown::fits(&text, 2800);
    Content {
        blocks: if document || text.is_empty() {
            vec![]
        } else {
            vec![plain(&text)]
        },
        text,
        format: session::markdown::Format::Plain,
        document,
        edit: false,
    }
}

pub fn markdown(text: String) -> Content {
    let mut content = message(text);
    content.format = session::markdown::Format::Markdown;
    content.document = !session::markdown::fits(&content.text, 2800);
    if !content.document {
        content.blocks = session::markdown::Document::parse(&content.text)
            .ok()
            .and_then(|doc| crate::markdown::blocks(&doc))
            .unwrap_or_else(|| {
                content.document = true;
                Vec::new()
            });
    } else {
        content.blocks.clear();
    }
    content
}
