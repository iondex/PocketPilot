//! Agent runtime adapters. The first implementation wraps Codex app-server.
use session::AgentProvider;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Codex,
    ClaudeCode,
    Pi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    pub list: bool,
    pub observe: bool,
    pub history: bool,
    pub control: bool,
    pub live_attach: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("runtime adapter is unavailable")]
    Unavailable,
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
}

#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    fn kind(&self) -> RuntimeKind;
    fn capabilities(&self) -> Capabilities;
    fn provider(&self) -> Arc<dyn session::AgentProvider>;
}

/// Codex is the current complete adapter. Claude Code and Pi remain planned drivers.
#[derive(Clone)]
pub struct CodexAdapter {
    provider: Arc<codex::Codex>,
}

impl CodexAdapter {
    pub fn new(provider: codex::Codex) -> Self {
        Self {
            provider: Arc::new(provider),
        }
    }

    pub fn from_arc(provider: Arc<codex::Codex>) -> Self {
        Self { provider }
    }

    pub fn is_connected(&self) -> bool {
        self.provider.client().is_connected()
    }

    pub fn with_control(self, verified: bool) -> Self {
        Self::from_arc(Arc::new((*self.provider).clone().with_control(verified)))
    }
}

impl Adapter for CodexAdapter {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    fn capabilities(&self) -> Capabilities {
        let capabilities = self.provider.capabilities();
        Capabilities {
            list: capabilities.list,
            observe: capabilities.live_status,
            history: capabilities.history,
            control: capabilities.control_verified,
            live_attach: capabilities.live_status,
        }
    }

    fn provider(&self) -> Arc<dyn session::AgentProvider> {
        self.provider.clone()
    }
}
