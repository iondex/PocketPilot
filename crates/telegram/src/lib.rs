//! Telegram transport. Only the service command starts receiving or sending messages.
pub mod api;

pub mod config;

pub mod credentials;

pub mod ledger;

pub mod pairing;

pub mod runtime;

pub mod storage;
pub use runtime::Telegram;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::time::Duration;
use teloxide::{ApiError, Bot, RequestError, prelude::Requester, requests::Request};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Telegram API request failed (credentials and response are redacted)")]
    Request,
    #[error("Telegram API request timed out")]
    Timeout,
    #[error("Telegram rejected the bot token")]
    Authentication,
    #[error("Telegram network connection failed (URL and credentials are redacted)")]
    Network,
    #[error("Telegram returned an incompatible response (body is redacted)")]
    Protocol,
    #[error("Telegram rate limit; retry in {0} seconds")]
    RetryAfter(u64),
    #[error("Telegram rejected the request")]
    Rejected,
    #[error("Telegram recipient is unavailable")]
    NoRecipient,
    #[error("Telegram message already matches the requested content")]
    NotModified,
    #[error("history exceeds Telegram's single attachment limit")]
    TooLarge,
    #[error("Telegram topic is closed or no longer available")]
    TopicUnavailable,
    #[error("Telegram message is missing or cannot be edited")]
    MessageUnavailable,
}

pub(crate) fn safe_error(error: RequestError) -> Error {
    match error {
        RequestError::Api(ApiError::Unknown(text)) if text == "Bad Request: TOPIC_NOT_MODIFIED" => {
            Error::NotModified
        }
        RequestError::Api(ApiError::Unknown(text))
            if matches!(
                text.as_str(),
                "Bad Request: TOPIC_CLOSED"
                    | "Bad Request: message thread not found"
                    | "Bad Request: MESSAGE_THREAD_NOT_FOUND"
            ) =>
        {
            Error::TopicUnavailable
        }
        RequestError::Api(ApiError::MessageNotModified) => Error::NotModified,
        RequestError::Api(ApiError::MessageToEditNotFound | ApiError::MessageCantBeEdited) => {
            Error::MessageUnavailable
        }
        RequestError::Api(
            ApiError::BotBlocked | ApiError::UserDeactivated | ApiError::ChatNotFound,
        ) => Error::NoRecipient,
        RequestError::Api(ApiError::Unknown(_)) => Error::Request,
        RequestError::Api(ApiError::InvalidToken) => Error::Authentication,
        RequestError::Network(error) if error.is_timeout() => Error::Timeout,
        RequestError::Network(_) => Error::Network,
        RequestError::InvalidJson { .. } => Error::Protocol,
        RequestError::RetryAfter(seconds) => Error::RetryAfter(u64::from(seconds.seconds())),
        RequestError::Api(_) => Error::Rejected,
        _ => Error::Request,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    pub bot_id: u64,
    pub username: Option<String>,
    pub webhook_enabled: bool,
    pub topics_enabled: bool,
    pub user_topic_creation_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Denied,
    PairingRequired,
    Authorized,
}

pub fn access(user: u64, chat: i64, private: bool, paired: bool) -> Access {
    if !private || chat <= 0 || u64::try_from(chat).ok() != Some(user) {
        return Access::Denied;
    }
    if paired {
        Access::Authorized
    } else {
        Access::PairingRequired
    }
}

pub struct Inspector {
    bot: Bot,
    timeout: Duration,
}

impl Inspector {
    pub fn new(token: SecretString, timeout: Duration) -> Self {
        // No from_env: ordinary settings and credentials come from explicit config.
        Self {
            bot: Bot::new(token.expose_secret()),
            timeout,
        }
    }

    pub async fn check(&self) -> Result<Identity, Error> {
        let me = api::account(&self.bot, self.timeout).await?;
        let hook = tokio::time::timeout(self.timeout, self.bot.get_webhook_info().send())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(safe_error)?;
        Ok(Identity {
            bot_id: me.id,
            username: me.username,
            webhook_enabled: hook.url.is_some(),
            topics_enabled: me.has_topics_enabled,
            user_topic_creation_enabled: me.allows_users_to_create_topics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_never_authorizes_a_group_or_another_private_peer() {
        assert_eq!(access(10, -100, false, true), Access::Denied);
        assert_eq!(access(10, 11, true, true), Access::Denied);
        assert_eq!(access(10, 10, true, false), Access::PairingRequired);
        assert_eq!(access(10, 10, true, true), Access::Authorized);
    }
}
