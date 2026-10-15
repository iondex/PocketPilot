use crate::{Capabilities, History, Page, Session, SessionQuery};

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
}

#[async_trait::async_trait]
pub trait AgentProvider: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    async fn list(&self, query: SessionQuery) -> Result<Page<Session>, ProviderError>;
    async fn read(&self, thread: &str) -> Result<Session, ProviderError>;
    async fn history(&self, thread: &str) -> Result<History, ProviderError>;
}
