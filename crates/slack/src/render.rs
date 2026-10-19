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
    use session::RuntimeStatus;
    let id = match status {
        RuntimeStatus::NotLoaded => "state.not_loaded",
        RuntimeStatus::Idle => "state.idle",
        RuntimeStatus::Active {
            waiting_on_approval: true,
            ..
        } => "state.waiting_approval",
        RuntimeStatus::Active {
            waiting_on_user_input: true,
            ..
        } => "state.waiting_input",
        RuntimeStatus::Active { .. } => "state.active",
        RuntimeStatus::SystemError => "state.error",
        RuntimeStatus::Unknown => "state.unknown",
    };
    i18n::text(locale, id)
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
    let document = text.chars().count() > 2800;
    Content {
        blocks: if document || text.is_empty() {
            vec![]
        } else {
            vec![plain(&text)]
        },
        text,
        document,
        edit: false,
    }
}
