//! PocketPilot-owned state only; never opens a Codex database.
use session::operation::OperationState;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, sync::Arc, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("provider ID points at a different execution endpoint; assign a new provider ID")]
    InstanceConflict,
    #[error("local state database could not be accessed")]
    Database,
    #[error("invalid operation state transition")]
    Transition,
    #[error("topic was previously associated with another session")]
    ContainerConflict,
    #[error("another PocketPilot process owns this state directory")]
    Locked,
    #[error("input identity was already assigned to a different session")]
    SourceConflict,
    #[error("platform resource creation outcome is unknown; inspect Discord before retrying")]
    ProvisionUnknown,
    #[error("a paired private chat is required")]
    NotPaired,
}

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
    _lock: Arc<std::fs::File>,
}

impl Store {
    pub async fn open(directory: &Path) -> Result<Self, StoreError> {
        let exists = directory.exists();
        std::fs::create_dir_all(directory).map_err(|_| StoreError::Database)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !exists {
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| StoreError::Database)?;
            }
        }
        let path = directory.join("state.db");
        let lock_path = directory.join("pocketpilot.lock");
        if path.exists() && !lock_path.exists() {
            return Err(StoreError::Database);
        }
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| StoreError::Database)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| StoreError::Locked)?;
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let store = Self::connect(options, lock).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| StoreError::Database)?;
            for suffix in ["-wal", "-shm"] {
                let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
                if sidecar.exists() {
                    std::fs::set_permissions(sidecar, std::fs::Permissions::from_mode(0o600))
                        .map_err(|_| StoreError::Database)?;
                }
            }
        }
        Ok(store)
    }

    async fn connect(
        options: SqliteConnectOptions,
        lock: std::fs::File,
    ) -> Result<Self, StoreError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| StoreError::Database)?;
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .map_err(|_| StoreError::Database)?;
        let store = Self {
            pool,
            _lock: Arc::new(lock),
        };
        // Process recovery changes only our ledger, never issues remote operations.
        sqlx::query("UPDATE operations SET state='outcome_unknown' WHERE state='dispatching'")
            .execute(&store.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        sqlx::query("UPDATE deliveries SET state='outcome_unknown' WHERE state='dispatching'")
            .execute(&store.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        store.redact_pairing_codes().await?;
        Ok(store)
    }

    pub async fn reserve_operation(
        &self,
        source_key: &str,
        instance: &str,
        thread: &str,
    ) -> Result<String, StoreError> {
        let id = uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO operations(id,source_key,instance_id,thread_id,state) VALUES(?,?,?,?,'received') ON CONFLICT(source_key) DO NOTHING")
            .bind(&id).bind(source_key).bind(instance).bind(thread).execute(&self.pool).await.map_err(|_| StoreError::Database)?;
        let row = sqlx::query("SELECT id,instance_id,thread_id FROM operations WHERE source_key=?")
            .bind(source_key)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        if row.get::<String, _>("instance_id") != instance
            || row.get::<String, _>("thread_id") != thread
        {
            return Err(StoreError::SourceConflict);
        }
        Ok(row.get("id"))
    }

    pub async fn transition(
        &self,
        id: &str,
        from: OperationState,
        to: OperationState,
    ) -> Result<(), StoreError> {
        if !from.can_transition(to) {
            return Err(StoreError::Transition);
        }
        let count = sqlx::query(
            "UPDATE operations SET state=?,updated_at=unixepoch() WHERE id=? AND state=?",
        )
        .bind(state_name(to))
        .bind(id)
        .bind(state_name(from))
        .execute(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?
        .rows_affected();
        if count != 1 {
            return Err(StoreError::Transition);
        }
        Ok(())
    }

    pub async fn operation_state(&self, id: &str) -> Result<OperationState, StoreError> {
        let name: String = sqlx::query_scalar("SELECT state FROM operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        parse_state(&name)
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

fn state_name(state: OperationState) -> &'static str {
    match state {
        OperationState::Received => "received",
        OperationState::Prepared => "prepared",
        OperationState::Dispatching => "dispatching",
        OperationState::Accepted => "accepted",
        OperationState::Rejected => "rejected",
        OperationState::OutcomeUnknown => "outcome_unknown",
    }
}
fn parse_state(value: &str) -> Result<OperationState, StoreError> {
    match value {
        "received" => Ok(OperationState::Received),
        "prepared" => Ok(OperationState::Prepared),
        "dispatching" => Ok(OperationState::Dispatching),
        "accepted" => Ok(OperationState::Accepted),
        "rejected" => Ok(OperationState::Rejected),
        "outcome_unknown" => Ok(OperationState::OutcomeUnknown),
        _ => Err(StoreError::Database),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
