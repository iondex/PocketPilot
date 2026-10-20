//! Runtime identity, lifecycle and construction, independent of any native wire protocol.
use connections::{Endpoint, Timeouts};
use serde::{Deserialize, Serialize};
use session::{AgentProvider, ProviderError};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Codex,
    ClaudeCode,
    Pi,
}

impl RuntimeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Pi => "pi",
        }
    }
}

/// Location of an existing runtime server; discovering it must not start a daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEndpoint {
    pub binary: String,
    pub socket: String,
}

/// A fresh connection attempt, using the caller's verified control grant.
pub struct ConnectOptions {
    pub instance: String,
    pub endpoint: Endpoint,
    pub timeouts: Timeouts,
    pub control_verified: bool,
}

/// Session operations are inherited from AgentProvider, avoiding a second mutation vocabulary.
/// Dropping this handle disconnects a client; it must never stop the native runtime.
pub trait Adapter: AgentProvider {
    fn kind(&self) -> RuntimeKind;

    fn is_connected(&self) -> bool;

    /// Publish a new immutable capability snapshot when the operator grant changes.
    fn with_control(&self, verified: bool) -> Arc<dyn Adapter>;
}

#[async_trait::async_trait]
pub trait AdapterFactory: Send + Sync {
    fn kind(&self) -> RuntimeKind;

    /// Connect and negotiate without creating or resuming any session.
    async fn connect(&self, options: ConnectOptions) -> Result<Arc<dyn Adapter>, ProviderError>;
}
