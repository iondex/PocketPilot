use crate::{Capabilities, History, Page, Session, SessionQuery};
use serde::{Deserialize, Serialize};

/// The complete mutation vocabulary. Ordinary background observers have no lifecycle actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Mutation {
    Upload {
        thread: String,
        path: String,
        bytes: Vec<u8>,
    },
    StartWithFiles {
        thread: String,
        text: String,
        client_id: String,
        files: Vec<crate::files::Uploaded>,
    },
    SteerWithFiles {
        thread: String,
        expected_turn: String,
        text: String,
        client_id: String,
        files: Vec<crate::files::Uploaded>,
    },
    Create {
        cwd: String,
    },
    Resume {
        thread: String,
    },
    SetTitleIfMissing {
        thread: String,
        title: String,
    },
    Start {
        thread: String,
        text: String,
        client_id: String,
    },
    Steer {
        thread: String,
        expected_turn: String,
        text: String,
        client_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub thread: String,
    pub turn: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider disconnected")]
    Disconnected,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider rejected request (code {0})")]
    Rejected(i64),
    #[error("provider protocol is incompatible")]
    Incompatible,
    #[error("operation is unsupported by this provider")]
    Unsupported,
    #[error("session state changed; refresh and confirm the input again")]
    Conflict,
}

impl ::i18n::LocalizedError for ProviderError {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Disconnected => ::i18n::Message::new("error.provider.disconnected"),
            Self::Timeout => ::i18n::Message::new("error.provider.timeout"),
            Self::Rejected(value) => {
                ::i18n::Message::new("error.provider.rejected").arg("code", value)
            }
            Self::Incompatible => ::i18n::Message::new("error.provider.incompatible"),
            Self::Unsupported => ::i18n::Message::new("error.provider.unsupported"),
            Self::Conflict => ::i18n::Message::new("error.provider.conflict"),
        }
    }
}

#[async_trait::async_trait]
pub trait AgentProvider: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    async fn list(&self, query: SessionQuery) -> Result<Page<Session>, ProviderError>;

    async fn read(&self, thread: &str) -> Result<Session, ProviderError>;

    /// Fetch the current turn precondition when metadata alone cannot provide it.
    async fn active_turn(&self, thread: &str) -> Result<Option<String>, ProviderError> {
        Ok(self.read(thread).await?.active_turn)
    }

    async fn history(&self, thread: &str) -> Result<History, ProviderError>;

    /// Wake observers on lifecycle changes; polling remains necessary after event loss.
    fn session_changes(&self) -> Option<tokio::sync::watch::Receiver<()>> {
        None
    }

    /// Inspect only the latest terminal turn, never an error quoted in message text.
    async fn retryable_failure(
        &self,
        _thread: &str,
    ) -> Result<Option<crate::retry::RetryableFailure>, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    /// Called only for a durably recorded user action or an explicitly enabled retry policy.
    async fn execute(&self, _mutation: Mutation) -> Result<Receipt, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}
