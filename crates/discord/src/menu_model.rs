//! Pair visible choices with navigation targets, then bind opaque, scoped action handles once.
use super::super::{Result, error};
use crate::{
    Input,
    storage::Store,
    ui::{self, Action, Screen, View},
};
use serenity::all::{CreateButton, CreateSelectMenuOption, ReactionType};

pub(super) struct Menu {
    pub text: String,
    placeholder: &'static str,
    choices: Vec<(View, CreateSelectMenuOption)>,
    buttons: Vec<Button>,
}

enum Button {
    Navigate { label: &'static str, view: View },
    Link { label: &'static str, url: String },
}

impl Menu {
    pub fn new(text: String, placeholder: &'static str) -> Self {
        Self {
            text,
            placeholder,
            choices: Vec::new(),
            buttons: Vec::new(),
        }
    }

    pub fn choice(&mut self, label: &str, detail: &str, view: View, icon: Option<&str>) {
        let mut option = ui::option(label, detail, self.choices.len());
        if let Some(icon) = icon {
            option = option.emoji(ReactionType::Unicode(icon.into()));
        }
        self.choices.push((view, option));
    }

    pub fn empty_hint(&mut self, locale: &str, id: &str) {
        if self.choices.is_empty() {
            self.text
                .push_str(&format!("\n\n{}", i18n::text(locale, id)));
        }
    }

    pub fn button(&mut self, label: &'static str, view: View) {
        self.buttons.push(Button::Navigate { label, view });
    }

    pub fn link(&mut self, label: &'static str, url: String) {
        self.buttons.push(Button::Link { label, url });
    }

    pub async fn bind(
        self,
        store: &Store,
        input: &Input,
        hash: &str,
        locale: &str,
    ) -> Result<Screen> {
        let mut screen = Screen::new(self.text);
        if !self.choices.is_empty() {
            let (views, options) = self.choices.into_iter().unzip();
            let token = store
                .ui_action(input, hash, Action::Select(views))
                .await
                .map_err(error)?;
            screen.select(&token, &i18n::text(locale, self.placeholder), options);
        }
        let mut buttons = Vec::new();
        for button in self.buttons {
            buttons.push(match button {
                Button::Navigate { label, view } => {
                    let token = store
                        .ui_action(input, hash, Action::Navigate(view))
                        .await
                        .map_err(error)?;
                    ui::button(&token, &i18n::text(locale, label))
                }
                Button::Link { label, url } => {
                    CreateButton::new_link(url).label(i18n::text(locale, label))
                }
            });
        }
        screen.buttons(buttons);
        Ok(screen)
    }
}
