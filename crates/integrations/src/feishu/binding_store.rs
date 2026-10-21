//! The common binding manager owns transitions; only Feishu scope and SQL live here.
use super::{config::Config, storage::Store};
use crate::bindings::{Change, Pin, Record, Repository, Scope, Target};
use engine::storage::StoreError;
use sqlx::{Row, Sqlite, Transaction};

pub(super) fn scope(config: &Config) -> Scope {
    Scope::Feishu {
        app: config.app_id.clone(),
        tenant: config.tenant_key.clone(),
        owner: config.owner_open_id.clone(),
    }
}

impl Store {
    fn scoped(&self, scope: &Scope) -> bool {
        matches!(scope,Scope::Feishu {app,tenant,owner} if self.namespace==format!("feishu:{app}:{tenant}:{owner}"))
    }
}

#[async_trait::async_trait]
impl Repository for Store {
    fn core(&self) -> &engine::storage::Store {
        &self.core
    }

    async fn records(&self, scope: &Scope) -> Result<Vec<Record>, StoreError> {
        if !self.scoped(scope) {
            return Err(StoreError::ContainerConflict);
        }
        let rows=sqlx::query("SELECT id,root,project,instance,session,cwd,endpoint,active,revision FROM feishu_bindings WHERE namespace=? AND state IN ('ready','unknown') ORDER BY rowid DESC LIMIT 1024")
            .bind(&self.namespace).fetch_all(self.core.pool()).await.map_err(|_|StoreError::Database)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                target: Target::Feishu { id: row.get("id") },
                reference: session::SessionRef {
                    instance: row.get("instance"),
                    thread: row.get("session"),
                },
                project: row.get("project"),
                pin: Some(Pin {
                    cwd: row.get("cwd"),
                    endpoint: row.get("endpoint"),
                }),
                active: row.get("active"),
                revision: row.get("revision"),
                title: None,
                root: Some(row.get("root")),
            })
            .collect())
    }

    async fn claim(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        scope: &Scope,
        target: &Target,
        revision: i64,
        change: Change,
    ) -> Result<bool, StoreError> {
        if !self.scoped(scope) {
            return Err(StoreError::ContainerConflict);
        }
        let Target::Feishu { id } = target else {
            return Err(StoreError::ContainerConflict);
        };
        let detach = matches!(change, Change::Detach);
        let n=sqlx::query("UPDATE feishu_bindings SET active=CASE WHEN ? THEN 0 ELSE active END,revision=revision+?,state=CASE WHEN ? THEN 'ready' ELSE state END WHERE namespace=? AND id=? AND revision=? AND active=1 AND state IN ('ready','unknown')")
            .bind(detach).bind(i64::from(detach)).bind(detach).bind(&self.namespace).bind(id).bind(revision).execute(&mut **tx).await.map_err(|_|StoreError::Database)?.rows_affected();
        Ok(n == 1)
    }
}
