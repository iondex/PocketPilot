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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_have_no_workspace_or_identity_prefix_and_waiting_is_not_typing() {
        assert_eq!(title("原始标题"), "原始标题");
        assert_eq!(
            emoji(&RuntimeStatus::NotLoaded),
            emoji(&RuntimeStatus::Idle)
        );
        assert!(title(&"长".repeat(100)).chars().count() <= 40);
        assert!(!typing(&RuntimeStatus::Active {
            waiting_on_approval: true,
            waiting_on_user_input: false
        }));
        assert!(typing(&RuntimeStatus::Active {
            waiting_on_approval: false,
            waiting_on_user_input: false
        }));
    }
}
