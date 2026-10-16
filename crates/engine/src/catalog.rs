//! Dynamic providers and workspaces. Catalog writes never send an agent mutation.
use crate::{
    config::{Config, ConfigError, Provider, Workspace},
    storage::StoreError,
};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Catalog {
    pool: SqlitePool,
}

impl Catalog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Catalog administration can coexist with a running bridge. This handle has
    /// no operation-recovery or agent-dispatch API and takes no service lock.
    pub async fn open(directory: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(directory).map_err(|_| StoreError::Database)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| StoreError::Database)?;
        }
        let path = directory.join("state.db");

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&path)
                .map_err(|_| StoreError::Database)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| StoreError::Database)?;
        crate::storage::initialize_schema(&pool)
            .await
            .map_err(|_| StoreError::Database)?;
        Ok(Self::new(pool))
    }

    pub async fn load(&self, config: &mut Config) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
        let providers = sqlx::query("SELECT payload FROM catalog_providers ORDER BY id")
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
        let workspaces = sqlx::query("SELECT payload FROM catalog_workspaces ORDER BY id")
            .fetch_all(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
        tx.commit().await.map_err(|_| StoreError::Database)?;
        config.providers = providers
            .into_iter()
            .map(|r| {
                serde_json::from_str::<Provider>(r.get("payload")).map_err(|_| StoreError::Database)
            })
            .collect::<Result<Vec<_>, _>>()?;
        config
            .providers
            .retain(|p| config.hosts.iter().any(|h| h.id == p.host_id));
        config.workspaces = workspaces
            .into_iter()
            .map(|r| {
                serde_json::from_str::<Workspace>(r.get("payload"))
                    .map_err(|_| StoreError::Database)
            })
            .collect::<Result<Vec<_>, _>>()?;
        restrict(config);
        config.validate().map_err(|_| StoreError::Database)
    }

    pub async fn discover(
        &self,
        host: &str,
        endpoint: codex::discovery::Endpoint,
    ) -> Result<Provider, StoreError> {
        // Keep endpoint identity immutable. A changed socket is a new provider;
        // existing bindings and control attestations cannot silently follow it.
        let rows = sqlx::query("SELECT payload FROM catalog_providers WHERE host_id=?")
            .bind(host)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        for row in rows {
            let provider: Provider =
                serde_json::from_str(row.get("payload")).map_err(|_| StoreError::Database)?;
            if provider.endpoint.as_ref() == Some(&endpoint) {
                return Ok(provider);
            }
        }
        let provider = Provider {
            id: uuid::Uuid::new_v4().to_string(),
            host_id: host.into(),
            kind: "codex".into(),
            connection_mode: "existing_proxy".into(),
            compatibility_profile: "latest-tested-codex".into(),
            control_verified: false,
            endpoint: Some(endpoint),
        };
        let payload = serde_json::to_string(&provider).map_err(|_| StoreError::Database)?;
        sqlx::query("INSERT INTO catalog_providers(id,host_id,payload) VALUES(?,?,?) ON CONFLICT DO NOTHING")
            .bind(&provider.id).bind(host).bind(payload).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        let row=sqlx::query("SELECT payload FROM catalog_providers WHERE host_id=? AND json_extract(payload,'$.endpoint.binary')=? AND json_extract(payload,'$.endpoint.socket')=?")
            .bind(host).bind(&provider.endpoint.as_ref().ok_or(StoreError::Database)?.binary).bind(&provider.endpoint.as_ref().ok_or(StoreError::Database)?.socket).fetch_one(&self.pool).await.map_err(|_|StoreError::Database)?;
        let provider: Provider =
            serde_json::from_str(row.get("payload")).map_err(|_| StoreError::Database)?;
        tracing::info!(host_id=host,provider_id=%provider.id,"discovered provider; control remains read-only");
        Ok(provider)
    }

    pub async fn save_provider(&self, provider: &Provider) -> Result<(), StoreError> {
        let payload = serde_json::to_string(provider).map_err(|_| StoreError::Database)?;
        sqlx::query("INSERT INTO catalog_providers(id,host_id,payload) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload WHERE catalog_providers.host_id=excluded.host_id")
            .bind(&provider.id).bind(&provider.host_id).bind(payload).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }

    pub async fn add_workspace(
        &self,
        config: &Config,
        provider: &str,
        cwd: &str,
        name: Option<&str>,
    ) -> Result<Workspace, ConfigError> {
        self.register_workspace(config, provider, cwd, name, false)
            .await?
            .ok_or(ConfigError::Invalid)
    }

    pub async fn remove_workspace(&self, id: &str) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
        sqlx::query("INSERT INTO workspace_exclusions(provider_id,cwd) SELECT provider_id,json_extract(payload,'$.cwd') FROM catalog_workspaces WHERE id=? ON CONFLICT DO NOTHING")
            .bind(id).execute(&mut *tx).await.map_err(|_|StoreError::Database)?;
        sqlx::query("DELETE FROM catalog_workspaces WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
        tx.commit().await.map_err(|_| StoreError::Database)?;
        tracing::info!(
            workspace_id = id,
            "workspace removed from catalog; remote session unchanged"
        );
        Ok(())
    }
}

pub fn restrict(config: &mut Config) {
    config
        .providers
        .retain(|p| config.hosts.iter().any(|h| h.id == p.host_id));
    config.workspaces.retain(|w| {
        config
            .providers
            .iter()
            .find(|p| p.id == w.provider_id)
            .and_then(|p| config.hosts.iter().find(|h| h.id == p.host_id))
            .is_some_and(|h| {
                std::iter::once(&w.cwd)
                    .chain(w.additional_cwds.iter())
                    .all(|cwd| {
                        h.allowed_workspace_roots
                            .iter()
                            .any(|r| Path::new(cwd).starts_with(r))
                    })
            })
    });
}

#[path = "workspace_discovery.rs"]
mod workspace_discovery;

pub use workspace_discovery::WorkspaceDiscovery;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
