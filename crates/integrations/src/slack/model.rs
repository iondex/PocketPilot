use serde::{Deserialize, Serialize};
use slack_morphism::prelude::SlackBlock;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Menu,
    Workspaces {
        offset: usize,
    },
    Projects {
        #[serde(default)]
        host: String,
        offset: usize,
    },
    UnbindConfirm,
    Sessions {
        workspace: String,
        cursor: Option<String>,
    },
    Open {
        workspace: String,
        thread: String,
    },
    New {
        workspace: String,
        title: String,
    },
    Input {
        text: String,
    },
    Status,
    History {
        export: bool,
    },
    Unbind,
    Confirm {
        inbox: String,
    },
}

impl Command {
    pub fn writes(&self) -> bool {
        matches!(
            self,
            Self::New { .. } | Self::Input { .. } | Self::Unbind | Self::Confirm { .. }
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub uploads: Vec<session::files::Incoming>,
    pub id: String,
    pub channel: String,
    pub user: String,
    pub root: Option<String>,
    #[serde(default)]
    pub menu_message: Option<String>,
    pub binding: Option<String>,
    pub revision: Option<i64>,
    pub date: i64,
    pub command: Command,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Content {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<session::presentation::Section>,
    #[serde(default)]
    pub format: session::markdown::Format,
    #[serde(default)]
    pub blocks: Vec<SlackBlock>,
    /// Legacy file deliveries are rejected, never uploaded.
    #[serde(default, skip_serializing)]
    pub document: bool,
    #[serde(default)]
    pub edit: bool,
}

impl Content {
    pub fn text(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Run(Command),
    NewModal { workspace: String },
    NewSubmit { workspace: String },
}

#[derive(Clone, Copy, Default)]
pub struct InteractionSource<'a> {
    pub id: Option<&'a str>,
    pub message: Option<&'a str>,
}
