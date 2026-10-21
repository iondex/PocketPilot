//! Freeze entry previews and live deltas together with their full observation baseline.
use super::*;
use crate::slack::render;
use session::{History, Role};
use std::collections::BTreeMap;

pub(super) struct Observation {
    pub content: Content,
    pub cursor: String,
}

/// Reading full history establishes a cursor; it never means displaying full history.
/// Opening shows the last interaction, while subsequent polls show visible changes only.
pub(super) fn capture(
    binding: &Binding,
    history: &History,
    namespace: &str,
    locale: &str,
) -> Result<Observation> {
    if !history.all_pages_read {
        return Err(Error::Ui("slack.error.history"));
    }
    let cursor: BTreeMap<_, _> = history
        .items
        .iter()
        .map(|item| (item_key(item), engine::observation::visible_hash(item)))
        .collect();
    let sections = if binding.history_ready {
        let previous: BTreeMap<String, String> = serde_json::from_str(&binding.observed)?;
        let own_prefix = format!("pp:slack:{namespace}:");
        history
            .items
            .iter()
            .filter(|item| !item.is_transient_error())
            .filter(|item| !engine::observation::unchanged(item, previous.get(&item_key(item))))
            .filter(|item| {
                !(item.role == Role::User
                    && item
                        .client_id
                        .as_ref()
                        .is_some_and(|id| id.starts_with(&own_prefix)))
            })
            .map(|item| session::presentation::Section::item(item, locale))
            .collect()
    } else {
        session::presentation::latest(history, locale)
    };
    Ok(Observation {
        content: render::conversation(sections),
        cursor: serde_json::to_string(&cursor)?,
    })
}

fn item_key(item: &session::TextItem) -> String {
    format!("{}:{}:{}", item.turn_id.len(), item.turn_id, item.item_id)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
