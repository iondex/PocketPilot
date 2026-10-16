//! Explicit /history shows the latest user input and confirmed final answers only.
use super::Service;
use crate::{
    api::{Input, escape_markdown},
    i18n,
};
use session::History;

impl Service {
    pub(super) fn history_summary(&self, input: &Input, history: &History) -> String {
        if !history.all_pages_read {
            return i18n::tr(input, "tg.history.incomplete");
        }
        let Some(interaction) = session::delivery::last_interaction(history) else {
            return i18n::tr(input, "tg.history.no.input");
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
        format!(
            "**{}**\n\n{}\n\n**{}**\n\n{}",
            i18n::tr(input, "tg.history.user"),
            escape_markdown(&interaction.user.text),
            i18n::tr(input, "tg.history.final"),
            answer
        )
    }
}
