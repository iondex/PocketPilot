//! Scoped SQL adapter for the shared binding manager.
use super::Store;
use crate::bindings::{Change, Pin, Record, Repository, Scope, Target};
use engine::storage::{Store as CoreStore, StoreError};
use sqlx::{Row, Sqlite, Transaction};

#[async_trait::async_trait]
impl Repository for Store {
    fn core(&self) -> &CoreStore {
        &self.core
    }

    async fn records(&self, scope: &Scope) -> Result<Vec<Record>, StoreError> {
        let Scope::Slack {
            app,
            team,
            bot,
            channel,
            owner,
        } = scope
        else {
            return Err(StoreError::ContainerConflict);
        };
        if self.namespace != format!("{app}:{team}:{bot}") {
            return Err(StoreError::ContainerConflict);
        }
        let rows = sqlx::query(
            "SELECT id,root,instance,thread,workspace,cwd,endpoint,active,revision FROM slack_bindings WHERE
            app=? AND team=? AND bot=? AND channel=? AND owner=? ORDER BY root DESC",
        )
        .bind(app)
        .bind(team)
        .bind(bot)
        .bind(channel)
        .bind(owner)
        .fetch_all(self.core.pool())
        .await
        .map_err(|_| StoreError::Database)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                target: Target::Slack { id: row.get("id") },
                reference: session::SessionRef {
                    instance: row.get("instance"),
                    thread: row.get("thread"),
                },
                project: row.get("workspace"),
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
        transaction: &mut Transaction<'_, Sqlite>,
        scope: &Scope,
        target: &Target,
        revision: i64,
        change: Change,
    ) -> Result<bool, StoreError> {
        let (
            Scope::Slack {
                app,
                team,
                bot,
                channel,
                owner,
            },
            Target::Slack { id },
        ) = (scope, target)
        else {
            return Err(StoreError::ContainerConflict);
        };
        if self.namespace != format!("{app}:{team}:{bot}") {
            return Err(StoreError::ContainerConflict);
        }
        let detach = matches!(change, Change::Detach);
        let changed = sqlx::query(
            "UPDATE slack_bindings SET active=CASE WHEN ? THEN 0 ELSE active END,revision=revision+? WHERE
            id=? AND app=? AND team=? AND bot=? AND channel=? AND owner=? AND revision=? AND active=1",
        )
        .bind(detach)
        .bind(i64::from(detach))
        .bind(id)
        .bind(app)
        .bind(team)
        .bind(bot)
        .bind(channel)
        .bind(owner)
        .bind(revision)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Database)?.rows_affected();
        Ok(changed == 1)
    }
}
