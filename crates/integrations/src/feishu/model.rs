//! Durable commands contain server-issued handles, never trusted callback paths.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: String,
    pub chat: String,
    pub message: String,
    pub root: Option<String>,
    pub thread: Option<String>,
    pub date: i64,
    pub command: Command,
    #[serde(default)]
    pub files: Vec<session::files::Incoming>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Command {
    Unsupported,
    Menu(Action),
    Text(String),
    Create { project: String, title: String },
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Action {
    Home {
        offset: usize,
    },
    Projects {
        host: String,
        offset: usize,
    },
    Sessions {
        project: String,
        cursor: Option<String>,
    },
    New {
        project: String,
    },
    Create {
        project: String,
    },
    Groups {
        project: String,
        session: String,
    },
    Bind {
        project: String,
        session: String,
        group: String,
    },
    Bindings {
        offset: usize,
    },
    Binding {
        id: String,
    },
    Change {
        id: String,
        version: crate::bindings::Version,
        change: crate::bindings::Change,
        confirmed: bool,
    },
}

#[derive(Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Binding {
    pub id: String,
    pub chat: String,
    pub root: String,
    pub topic: String,
    pub project: String,
    pub instance: String,
    pub session: String,
    pub cwd: String,
    pub endpoint: String,
    pub revision: i64,
    pub active: bool,
    pub cursor: String,
    pub ready: bool,
    pub status: String,
    pub state: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Destination {
    pub chat: String,
    pub root: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Resource {
    pub message: String,
    pub key: String,
    pub image: bool,
}
