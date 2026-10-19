pub mod config;

#[cfg(feature = "runtime")]
pub mod api;

#[cfg(feature = "runtime")]
pub mod model;

#[cfg(feature = "runtime")]
pub mod storage;

#[cfg(feature = "runtime")]
mod render;

#[cfg(feature = "runtime")]
pub mod runtime;

#[cfg(feature = "runtime")]
mod socket;

#[cfg(feature = "runtime")]
pub use runtime::Slack;

#[cfg(feature = "runtime")]
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "runtime")]
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
    Connection(#[from] engine::connections::Error),
    #[error(transparent)]
    Operation(#[from] engine::operations::Error),
}

#[cfg(feature = "runtime")]
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

#[cfg(all(test, feature = "runtime"))]
mod tests;
