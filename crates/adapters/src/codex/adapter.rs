//! The Codex implementation of the runtime contract; native DTOs do not cross this seam.
use super::Codex;
use crate::{Adapter, AdapterFactory, ConnectOptions, RuntimeKind};
use session::ProviderError;
use std::sync::Arc;

pub struct Factory;

#[async_trait::async_trait]
impl AdapterFactory for Factory {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    async fn connect(&self, options: ConnectOptions) -> Result<Arc<dyn Adapter>, ProviderError> {
        let client = super::connect(options.endpoint, options.timeouts).await?;
        Ok(Arc::new(
            Codex::new(options.instance, client).with_control(options.control_verified),
        ))
    }
}

impl Adapter for Codex {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    fn is_connected(&self) -> bool {
        self.client().is_connected()
    }

    fn with_control(&self, verified: bool) -> Arc<dyn Adapter> {
        Arc::new(self.clone().with_control(verified))
    }
}
