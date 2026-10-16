use engine::storage::Store as CoreStore;
pub use engine::storage::StoreError;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
    core: CoreStore,
}

impl std::ops::Deref for Store {
    type Target = CoreStore;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl Store {
    pub fn new(core: CoreStore) -> Self {
        Self {
            pool: core.pool().clone(),
            core,
        }
    }

    pub fn core(&self) -> &CoreStore {
        &self.core
    }

    pub async fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let store = Self::new(CoreStore::open(path).await?);
        store.recover_deliveries().await?;
        Ok(store)
    }

    pub(crate) async fn recover_deliveries(&self) -> Result<(), StoreError> {
        sqlx::query("UPDATE deliveries SET state='outcome_unknown' WHERE state='dispatching'")
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        self.redact_pairing_codes().await
    }

    pub async fn bind(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        instance: &str,
        thread: &str,
    ) -> Result<(), StoreError> {
        if chat <= 0
            || !self
                .pairing()
                .is_paired(bot, chat as u64, chat)
                .await
                .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::NotPaired);
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
mod tests {
    use super::*;
    use session::operation::OperationState;

    #[tokio::test]
    async fn a_second_writer_is_rejected_and_an_input_cannot_change_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        assert!(matches!(
            Store::open(dir.path()).await,
            Err(StoreError::Locked)
        ));
        store
            .reserve_operation("same-message", "host", "original")
            .await
            .unwrap();
        assert!(matches!(
            store
                .reserve_operation("same-message", "host", "other")
                .await,
            Err(StoreError::SourceConflict)
        ));
    }

    #[tokio::test]
    async fn duplicate_inputs_reuse_operations_and_restart_preserves_uncertainty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let id = store
            .reserve_operation("message:1", "host", "thread")
            .await
            .unwrap();
        assert_eq!(
            store
                .reserve_operation("message:1", "host", "thread")
                .await
                .unwrap(),
            id
        );
        store
            .transition(&id, OperationState::Received, OperationState::Prepared)
            .await
            .unwrap();
        store
            .transition(&id, OperationState::Prepared, OperationState::Dispatching)
            .await
            .unwrap();
        store.pool.close().await;
        drop(store);
        let recovered = Store::open(dir.path()).await.unwrap();
        assert_eq!(
            recovered.operation_state(&id).await.unwrap(),
            OperationState::OutcomeUnknown
        );
        assert!(
            recovered
                .transition(
                    &id,
                    OperationState::OutcomeUnknown,
                    OperationState::Dispatching
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn private_containers_are_bot_scoped_and_never_retarget_within_an_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for bot in [1, 2] {
            let code = store
                .pairing()
                .request_pairing(bot, 100, 100, true, 1)
                .await
                .unwrap();
            store.pairing().approve_pairing(&code, 2).await.unwrap();
        }
        store.bind(1, 100, 1, "host", "original").await.unwrap();
        store.bind(2, 100, 1, "host", "different").await.unwrap();
        assert!(matches!(
            store.bind(2, 100, 1, "host", "retargeted").await,
            Err(StoreError::ContainerConflict)
        ));
        assert!(matches!(
            store.bind(1, -100, 1, "host", "original").await,
            Err(StoreError::NotPaired)
        ));
    }
}
