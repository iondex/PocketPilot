//! Scoped SQL adapter for the shared binding manager.
use super::Store;
use crate::bindings::{Change, Record, Repository, Scope, Target};
use engine::storage::{Store as CoreStore, StoreError};
use sqlx::{Row, Sqlite, Transaction};

#[async_trait::async_trait]
impl Repository for Store {
    fn core(&self) -> &CoreStore {
        Store::core(self)
    }

    async fn records(&self, scope: &Scope) -> Result<Vec<Record>, StoreError> {
        let Scope::Telegram { bot, chat } = scope else {
            return Err(StoreError::ContainerConflict);
        };
        if *chat <= 0 {
            return Err(StoreError::NotPaired);
        }
        let rows = sqlx::query(
            "SELECT topic_id,instance_id,thread_id,workspace_id,active,revision,display_name,topic_title FROM
            bindings WHERE bot_id=? AND chat_id=? ORDER BY topic_id",
        )
        .bind(bot)
        .bind(chat)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                target: Target::Telegram {
                    topic: row.get("topic_id"),
                },
                reference: session::SessionRef {
                    instance: row.get("instance_id"),
                    thread: row.get("thread_id"),
                },
                project: row
                    .get::<Option<String>, _>("workspace_id")
                    .unwrap_or_default(),
                pin: None,
                active: row.get("active"),
                revision: row.get("revision"),
                title: row
                    .get::<Option<String>, _>("display_name")
                    .or(row.get("topic_title")),
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
        let (Scope::Telegram { bot, chat }, Target::Telegram { topic }) = (scope, target) else {
            return Err(StoreError::ContainerConflict);
        };
        if *chat <= 0 {
            return Err(StoreError::NotPaired);
        }
        let detach = matches!(change, Change::Detach);
        let changed = sqlx::query(
            "UPDATE bindings SET active=CASE WHEN ? THEN 0 ELSE active END,revision=revision+? WHERE bot_id=?
            AND chat_id=? AND topic_id=? AND revision=? AND active=1",
        )
        .bind(detach)
        .bind(i64::from(detach))
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .bind(revision)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Database)?.rows_affected();
        Ok(changed == 1)
    }
}
