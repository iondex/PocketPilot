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
    #[error(
        "platform resource creation outcome is unknown; inspect platform resources before retrying"
    )]
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
        crate::storage::initialize_schema(&pool)
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
        Ok(store)
    }

    pub async fn check_instance(
        &self,
        id: &str,
        hash: &str,
        register: bool,
    ) -> Result<(), StoreError> {
        if register {
            sqlx::query(
                "INSERT INTO instances(id,endpoint_hash) VALUES(?,?) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(hash)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        }
        let saved: Option<String> =
            sqlx::query_scalar("SELECT endpoint_hash FROM instances WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| StoreError::Database)?;
        if saved.as_deref().is_some_and(|saved| saved != hash) {
            return Err(StoreError::InstanceConflict);
        }
        Ok(())
    }

    pub async fn accept_receipt(
        &self,
        id: &str,
        receipt: &session::provider::Receipt,
    ) -> Result<(), StoreError> {
        let receipt = serde_json::to_string(receipt).map_err(|_| StoreError::Database)?;
        let changed=sqlx::query("UPDATE operations SET receipt=?,state='accepted',updated_at=unixepoch() WHERE id=? AND state='dispatching'")
            .bind(receipt)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?.rows_affected();
        if changed != 1 {
            return Err(StoreError::Transition);
        }
        Ok(())
    }

    pub async fn receipt(
        &self,
        id: &str,
    ) -> Result<Option<session::provider::Receipt>, StoreError> {
        let text: Option<String> = sqlx::query_scalar("SELECT receipt FROM operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        text.map(|text| serde_json::from_str(&text).map_err(|_| StoreError::Database))
            .transpose()
    }

    pub async fn probe_threads(
        &self,
        instance: &str,
    ) -> Result<std::collections::HashSet<String>, StoreError> {
        let receipts=sqlx::query("SELECT receipt FROM operations WHERE instance_id=? AND source_key LIKE 'probe:%:create' AND state='accepted'")
            .bind(instance).fetch_all(&self.pool).await.map_err(|_|StoreError::Database)?;
        let mut threads = std::collections::HashSet::new();
        for row in receipts {
            let Some(raw) = row.get::<Option<String>, _>("receipt") else {
                continue;
            };
            let receipt: session::provider::Receipt =
                serde_json::from_str(&raw).map_err(|_| StoreError::Database)?;
            threads.insert(receipt.thread);
        }
        Ok(threads)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
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

/// Prototype initialization is idempotent and never recovers/replays operations.
pub async fn initialize_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(include_str!("../../../schema.sql"))
        .execute(pool)
        .await
        .inspect_err(|error| {
            tracing::error!(
                database_code = error
                    .as_database_error()
                    .and_then(|e| e.code())
                    .as_deref()
                    .unwrap_or("unavailable"),
                "prototype schema initialization failed"
            );
        })?;
    Ok(())
}
