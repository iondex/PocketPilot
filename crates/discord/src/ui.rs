//! Native Discord navigation payloads. Catalog rules remain in engine::navigation.
use serde::{Deserialize, Serialize};
use serenity::all::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum View {
    Workspaces {
        offset: usize,
    },
    Projects {
        workspace: String,
        offset: usize,
    },
    Sessions {
        project: String,
        cursors: Vec<Option<String>>,
    },
    Open {
        project: String,
        thread: String,
        cursors: Vec<Option<String>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Navigate(View),
    Select(Vec<View>),
}

/// Only opaque action handles cross the platform boundary; IDs/cursors stay in SQLite.
pub struct Screen {
    pub text: String,
    pub components: Vec<CreateActionRow>,
}

impl Screen {
    pub fn new(text: String) -> Self {
        Self {
            text,
            components: Vec::new(),
        }
    }

    pub fn select(&mut self, token: &str, placeholder: &str, options: Vec<CreateSelectMenuOption>) {
        if !options.is_empty() {
            self.components.push(CreateActionRow::SelectMenu(
                CreateSelectMenu::new(
                    format!("pp:{token}"),
                    CreateSelectMenuKind::String { options },
                )
                .placeholder(placeholder)
                .min_values(1)
                .max_values(1),
            ));
        }
    }

    pub fn buttons(&mut self, buttons: Vec<CreateButton>) {
        if !buttons.is_empty() {
            self.components.push(CreateActionRow::Buttons(buttons));
        }
    }
}

pub fn button(token: &str, label: &str) -> CreateButton {
    CreateButton::new(format!("pp:{token}"))
        .label(label)
        .style(ButtonStyle::Secondary)
}

pub fn option(label: &str, detail: &str, index: usize) -> CreateSelectMenuOption {
    CreateSelectMenuOption::new(short(label, 90), index.to_string()).description(short(detail, 95))
}

/// Bound both UTF-16 length and line count; never split an emoji grapheme.
pub fn short(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.is_empty() { "—" } else { &text };
    session::markdown::preview(text, limit)
}

pub fn quoted(text: &str) -> String {
    crate::render::literal(&short(text, 120))
}

pub use session::presentation::{status, status_icon};

pub fn updated(time: Option<i64>, locale: &str) -> String {
    let Some(time) = time else {
        return i18n::text(locale, "discord.ui.updated_unknown");
    };
    let seconds = engine::now().saturating_sub(time).max(0);
    let (id, count) = if seconds < 60 {
        return i18n::text(locale, "discord.ui.updated_now");
    } else if seconds < 3600 {
        ("discord.ui.updated_minutes", seconds / 60)
    } else if seconds < 86400 {
        ("discord.ui.updated_hours", seconds / 3600)
    } else {
        ("discord.ui.updated_days", seconds / 86400)
    };
    i18n::Message::new(id).arg("count", count).render(locale)
}
