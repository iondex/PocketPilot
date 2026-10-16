//! Explicit /history shows the latest user input and confirmed final answers only.
use super::Service;
use crate::{api::Input, i18n};
use session::History;

impl Service {
    pub(super) fn error_history(&self, input: &Input, history: &History) -> String {
        let mut errors = history
            .items
            .iter()
            .rev()
            .filter(|i| i.is_error())
            .take(10)
            .collect::<Vec<_>>();
        if errors.is_empty() {
            return i18n::tr(input, "tg.history.errors.empty");
        }
        errors.reverse();
        let mut text = format!("**{}**", i18n::tr(input, "tg.history.errors.heading"));
        for error in errors {
            let turn = crate::api::escape_markdown(&error.turn_id);
            text.push_str(&format!(
                "\n\n**{turn}**\n{}",
                super::messages::error_text(error, crate::i18n::lang(input))
            ));
        }
        if !history.all_pages_read {
            text.push_str(&format!("\n\n{}", i18n::tr(input, "tg.history.incomplete")));
        }
        text
    }

    pub(super) fn history_summary(&self, input: &Input, history: &History) -> String {
        if !history.all_pages_read {
            return i18n::tr(input, "tg.history.incomplete");
        }
        let Some(interaction) = session::delivery::last_interaction(history) else {
            return history
                .items
                .iter()
                .rev()
                .find(|i| i.is_error())
                .map(|error| super::messages::error_text(error, crate::i18n::lang(input)))
                .unwrap_or_else(|| i18n::tr(input, "tg.history.no.input"));
        };
        let answer = if interaction.answers.is_empty() {
            i18n::tr(input, "tg.history.no.final")
        } else {
            interaction
                .answers
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let mut text = format!(
            "**{}**\n\n{}\n\n**{}**\n\n{}",
            i18n::tr(input, "tg.history.user"),
            super::messages::user_text(interaction.user),
            i18n::tr(input, "tg.history.final"),
            if interaction.answers.is_empty() {
                answer
            } else {
                format!("✅ {answer}")
            }
        );
        for error in interaction.errors {
            text.push_str(&format!(
                "\n\n{}",
                super::messages::error_text(error, crate::i18n::lang(input))
            ));
        }
        if history
            .items
            .iter()
            .any(|item| item.is_error() && item.turn_id != interaction.user.turn_id)
        {
            text.push_str(&format!(
                "\n\n{}",
                i18n::tr(input, "tg.history.errors.earlier")
            ));
        }
        text
    }
}
