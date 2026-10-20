//! Compact Telegram presentation, independent of provider state descriptions.
use serde::{Deserialize, Serialize};
use session::RuntimeStatus;

pub const MANAGEMENT_TITLE: &str = "tg.management.title";

pub const MANAGEMENT_ICON: &str = "📁";

pub fn emoji(status: &RuntimeStatus) -> &'static str {
    match status {
        RuntimeStatus::NotLoaded | RuntimeStatus::Idle => "💬",
        RuntimeStatus::Active {
            waiting_on_approval: true,
            ..
        } => "👀",
        RuntimeStatus::Active {
            waiting_on_user_input: true,
            ..
        } => "❓",
        RuntimeStatus::Active { .. } => "⚡️",
        RuntimeStatus::SystemError => "❗️",
        RuntimeStatus::Unknown => "⁉️",
    }
}

pub fn typing(status: &RuntimeStatus) -> bool {
    matches!(
        status,
        RuntimeStatus::Active {
            waiting_on_approval: false,
            waiting_on_user_input: false
        }
    )
}

pub fn title(original: &str) -> String {
    let original = original.trim();
    session::delivery::short_title(
        if original.is_empty() {
            "新会话"
        } else {
            original
        },
        40,
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Appearance {
    pub title: String,
    pub emoji: String,
}

impl Appearance {
    pub(crate) fn receipt(&self) -> String {
        format!("icon:v1:{}", self.emoji)
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
