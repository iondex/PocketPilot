//! The common binding manager owns transitions; only Feishu/Lark scope and SQL live here.
use super::{
    config::{Config, Region},
    storage::Store,
};
use crate::bindings::{Change, Pin, Record, Repository, Scope, Target};
use engine::storage::StoreError;
use sqlx::{Row, Sqlite, Transaction};

pub(super) fn scope(region: Region, config: &Config) -> Scope {
    match region {
        Region::Feishu => Scope::Feishu {
            app: config.app_id.clone(),
            tenant: config.tenant_key.clone(),
            owner: config.owner_open_id.clone(),
        },
        Region::Lark => Scope::Lark {
            app: config.app_id.clone(),
            tenant: config.tenant_key.clone(),
            owner: config.owner_open_id.clone(),
        },
    }
}

impl Store {
    fn scoped(&self, scope: &Scope) -> bool {
        let (region, app, tenant, owner) = match scope {
            Scope::Feishu { app, tenant, owner } => (Region::Feishu, app, tenant, owner),
            Scope::Lark { app, tenant, owner } => (Region::Lark, app, tenant, owner),
            _ => return false,
        };
        self.region == region
            && self.namespace == format!("{}:{app}:{tenant}:{owner}", region.as_str())
    }

    pub(super) fn target(&self, id: String) -> Target {
        match self.region {
            Region::Feishu => Target::Feishu { id },
            Region::Lark => Target::Lark { id },
        }
    }

    /// Reject a target from the other platform even if its external ID is identical.
    pub(super) fn binding_id<'a>(&self, target: &'a Target) -> Result<&'a str, StoreError> {
        match (self.region, target) {
            (Region::Feishu, Target::Feishu { id }) | (Region::Lark, Target::Lark { id }) => Ok(id),
            _ => Err(StoreError::ContainerConflict),
        }
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
        let rows = sqlx::query(
            "SELECT id,root,project,instance,session,cwd,endpoint,active,revision \
            FROM feishu_bindings WHERE namespace=? AND state IN ('ready','unknown') \
            ORDER BY rowid DESC LIMIT 1024",
        )
        .bind(&self.namespace)
        .fetch_all(self.core.pool())
        .await
        .map_err(|_| StoreError::Database)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                target: self.target(row.get("id")),
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
        let id = self.binding_id(target)?;
        let detach = matches!(change, Change::Detach);
        let n = sqlx::query(
            "UPDATE feishu_bindings SET active=CASE WHEN ? THEN 0 ELSE active END,\
            revision=revision+?,state=CASE WHEN ? THEN 'ready' ELSE state END WHERE \
            namespace=? AND id=? AND revision=? AND active=1 AND state IN ('ready',\
            'unknown')",
        )
        .bind(detach)
        .bind(i64::from(detach))
        .bind(detach)
        .bind(&self.namespace)
        .bind(id)
        .bind(revision)
        .execute(&mut **tx)
        .await
        .map_err(|_| StoreError::Database)?
        .rows_affected();
        Ok(n == 1)
    }
}
