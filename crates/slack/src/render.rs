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

pub fn button(label: &str, nonce: &str) -> SlackBlock {
    SlackActionsBlock::new(vec![
        SlackBlockButtonElement::new(pt!(label))
            .with_action_id("pp".into())
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
