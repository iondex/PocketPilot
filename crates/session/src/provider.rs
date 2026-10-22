//! Platform-neutral observations and explicitly authorized runtime mutations.
//!
//! Providers translate native protocols into these models. They do not decide
//! platform authorization or operation deduplication; callers own those checks
//! and the durable dispatch claim before invoking [`AgentProvider::execute`].
use crate::{Capabilities, History, Page, Session, SessionQuery};
use serde::{Deserialize, Serialize};

/// The complete mutation vocabulary. Ordinary background observers have no lifecycle actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Mutation {
    /// Answer a live structured question using its current submission token.
    Answer {
        thread: String,
        submission: crate::questions::Submission,
    },
    /// Stage bytes in the runtime; this is a remote write even without a turn.
    Upload {
        thread: String,
        path: String,
        bytes: Vec<u8>,
    },
    /// Start a turn with files that have already been uploaded.
    StartWithFiles {
        thread: String,
        text: String,
        client_id: String,
        files: Vec<crate::files::Uploaded>,
    },
    /// Add uploaded files and input only if the expected turn is still active.
    SteerWithFiles {
        thread: String,
        expected_turn: String,
        text: String,
        client_id: String,
        files: Vec<crate::files::Uploaded>,
    },
    /// Create a native session in the explicitly selected project directory.
    Create { cwd: String },
    /// Load a session for control; listing and history must not call this implicitly.
    Resume { thread: String },
    /// Fill an empty native title during an explicit, authorized user action.
    SetTitleIfMissing { thread: String, title: String },
    /// Start a turn with a caller-assigned correlation ID, not a replay guarantee.
    Start {
        thread: String,
        text: String,
        client_id: String,
    },
    /// Add input only if the expected turn is still active.
    Steer {
        thread: String,
        expected_turn: String,
        text: String,
        client_id: String,
    },
}

/// Native acceptance of a mutation, not proof that an agent turn completed.
///
/// The caller must persist this receipt before reporting success. A missing
/// receipt after a timeout/disconnect does not imply that the mutation failed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub thread: String,
    pub turn: Option<String>,
}

/// Typed failures whose certainty determines whether dispatch stays unresolved.
///
/// Disconnect, timeout and incompatible responses cannot prove that a write was
/// rejected. Callers must preserve an unknown outcome rather than replay it.
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
    #[error(
        "invalid answer; use !answer followed by the current question code and one numbered answer per line"
    )]
    InvalidAnswer,
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
            Self::InvalidAnswer => ::i18n::Message::new("common.question.invalid"),
            Self::Conflict => ::i18n::Message::new("error.provider.conflict"),
        }
    }
}

/// Read native state without acquiring control; mutate only on explicit authority.
///
/// Implementations keep observations read-only and preserve native ownership of
/// sessions. A read must not start, resume or title a session as a side effect.
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

    /// Only dialogs owned by this live runtime connection/process are answerable.
    async fn questions(
        &self,
        _thread: &str,
    ) -> Result<Vec<crate::questions::Dialog>, ProviderError> {
        Ok(Vec::new())
    }

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

    /// Execute one durably recorded user action or an explicitly enabled retry policy.
    ///
    /// Callers own authorization, freshness checks and provider/session
    /// serialization. Implementations must not retry a write with an uncertain
    /// outcome; `client_id` correlates input but does not promise idempotency.
    ///
    /// # Errors
    ///
    /// Definite rejection is reported as `Rejected`, `Conflict`, `Unsupported`
    /// or `InvalidAnswer`. Other errors leave the write outcome unknown.
    ///
    /// # Cancellation
    ///
    /// Dropping this future cannot undo bytes already sent. The caller must
    /// retain its dispatch claim and recover it as an uncertain outcome.
    async fn execute(&self, _mutation: Mutation) -> Result<Receipt, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}
