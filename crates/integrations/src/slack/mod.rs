//! Expose Slack integration entry points; transport, UI and durable state stay local.

#[path = "config.rs"]
pub mod config;

#[path = "api.rs"]
pub mod api;

#[path = "model.rs"]
pub mod model;

#[path = "storage.rs"]
pub mod storage;

#[path = "render.rs"]
pub(crate) mod render;

#[path = "markdown.rs"]
mod markdown;

#[path = "runtime.rs"]
pub mod runtime;

#[path = "socket.rs"]
mod socket;

#[path = "socket_model.rs"]
mod socket_model;

pub use runtime::Slack;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Slack request failed (response redacted)")]
    Request,
    #[error("Slack rate limited the request")]
    RateLimit(u64),
    #[error("Slack operation rejected: {0}")]
    Ui(&'static str),
    #[error("Slack state database failed")]
    Database(#[from] sqlx::Error),
    #[error("Slack data could not be encoded")]
    Encoding(#[from] serde_json::Error),
    #[error(transparent)]
    Store(#[from] engine::storage::StoreError),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error(transparent)]
    Connection(#[from] engine::runtimes::Error),
    #[error(transparent)]
    Operation(#[from] engine::operations::Error),
}

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        match self {
            Self::Ui(id) => i18n::Message::new(id),
            Self::RateLimit(_) => i18n::Message::new("slack.error.rate_limit"),
            Self::Request | Self::Encoding(_) | Self::Database(_) => {
                i18n::Message::new("error.slack.request")
            }
            Self::Store(e) => e.message(),
            Self::Provider(e) => e.message(),
            Self::Connection(e) => e.message(),
            Self::Operation(e) => e.message(),
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
