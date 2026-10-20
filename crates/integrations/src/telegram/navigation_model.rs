//! Persisted menu content and native Telegram presentation.
use super::Action;
use crate::telegram::ledger::Scope;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Choice {
    pub(super) label: UiText,
    pub(super) action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Menu {
    pub(super) heading: UiText,
    pub(super) note: UiText,
    pub(super) choices: Vec<Choice>,
    pub(super) back: Action,
    pub(super) refresh: Action,
    pub(super) scope: Option<Scope>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum UiText {
    Message { id: String },
    Literal(String),
}

impl UiText {
    pub(super) fn id(id: &str) -> Self {
        Self::Message { id: id.into() }
    }

    pub(super) fn render(&self, locale: &str) -> String {
        match self {
            Self::Message { id } => ::i18n::text(locale, id),
            Self::Literal(text) => text.clone(),
        }
    }
}

impl From<String> for UiText {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

impl From<&str> for UiText {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}

fn md(text: &str) -> String {
    crate::telegram::api::escape_markdown(text)
}

impl Menu {
    pub(super) fn render(&self, locale: &str) -> String {
        let mut text = format!("*{}*\n", md(&self.heading.render(locale)));
        let note = self.note.render(locale);
        if !note.is_empty() {
            text.push_str(&format!("\n{}\n", md(&note)));
        }
        text
    }

    pub(super) fn buttons(&self, id: &str, locale: &str) -> Vec<Vec<crate::telegram::api::Button>> {
        let compact = matches!(
            self.refresh,
            Action::Home
                | Action::Host { .. }
                | Action::HostPage { .. }
                | Action::NewSession
                | Action::NewHost { .. }
                | Action::NewHostPage { .. }
        );
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut utility_row = false;
        for (index, choice) in self.choices.iter().enumerate() {
            let utility = matches!(
                choice.action,
                Action::NewSession | Action::Topics { after: 0 } | Action::ConfirmClearTopics
            );
            let paging = matches!(
                choice.action,
                Action::HostPage { .. }
                    | Action::NewHostPage { .. }
                    | Action::Workspace {
                        cursor: Some(_),
                        ..
                    }
            );
            if (paging || utility != utility_row) && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            utility_row = utility;
            row.push(crate::telegram::api::Button {
                label: session::delivery::short_title(
                    &choice.label.render(locale),
                    if compact { 24 } else { 40 },
                ),
                data: format!("nav:{id}:{}", index + 1),
            });
            if !compact || paging || row.len() == 2 {
                rows.push(std::mem::take(&mut row));
            }
        }
        if !row.is_empty() {
            rows.push(row);
        }
        rows.push(
            [("nav.back", "b"), ("nav.home", "h"), ("nav.refresh", "r")]
                .into_iter()
                .map(|(key, action)| crate::telegram::api::Button {
                    label: ::i18n::text(locale, key),
                    data: format!("nav:{id}:{action}"),
                })
                .collect(),
        );
        rows
    }
}
