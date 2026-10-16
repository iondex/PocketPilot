//! Discover project cwd metadata through all session pages; never load a turn.
use super::*;
use crate::connections::{Connections, Error};
use session::{AgentProvider, ProviderError, SessionQuery};
use std::{collections::HashSet, path::Component};

#[derive(Default, Debug)]
pub struct WorkspaceDiscovery {
    pub added: usize,
    pub pages: usize,
    pub complete: bool,
}

impl Catalog {
    pub async fn touch_workspace(
        &self,
        id: &str,
        updated_at: Option<i64>,
    ) -> Result<(), StoreError> {
        if let Some(at) = updated_at.filter(|at| *at > 0) {
            sqlx::query("UPDATE catalog_workspaces SET payload=json_set(payload,'$.updated_at',?) WHERE id=? AND COALESCE(json_extract(payload,'$.updated_at'),0)<?")
                .bind(at).bind(id).bind(at).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        }
        Ok(())
    }

    pub(super) async fn register_workspace(
        &self,
        config: &Config,
        provider: &str,
        cwd: &str,
        name: Option<&str>,
        automatic: bool,
    ) -> Result<Option<Workspace>, ConfigError> {
        let host = config
            .providers
            .iter()
            .find(|p| p.id == provider)
            .and_then(|p| config.hosts.iter().find(|h| h.id == p.host_id))
            .ok_or(ConfigError::Reference)?;
        let path = Path::new(cwd);
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || !host
                .allowed_workspace_roots
                .iter()
                .any(|root| path.starts_with(root))
        {
            return Err(ConfigError::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(|_| ConfigError::Io)?;
        if automatic {
            let excluded: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM workspace_exclusions WHERE provider_id=? AND cwd=?)",
            )
            .bind(provider)
            .bind(cwd)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| ConfigError::Io)?;
            if excluded {
                return Ok(None);
            }
        } else {
            sqlx::query("DELETE FROM workspace_exclusions WHERE provider_id=? AND cwd=?")
                .bind(provider)
                .bind(cwd)
                .execute(&mut *tx)
                .await
                .map_err(|_| ConfigError::Io)?;
        }
        let existing:Option<String>=sqlx::query_scalar("SELECT payload FROM catalog_workspaces WHERE provider_id=? AND json_extract(payload,'$.cwd')=?")
            .bind(provider).bind(cwd).fetch_optional(&mut *tx).await.map_err(|_|ConfigError::Io)?;
        if let Some(existing) = existing {
            let workspace: Workspace =
                serde_json::from_str(&existing).map_err(|_| ConfigError::Invalid)?;
            let mut next = config.clone();
            next.workspaces.retain(|w| w.id != workspace.id);
            next.workspaces.push(workspace.clone());
            next.validate()?;
            tx.commit().await.map_err(|_| ConfigError::Io)?;
            return Ok(Some(workspace));
        }
        let occupied: Vec<String> = sqlx::query_scalar(
            "SELECT json_extract(payload,'$.short_name') FROM catalog_workspaces",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| ConfigError::Io)?;
        let mut workspace = config
            .workspaces
            .iter()
            .find(|w| w.provider_id == provider && w.cwd == cwd)
            .cloned()
            .unwrap_or_else(|| {
                let id = uuid::Uuid::new_v4().to_string();
                let basename = Path::new(cwd)
                    .file_name()
                    .and_then(|p| p.to_str())
                    .unwrap_or("workspace");
                let name = name.filter(|n| !n.trim().is_empty()).unwrap_or(basename);
                let short_name = if occupied.iter().any(|n| n == name)
                    || config.workspaces.iter().any(|w| w.short_name == name)
                {
                    format!("{name}-{}", &id[..8])
                } else {
                    name.into()
                };
                Workspace {
                    id,
                    provider_id: provider.into(),
                    cwd: cwd.into(),
                    name: name.into(),
                    short_name,
                    additional_cwds: vec![],
                    updated_at: None,
                }
            });
        // Avoid alias races across hosts: the SQL transaction owns selection and insertion.
        while occupied.contains(&workspace.short_name) {
            workspace.short_name = format!("{}-{}", workspace.name, uuid::Uuid::new_v4().simple());
        }
        let mut next = config.clone();
        next.workspaces.retain(|w| w.id != workspace.id);
        next.workspaces.push(workspace.clone());
        next.validate()?;
        let payload = serde_json::to_string(&workspace).map_err(|_| ConfigError::Invalid)?;
        sqlx::query("INSERT INTO catalog_workspaces(id,provider_id,payload) VALUES(?,?,?)")
            .bind(&workspace.id)
            .bind(provider)
            .bind(payload)
            .execute(&mut *tx)
            .await
            .map_err(|_| ConfigError::Io)?;
        tx.commit().await.map_err(|_| ConfigError::Io)?;
        tracing::info!(workspace_id=%workspace.id,provider_id=provider,automatic,"workspace registered; no agent mutation");
        Ok(Some(workspace))
    }

    pub async fn discover_workspaces(
        &self,
        config: &Config,
        provider: &str,
        agent: &dyn AgentProvider,
    ) -> Result<WorkspaceDiscovery, Error> {
        let host = config
            .providers
            .iter()
            .find(|p| p.id == provider)
            .and_then(|p| config.hosts.iter().find(|h| h.id == p.host_id))
            .ok_or(Error::Selection)?;
        let mut report = WorkspaceDiscovery {
            complete: true,
            ..Default::default()
        };
        let stored: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM catalog_workspaces WHERE provider_id=?")
                .bind(provider)
                .fetch_all(&self.pool)
                .await
                .map_err(|_| StoreError::Database)?;
        let mut known = HashSet::new();
        let mut owners = std::collections::HashMap::new();
        for payload in stored {
            let workspace: Workspace =
                serde_json::from_str(&payload).map_err(|_| StoreError::Database)?;
            for cwd in std::iter::once(workspace.cwd).chain(workspace.additional_cwds) {
                owners.insert(cwd.clone(), workspace.id.clone());
                known.insert(cwd);
            }
        }
        for archived in [false, true] {
            let mut cursor = None;
            let mut seen = HashSet::new();
            let mut finished = false;
            for _ in 0..1000 {
                if !seen.insert(cursor.clone()) {
                    return Err(ProviderError::Incompatible.into());
                }
                let page = agent
                    .list(SessionQuery {
                        cwd: None,
                        cursor,
                        limit: 100,
                        archived,
                    })
                    .await?;
                report.pages += 1;
                for session in page.data {
                    let cwd = session.cwd;
                    let path = Path::new(&cwd);
                    if session.reference.instance == provider
                        && let Some(owner) = owners.get(&cwd)
                    {
                        self.touch_workspace(owner, session.updated_at).await?;
                    }
                    if session.reference.instance != provider
                        || known.contains(&cwd)
                        || !path.is_absolute()
                        || path
                            .components()
                            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
                        || !host
                            .allowed_workspace_roots
                            .iter()
                            .any(|root| path.starts_with(root))
                    {
                        continue;
                    }
                    if let Some(workspace) = self
                        .register_workspace(config, provider, &cwd, None, true)
                        .await?
                    {
                        self.touch_workspace(&workspace.id, session.updated_at)
                            .await?;
                        owners.insert(cwd.clone(), workspace.id);
                        report.added += 1;
                    }
                    known.insert(cwd);
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    finished = true;
                    break;
                }
            }
            report.complete &= finished;
        }
        tracing::info!(
            provider_id = provider,
            pages = report.pages,
            added = report.added,
            complete = report.complete,
            "workspace metadata discovery finished"
        );
        Ok(report)
    }

    pub async fn refresh_host_workspaces(
        &self,
        config: &Config,
        connections: &Connections,
        host: &str,
    ) -> Result<WorkspaceDiscovery, Error> {
        let actor = connections.actor("__workspace_catalog", host).await;
        let _guard = actor.lock().await;
        let mut report = WorkspaceDiscovery {
            complete: true,
            ..Default::default()
        };
        for provider in config.providers.iter().filter(|p| p.host_id == host) {
            let agent = connections.get_provider(config, &provider.id).await?;
            let result = self
                .discover_workspaces(config, &provider.id, agent.as_ref())
                .await?;
            report.added += result.added;
            report.pages += result.pages;
            report.complete &= result.complete;
        }
        Ok(report)
    }
}
