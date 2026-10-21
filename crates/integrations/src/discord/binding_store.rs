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
        let Scope::Discord { bot, guild, owner } = scope else {
            return Err(StoreError::ContainerConflict);
        };
        let rows = sqlx::query(
            "SELECT channel,instance,thread,workspace,cwd,endpoint,active,revision FROM discord_bindings
            WHERE bot=? AND guild=? AND owner=? ORDER BY channel",
        )
        .bind(bot.to_string())
        .bind(guild.to_string())
        .bind(owner.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                target: Target::Discord {
                    channel: row.get("channel"),
                },
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
                root: None,
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
        let (Scope::Discord { bot, guild, owner }, Target::Discord { channel }) = (scope, target)
        else {
            return Err(StoreError::ContainerConflict);
        };
        let detach = matches!(change, Change::Detach);
        let changed = sqlx::query(
            "UPDATE discord_bindings SET active=CASE WHEN ? THEN 0 ELSE active END,revision=revision+? WHERE
            bot=? AND guild=? AND owner=? AND channel=? AND revision=? AND active=1",
        )
        .bind(detach)
        .bind(i64::from(detach))
        .bind(bot.to_string())
        .bind(guild.to_string())
        .bind(owner.to_string())
        .bind(channel)
        .bind(revision)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Database)?.rows_affected();
        Ok(changed == 1)
    }
}
