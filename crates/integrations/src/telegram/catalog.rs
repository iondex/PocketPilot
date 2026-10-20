//! Explicit catalog refresh for Telegram management and creation flows.
use super::*;

impl Service {
    pub(super) async fn refresh_catalog_snapshot(
        &self,
        catalog: &engine::catalog::Catalog,
    ) -> Result<Arc<Snapshot>, Error> {
        let mut config = self.config.snapshot().config.clone();
        catalog.load(&mut config).await?;
        self.config.publish_catalog(config)?;
        Ok(self.config.snapshot())
    }

    pub(super) async fn workspace_snapshot(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        host: &str,
    ) -> Result<(Arc<Snapshot>, bool), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let mut incomplete = false;
        if input
            .text
            .split_whitespace()
            .next()
            .is_some_and(|s| s.split('@').next() == Some("/refresh"))
        {
            let catalog = engine::catalog::Catalog::new(self.store.pool().clone());
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(12),
                catalog.refresh_host_workspaces(&snapshot.config, &self.runtimes, host),
            )
            .await;
            incomplete = !matches!(result,Ok(Ok(ref report)) if report.complete);
            if incomplete {
                tracing::info!(
                    host_id = host,
                    "workspace refresh incomplete; displaying registered projects"
                );
            }
            self.refresh_catalog_snapshot(&catalog).await?;
        }
        Ok((self.config.snapshot(), incomplete))
    }
}
