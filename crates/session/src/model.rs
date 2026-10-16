use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionRef {
    pub instance: String,
    pub thread: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeStatus {
    NotLoaded,
    Idle,
    Active {
        waiting_on_approval: bool,
        waiting_on_user_input: bool,
    },
    SystemError,
    #[default]
    Unknown,
}

impl RuntimeStatus {
    pub fn label(&self) -> String {
        self.label_in("zh-CN")
    }

    pub fn label_in(&self, locale: &str) -> String {
        let (emoji, id) = match self {
            Self::NotLoaded => ("⚪", "state.not_loaded"),
            Self::Idle => ("🟢", "state.idle"),
            Self::Active {
                waiting_on_approval: true,
                ..
            } => ("🟠", "state.waiting_approval"),
            Self::Active {
                waiting_on_user_input: true,
                ..
            } => ("🟡", "state.waiting_input"),
            Self::Active { .. } => ("🔵", "state.active"),
            Self::SystemError => ("🔴", "state.error"),
            Self::Unknown => ("⚫", "state.unknown"),
        };
        format!("{emoji} {}", i18n::text(locale, id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub reference: SessionRef,
    pub title: String,
    #[serde(default)]
    pub native_title: Option<String>,
    pub cwd: String,
    pub status: RuntimeStatus,
    /// Metadata may omit this ID for an active session. Use provider.active_turn before steer.
    pub active_turn: Option<String>,
    pub updated_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionQuery {
    pub cwd: Option<String>,
    pub cursor: Option<String>,
    pub limit: u32,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextItem {
    pub turn_id: String,
    pub item_id: String,
    pub role: Role,
    pub phase: Option<String>,
    pub text: String,
    pub complete: bool,
    pub client_id: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

impl TextItem {
    pub fn is_error(&self) -> bool {
        matches!(
            self.phase.as_deref(),
            Some("error" | "error_retry" | "turn_failed" | "turn_interrupted")
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct History {
    pub items: Vec<TextItem>,
    /// True only when pagination completed, not a guarantee of an atomic snapshot.
    pub all_pages_read: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Capabilities {
    pub list: bool,
    pub history: bool,
    pub live_status: bool,
    pub control_verified: bool,
}
