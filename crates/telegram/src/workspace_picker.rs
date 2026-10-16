//! Stable workspace ordering and bounded numbered pages, shared by browsing and /new.
use super::*;

pub(super) const PAGE_SIZE: usize = 8;

pub(super) struct WorkspacePage<'a> {
    pub entries: Vec<&'a Workspace>,
    pub offset: usize,
    pub total: usize,
    pub next: Option<usize>,
}

impl WorkspacePage<'_> {
    pub fn counter(&self, input: &Input) -> String {
        ::i18n::t!(
            "tg.workspace.page",
            locale = crate::i18n::lang(input),
            start = if self.total == 0 { 0 } else { self.offset + 1 },
            end = self.offset + self.entries.len(),
            total = self.total
        )
    }
}

pub(super) fn page<'a>(
    config: &'a engine::config::Config,
    host: &str,
    offset: usize,
) -> WorkspacePage<'a> {
    let mut entries = config
        .workspaces
        .iter()
        .filter(|w| {
            config
                .providers
                .iter()
                .any(|p| p.id == w.provider_id && p.host_id == host)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.cwd.cmp(&b.cwd))
            .then_with(|| a.id.cmp(&b.id))
    });
    let total = entries.len();
    let offset = if total == 0 {
        0
    } else {
        offset.min((total - 1) / PAGE_SIZE * PAGE_SIZE)
    };
    let next = (offset + PAGE_SIZE < total).then_some(offset + PAGE_SIZE);
    let entries = entries.into_iter().skip(offset).take(PAGE_SIZE).collect();
    WorkspacePage {
        entries,
        total,
        offset,
        next,
    }
}

impl Service {
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
                catalog.refresh_host_workspaces(&snapshot.config, &self.connections, host),
            )
            .await;
            incomplete = !matches!(result,Ok(Ok(ref report)) if report.complete);
            if incomplete {
                tracing::info!(
                    host_id = host,
                    "workspace refresh incomplete; displaying registered projects"
                );
            }
            let mut config = self.config.snapshot().config.clone();
            catalog.load(&mut config).await?;
            self.config.publish_catalog(config)?;
        }
        Ok((self.config.snapshot(), incomplete))
    }
}
