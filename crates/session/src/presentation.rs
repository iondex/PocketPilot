//! Shared conversation headings and phase cues; platforms render the Markdown body.
use crate::{History, Role, TextItem};
use serde::{Deserialize, Serialize};

pub fn role_heading(role: Role, locale: &str) -> String {
    let (icon, id) = match role {
        Role::User => ("👤", "common.role.user"),
        Role::Agent => ("🤖", "common.role.agent"),
    };
    format!("{icon} **{}**", i18n::text(locale, id))
}

pub fn phase_icon(phase: Option<&str>) -> &'static str {
    match phase {
        Some("final_answer") => "✅",
        Some("question") => "❓",
        Some("commentary" | "partial_answer" | "plan") => "💭",
        Some("error" | "turn_failed" | "turn_interrupted") => "❌",
        _ => "💬",
    }
}

pub fn message_heading(role: Role, phase: Option<&str>, locale: &str) -> String {
    let heading = role_heading(role, locale);
    if role == Role::User {
        heading
    } else {
        format!("{heading} · {}", phase_icon(phase))
    }
}

/// Generated heading stays separate so each platform can repeat it on continuation messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Section {
    pub heading: String,
    pub body: String,
}

impl Section {
    pub fn item(item: &TextItem, locale: &str) -> Self {
        let body = crate::content::display(item);
        Self {
            heading: message_heading(item.role, item.phase.as_deref(), locale),
            body,
        }
    }

    pub fn markdown(&self) -> String {
        format!("{}\n\n{}", self.heading, self.body)
    }
}

pub fn latest(history: &History, locale: &str) -> Vec<Section> {
    let mut sections = latest_interaction(history, locale);
    if history.all_pages_read {
        sections.extend(
            history
                .items
                .iter()
                .filter(|i| i.phase.as_deref() == Some("question"))
                .map(|i| Section::item(i, locale)),
        );
    }
    sections
}

fn latest_interaction(history: &History, locale: &str) -> Vec<Section> {
    let notice = |id| {
        vec![Section {
            heading: "ℹ️".into(),
            body: i18n::text(locale, id),
        }]
    };
    if !history.all_pages_read {
        return notice("common.history.incomplete");
    }
    let Some(interaction) = crate::delivery::last_interaction(history) else {
        return notice("common.history.no_input");
    };
    let answer = if interaction.answers.is_empty() {
        format!("ℹ️ {}", i18n::text(locale, "common.history.no_final"))
    } else {
        interaction
            .answers
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    vec![
        Section::item(interaction.user, locale),
        Section {
            heading: message_heading(
                Role::Agent,
                (!interaction.answers.is_empty()).then_some("final_answer"),
                locale,
            ),
            body: answer,
        },
    ]
}

pub fn status(status: &crate::RuntimeStatus, locale: &str) -> String {
    format!("{} {}", status_icon(status), status.text_in(locale))
}

pub fn status_icon(status: &crate::RuntimeStatus) -> &'static str {
    use crate::RuntimeStatus::*;
    match status {
        Idle => "🟢",
        Active {
            waiting_on_approval: true,
            ..
        }
        | Active {
            waiting_on_user_input: true,
            ..
        } => "🟠",
        Active { .. } => "🔵",
        NotLoaded => "💤",
        SystemError => "🔴",
        Unknown => "⚪",
    }
}
