//! Guild-owner-only slash-command prototype. No Message Content intent is needed.
#[path = "config.rs"]
pub mod config;

#[derive(serde::Serialize)]
pub struct Identity {
    pub bot_id: String,
    pub application_id: String,
    pub guild_id: String,
    pub owner_id: String,
    pub install_url: String,
}

#[path = "runtime.rs"]
pub mod runtime;

#[path = "storage.rs"]
pub mod storage;

pub use runtime::Discord;

#[path = "api.rs"]
pub mod api;

#[path = "render.rs"]
mod render;

pub mod native;

#[path = "ui.rs"]
pub mod ui;

#[path = "gateway.rs"]
pub mod gateway;

pub use api::Api;

use async_trait::async_trait;

pub use gateway::run;

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Discord request failed; credentials and response are redacted")]
    Request,
    #[error("Discord command is invalid or not authorized")]
    Denied,
    #[error("Discord ask requires text or an attachment")]
    MissingInput,
    #[error("Discord prototype backend rejected the operation; refresh its status")]
    Backend,
    #[error("message exceeds the processing limit")]
    TooLarge,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Request => ::i18n::Message::new("error.discord.request"),
            Self::Denied => ::i18n::Message::new("error.discord.denied"),
            Self::MissingInput => ::i18n::Message::new("error.discord.missing_input"),
            Self::Backend => ::i18n::Message::new("error.discord.backend"),
            Self::TooLarge => ::i18n::Message::new("error.discord.too_large"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Bindings,
    Workspaces,
    Projects {
        workspace: String,
    },
    Sessions {
        workspace: String,
        project: String,
        cursor: Option<String>,
    },
    Open {
        workspace: String,
        project: String,
        thread: String,
    },
    New {
        workspace: String,
        project: String,
        text: String,
        confirmed: bool,
    },
    Ask {
        text: String,
    },
    Status,
    Recent,
    Rename {
        title: String,
    },
    Unbind,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub uploads: Vec<session::files::Incoming>,
    pub locale: Option<String>,
    pub id: u64,
    pub bot: u64,
    pub guild: u64,
    pub channel: u64,
    pub user: u64,
    pub date: i64,
    pub command: Command,
}

#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Called after a guild-owner check and ephemeral interaction ACK.
    async fn handle(&self, api: &Api, input: Input) -> Result<String, i18n::Message>;

    /// Only local catalog/authorization work is permitted before a modal ACK.
    async fn new_session_form(
        &self,
        _api: &Api,
        _input: Input,
        _token: &str,
    ) -> Result<ui::NewSessionForm, i18n::Message> {
        Err("discord.ui.expired".into())
    }

    async fn menu(
        &self,
        _api: &Api,
        _input: Input,
        _token: Option<String>,
        _value: Option<String>,
    ) -> Result<ui::Screen, i18n::Message> {
        Err("discord.ui.expired".into())
    }
}

/// Restrict routing independently of command registration/Discord permissions.
pub fn authorized(expected_guild: u64, actual_guild: Option<u64>, owner: u64, user: u64) -> bool {
    actual_guild == Some(expected_guild) && owner != 0 && owner == user
}

pub fn channel_name(title: &str) -> String {
    let title = session::delivery::short_title(title, 65)
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let title = title
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let title = title.as_str();
    let title = if title.is_empty() { "session" } else { title };
    title.into()
}

pub(crate) fn parse(
    name: &str,
    text: &BTreeMap<String, String>,
    confirmed: bool,
) -> Result<Command, Error> {
    let field = |name: &str| {
        text.get(name)
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .ok_or(Error::Denied)
    };
    Ok(match name {
        "pp" | "pp-workspaces" => Command::Workspaces,
        "pp-bindings" => Command::Bindings,
        "pp-projects" | "pp-sessions" | "pp-open" if !text.contains_key("workspace") => {
            Command::Workspaces
        }
        "pp-projects" | "pp-sessions" if !text.contains_key("project") => Command::Projects {
            workspace: field("workspace")?,
        },
        "pp-projects" => Command::Projects {
            workspace: field("workspace")?,
        },
        "pp-sessions" => Command::Sessions {
            workspace: field("workspace")?,
            project: field("project")?,
            cursor: text.get("cursor").cloned(),
        },
        "pp-open" => Command::Open {
            workspace: field("workspace")?,
            project: field("project")?,
            thread: field("thread")?,
        },
        "pp-new" => Command::New {
            workspace: field("workspace")?,
            project: field("project")?,
            text: field("text")?,
            confirmed,
        },
        "pp-ask" => Command::Ask {
            text: text
                .get("text")
                .filter(|text| !text.trim().is_empty())
                .cloned()
                .ok_or(Error::MissingInput)?,
        },
        "pp-status" => Command::Status,
        "pp-recent" => Command::Recent,
        "pp-rename" => Command::Rename {
            title: field("title")?,
        },
        "pp-unbind" => Command::Unbind,
        _ => return Err(Error::Denied),
    })
}

#[cfg(test)]
#[path = "discord_tests.rs"]
mod tests;
