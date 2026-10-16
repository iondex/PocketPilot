use crate::{connections::Connections, reload::ConfigFile, storage::Store};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone)]
pub struct Context {
    pub config: Arc<ConfigFile>,
    pub store: Store,
    pub connections: Arc<Connections>,
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
    fn name(&self) -> &'static str;

    async fn run(
        &self,
        context: Context,
        stop: watch::Receiver<bool>,
    ) -> Result<(), IntegrationError>;
}
