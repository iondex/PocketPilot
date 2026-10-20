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
            Self::Backend => ::i18n::Message::new("error.discord.backend"),
            Self::TooLarge => ::i18n::Message::new("error.discord.too_large"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
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
            text: field("text")?,
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
mod tests {
    use super::*;

    #[test]
    fn guild_owner_is_required_and_stop_has_no_route() {
        assert!(authorized(1, Some(1), 2, 2));
        assert!(!authorized(1, None, 2, 2));
        assert!(!authorized(1, Some(3), 2, 2));
        assert!(!authorized(1, Some(1), 2, 3));
        assert!(parse("pp-stop", &BTreeMap::new(), true).is_err());
        assert!(parse("pp-ask", &BTreeMap::new(), true).is_err());
    }

    #[test]
    fn channel_names_are_short_and_cannot_ping_members() {
        let name = channel_name("Hello @everyone / Test");
        assert!(!name.contains('@'));
        assert_eq!(name, "hello-everyone-test");
        assert!(channel_name(&"好".repeat(500)).chars().count() < 100);
    }

    #[test]
    fn navigation_commands_fill_missing_selections_through_the_menu() {
        assert!(matches!(
            parse("pp", &BTreeMap::new(), false).unwrap(),
            Command::Workspaces
        ));
        assert!(matches!(
            parse("pp-open", &BTreeMap::new(), false).unwrap(),
            Command::Workspaces
        ));
        let mut fields = BTreeMap::from([("workspace".into(), "host-a".into())]);
        assert!(
            matches!(parse("pp-projects", &fields, false).unwrap(), Command::Projects { workspace } if workspace == "host-a")
        );
        assert!(matches!(
            parse("pp-sessions", &fields, false).unwrap(),
            Command::Projects { .. }
        ));
        fields.insert("project".into(), "project-a".into());
        assert!(
            matches!(parse("pp-sessions", &fields, false).unwrap(), Command::Sessions { workspace, project, .. } if workspace == "host-a" && project == "project-a")
        );
        fields.remove("workspace");
        assert!(matches!(
            parse("pp-sessions", &fields, false).unwrap(),
            Command::Workspaces
        ));
    }
}
