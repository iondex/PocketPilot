//! Domestic Feishu bot: private navigation, explicitly scoped group threads and typed SDK boundaries.
pub mod config;

mod api;

mod model;

mod render;

pub(crate) mod storage;

mod binding_store;

mod socket;

mod runtime;

pub use runtime::Feishu;

pub use api::Identity;

/// Read-only application identity probe; does not subscribe, send messages or run an agent.
pub async fn inspect(settings: &config::Config) -> Result<Identity> {
    use api::Platform;
    api::Api::new(settings)?.identity().await
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Feishu transport unavailable (details redacted)")]
    Request,
    #[error("Feishu API rejected the request (code {0})")]
    Api(i32),
    #[error("Feishu state unavailable")]
    Database(#[from] sqlx::Error),
    #[error("Feishu payload could not be encoded")]
    Encoding(#[from] serde_json::Error),
    #[error("Feishu action rejected: {0}")]
    Ui(&'static str),
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
            Self::Api(code) => i18n::Message::new("feishu.error.api").arg("code", code),
            Self::Request | Self::Database(_) | Self::Encoding(_) => {
                i18n::Message::new("feishu.error.request")
            }
            Self::Store(e) => e.message(),
            Self::Provider(e) => e.message(),
            Self::Connection(e) => e.message(),
            Self::Operation(e) => e.message(),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sdk_tests;

impl Error {
    /// Preserve numeric API diagnostics, never upstream bodies or credential URLs.
    fn from_sdk(error: openlark_core::error::CoreError) -> Self {
        match error {
            openlark_core::error::CoreError::Api(api) => Self::Api(api.raw_code),
            _ => Self::Request,
        }
    }
}
