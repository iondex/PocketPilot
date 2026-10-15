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
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotLoaded => "历史未加载",
            Self::Idle => "空闲",
            Self::Active {
                waiting_on_approval: true,
                ..
            } => "等待审批",
            Self::Active {
                waiting_on_user_input: true,
                ..
            } => "等待用户回答",
            Self::Active { .. } => "运行中",
            Self::SystemError => "服务错误",
            Self::Unknown => "状态未知",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub reference: SessionRef,
    pub title: String,
    pub cwd: String,
    pub status: RuntimeStatus,
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
