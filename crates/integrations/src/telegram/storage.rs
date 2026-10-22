use engine::storage::Store as CoreStore;
pub use engine::storage::StoreError;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
    core: CoreStore,
    pub(crate) owner: super::Owner,
}

impl std::ops::Deref for Store {
    type Target = CoreStore;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl Store {
    pub fn new(core: CoreStore, owner: super::Owner) -> Self {
        Self {
            pool: core.pool().clone(),
            core,
            owner,
        }
    }

    pub fn core(&self) -> &CoreStore {
        &self.core
    }

    pub async fn open(path: &std::path::Path, owner: super::Owner) -> Result<Self, StoreError> {
        let store = Self::new(CoreStore::open(path).await?, owner);
        store.recover_deliveries().await?;
        Ok(store)
    }

    pub(crate) async fn recover_deliveries(&self) -> Result<(), StoreError> {
        sqlx::query("UPDATE deliveries SET state='outcome_unknown' WHERE state='dispatching'")
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        Ok(())
    }

    pub async fn bind(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        instance: &str,
        thread: &str,
    ) -> Result<(), StoreError> {
        if !self.owner.chat(chat) {
            return Err(StoreError::Unauthorized);
        }
        let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
        let removing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM topic_removals WHERE bot_id=? AND chat_id=? AND topic_id=? AND state<>'rejected')")
            .bind(bot).bind(chat).bind(topic).fetch_one(&mut *tx).await.map_err(|_|StoreError::Database)?;
        if removing {
            return Err(StoreError::ContainerConflict);
        }
        sqlx::query("INSERT INTO containers(bot_id,chat_id,topic_id,instance_id,thread_id) VALUES(?,?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(bot).bind(chat).bind(topic).bind(instance).bind(thread).execute(&mut *tx).await.map_err(|_| StoreError::Database)?;
        let row = sqlx::query(
            "SELECT instance_id,thread_id FROM containers WHERE bot_id=? AND chat_id=? AND topic_id=?",
        )
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StoreError::Database)?;
        if row.get::<String, _>("instance_id") != instance
            || row.get::<String, _>("thread_id") != thread
        {
            return Err(StoreError::ContainerConflict);
        }
        sqlx::query("INSERT INTO bindings(bot_id,chat_id,topic_id,instance_id,thread_id) VALUES(?,?,?,?,?) ON CONFLICT(bot_id,chat_id,topic_id) DO UPDATE SET active=1,revision=revision+1")
            .bind(bot).bind(chat).bind(topic).bind(instance).bind(thread).execute(&mut *tx).await.map_err(|_| StoreError::Database)?;
        tx.commit().await.map_err(|_| StoreError::Database)
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

#[path = "binding_store.rs"]
mod binding_store;
