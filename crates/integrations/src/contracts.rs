use engine::{reload::ConfigFile, runtimes::RuntimeRegistry, storage::Store};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Telegram,
    Slack,
    Discord,
    Feishu,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Feishu => "feishu",
        }
    }
}

#[derive(Clone)]
pub struct Context {
    pub config: Arc<ConfigFile>,
    pub store: Store,
    pub runtimes: Arc<RuntimeRegistry>,
}

#[derive(Debug, thiserror::Error)]
#[error("{platform} integration failed: {message}")]
pub struct IntegrationError {
    pub platform: &'static str,
    pub message: i18n::Message,
}

impl i18n::LocalizedError for IntegrationError {
    fn message(&self) -> i18n::Message {
        self.message.clone()
    }
}

#[async_trait::async_trait]
pub trait Integration: Send + Sync {
    fn platform(&self) -> Platform;

    fn name(&self) -> &'static str {
        self.platform().as_str()
    }

    fn capabilities(&self) -> crate::rendering::Capabilities;

    async fn run(
        &self,
        context: Context,
        stop: watch::Receiver<bool>,
    ) -> Result<(), IntegrationError>;
}

/// Download only after the caller validates identity, binding and freshness.
/// Implementations constrain URLs, redirects, timeout and size to their own platform.
#[async_trait::async_trait]
pub trait AttachmentSource: Send + Sync {
    type Error: i18n::LocalizedError + Send + Sync;

    async fn download(
        &self,
        file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded, Self::Error>;
}
