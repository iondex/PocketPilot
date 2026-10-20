//! Native Discord navigation payloads. Catalog rules remain in integrations::navigation.
use serde::{Deserialize, Serialize};
use twilight_model::channel::message::{
    Component,
    component::{ButtonStyle, SelectMenuType},
};
use twilight_util::builder::message::{
    ActionRowBuilder, ButtonBuilder, SelectMenuBuilder, SelectMenuOptionBuilder,
};

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
    pub components: Vec<Component>,
}

impl Screen {
    pub fn new(text: String) -> Self {
        Self {
            text,
            components: Vec::new(),
        }
    }

    pub fn select(
        &mut self,
        token: &str,
        placeholder: &str,
        options: Vec<SelectMenuOptionBuilder>,
    ) {
        if !options.is_empty() {
            let mut menu = SelectMenuBuilder::new(format!("pp:{token}"), SelectMenuType::Text)
                .placeholder(placeholder)
                .min_values(1)
                .max_values(1);
            for option in options {
                menu = menu.option(option);
            }
            self.components.push(
                ActionRowBuilder::new()
                    .component(menu.build())
                    .build()
                    .into(),
            );
        }
    }

    pub fn buttons(&mut self, buttons: Vec<ButtonBuilder>) {
        if !buttons.is_empty() {
            let mut row = ActionRowBuilder::new();
            for button in buttons {
                row = row.component(button.build());
            }
            self.components.push(row.build().into());
        }
    }

    pub fn message(self) -> crate::discord::native::Message {
        crate::discord::native::Message::menu(self.text, self.components)
    }
}

pub fn button(token: &str, label: &str) -> ButtonBuilder {
    ButtonBuilder::new(ButtonStyle::Secondary)
        .custom_id(format!("pp:{token}"))
        .label(label)
}

pub fn link(url: String, label: &str) -> ButtonBuilder {
    ButtonBuilder::new(ButtonStyle::Link).url(url).label(label)
}

pub fn option(label: &str, detail: &str, index: usize) -> SelectMenuOptionBuilder {
    SelectMenuOptionBuilder::new(short(label, 90), index.to_string()).description(short(detail, 95))
}

/// Bound both UTF-16 length and line count; never split an emoji grapheme.
pub fn short(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.is_empty() { "—" } else { &text };
    session::markdown::preview(text, limit)
}

pub fn quoted(text: &str) -> String {
    crate::discord::render::literal(&short(text, 120))
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
