//! Guild-owner-only slash-command prototype. No Message Content intent is needed.
pub mod config;

#[cfg(feature = "runtime")]
pub mod runtime;

#[cfg(feature = "runtime")]
pub mod storage;

#[cfg(feature = "runtime")]
pub use runtime::Discord;

#[cfg(feature = "runtime")]
pub mod api;

#[cfg(feature = "runtime")]
pub mod gateway;

#[cfg(feature = "runtime")]
pub use api::Api;

#[cfg(feature = "runtime")]
use async_trait::async_trait;

#[cfg(feature = "runtime")]
pub use gateway::run;

#[cfg(any(feature = "runtime", test))]
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Discord request failed; credentials and response are redacted")]
    Request,
    #[error("Discord command is invalid or not authorized")]
    Denied,
    #[error("Discord prototype backend rejected the operation; refresh its status")]
    Backend,
    #[error("history exceeds the prototype's single attachment limit")]
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
        text: String,
        confirmed: bool,
    },
    Ask {
        text: String,
    },
    Status,
    History,
    Rename {
        title: String,
    },
    Unbind,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub locale: Option<String>,
    pub id: u64,
    pub bot: u64,
    pub guild: u64,
    pub channel: u64,
    pub user: u64,
    pub date: i64,
    pub command: Command,
}

#[cfg(feature = "runtime")]
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Called after a guild-owner check and ephemeral interaction ACK.
    async fn handle(&self, api: &Api, input: Input) -> Result<String, i18n::Message>;
}

/// Restrict routing independently of command registration/Discord permissions.
pub fn authorized(expected_guild: u64, actual_guild: Option<u64>, owner: u64, user: u64) -> bool {
    actual_guild == Some(expected_guild) && owner != 0 && owner == user
}

pub fn channel_name(title: &str, thread: &str) -> String {
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
    let title = title.trim_matches('-');
    let title = if title.is_empty() { "session" } else { title };
    let suffix = thread
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>();
    format!("{title}-{suffix}")
}

#[cfg(any(feature = "runtime", test))]
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
        "pp-workspaces" => Command::Workspaces,
        "pp-sessions" => Command::Sessions {
            workspace: field("workspace")?,
            cursor: text.get("cursor").cloned(),
        },
        "pp-open" => Command::Open {
            workspace: field("workspace")?,
            thread: field("thread")?,
        },
        "pp-new" => Command::New {
            workspace: field("workspace")?,
            text: field("text")?,
            confirmed,
        },
        "pp-ask" => Command::Ask {
            text: field("text")?,
        },
        "pp-status" => Command::Status,
        "pp-history" => Command::History,
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
        let name = channel_name("Hello @everyone / Test", "abcdef123456");
        assert!(!name.contains('@'));
        assert!(name.ends_with("-abcdef12"));
        assert!(channel_name(&"好".repeat(500), "thread").chars().count() < 100);
    }
}
