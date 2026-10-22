//! PocketPilot-owned SQLite state and process-level directory ownership.
//!
//! Cloned stores share the file lock and connection pool; the state directory
//! stays exclusively owned until the last store is dropped. This module never
//! opens a native runtime database or performs provider I/O.
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, sync::Arc, time::Duration};

#[path = "operation_store.rs"]
mod operation_store;

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
    #[error("an authorized private chat is required")]
    Unauthorized,
}

impl ::i18n::LocalizedError for StoreError {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::InstanceConflict => ::i18n::Message::new("error.store.instance_conflict"),
            Self::Database => ::i18n::Message::new("error.store.database"),
            Self::Transition => ::i18n::Message::new("error.store.transition"),
            Self::ContainerConflict => ::i18n::Message::new("error.store.container_conflict"),
            Self::Locked => ::i18n::Message::new("error.store.locked"),
            Self::SourceConflict => ::i18n::Message::new("error.store.source_conflict"),
            Self::ProvisionUnknown => ::i18n::Message::new("error.store.provision_unknown"),
            Self::Unauthorized => ::i18n::Message::new("error.store.unauthorized"),
        }
    }
}

/// Shared access to bridge state while retaining exclusive directory ownership.
#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
    _lock: Arc<std::fs::File>,
}

impl Store {
    /// Open or initialize the state directory and recover uncertain dispatches.
    ///
    /// Existing records are retained. Recovery only marks interrupted writes as
    /// uncertain; it does not contact a provider or replay an operation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Locked`] if another process owns the directory, or
    /// [`StoreError::Database`] for filesystem, schema or SQLite failures.
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

    /// Check an endpoint fingerprint, optionally registering a previously unseen ID.
    ///
    /// A saved ID cannot be rebound to another endpoint: persisted session and
    /// operation references would otherwise refer to a different runtime.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InstanceConflict`] on an identity mismatch and
    /// [`StoreError::Database`] if the ledger cannot be read or written.
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

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
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
