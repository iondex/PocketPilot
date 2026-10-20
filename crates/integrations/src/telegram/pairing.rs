use crate::telegram::storage::Store;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pairing state could not be accessed")]
    Database,
    #[error("pairing is available only in a private chat")]
    PrivateOnly,
    #[error("user is already paired")]
    AlreadyPaired,
    #[error("pairing code is invalid, expired or already used")]
    InvalidCode,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Database => ::i18n::Message::new("error.pairing.database"),
            Self::PrivateOnly => ::i18n::Message::new("error.pairing.private_only"),
            Self::AlreadyPaired => ::i18n::Message::new("error.pairing.already_paired"),
            Self::InvalidCode => ::i18n::Message::new("error.pairing.invalid_code"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Pending {
    pub bot_id: i64,
    pub user_id: i64,
    pub chat_id: i64,
    pub expires_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Paired {
    pub bot_id: i64,
    pub user_id: i64,
    pub chat_id: i64,
    pub paired_at: i64,
}

fn hash(code: &str) -> Result<String, Error> {
    let code = uuid::Uuid::parse_str(code).map_err(|_| Error::InvalidCode)?;
    Ok(format!("{:x}", Sha256::digest(code.as_bytes())))
}

#[derive(Clone)]
pub struct Registry {
    pool: SqlitePool,
}

impl Store {
    pub fn pairing(&self) -> Registry {
        Registry {
            pool: self.pool.clone(),
        }
    }
}

impl Registry {
    /// Admin CLI access: never takes the service lock or recovers execution rows.
    pub async fn open(directory: &Path) -> Result<Self, Error> {
        let path = directory.join("state.db");
        if !path.exists() {
            let store = Store::open(directory).await.map_err(|_| Error::Database)?;
            return Ok(store.pairing());
        }
        if !directory.join("pocketpilot.lock").exists() {
            return Err(Error::Database);
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| Error::Database)?;
        engine::storage::initialize_schema(&pool)
            .await
            .map_err(|_| Error::Database)?;
        Ok(Self { pool })
    }

    /// Called by the future private-message handler. No Telegram message is sent here.
    pub async fn request_pairing(
        &self,
        bot: i64,
        user: u64,
        chat: i64,
        private: bool,
        now: i64,
    ) -> Result<String, Error> {
        if bot <= 0
            || crate::telegram::access(user, chat, private, false)
                == crate::telegram::Access::Denied
        {
            return Err(Error::PrivateOnly);
        }
        if self.is_paired(bot, user, chat).await? {
            return Err(Error::AlreadyPaired);
        }
        let code = uuid::Uuid::new_v4().to_string();
        let digest = hash(&code)?;
        let expires = now.checked_add(600).ok_or(Error::InvalidCode)?;
        sqlx::query("INSERT INTO pairing_requests(code_hash,bot_id,user_id,chat_id,expires_at) VALUES(?,?,?,?,?) ON CONFLICT(bot_id,user_id) DO UPDATE SET code_hash=excluded.code_hash,chat_id=excluded.chat_id,expires_at=excluded.expires_at")
            .bind(digest).bind(bot).bind(chat).bind(chat).bind(expires).execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(code)
    }

    pub async fn is_paired(&self, bot: i64, user: u64, chat: i64) -> Result<bool, Error> {
        if crate::telegram::access(user, chat, true, false) == crate::telegram::Access::Denied {
            return Ok(false);
        }
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pairings WHERE bot_id=? AND user_id=? AND chat_id=?)",
        )
        .bind(bot)
        .bind(chat)
        .bind(chat)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        Ok(found)
    }

    pub async fn approve_pairing(&self, code: &str, now: i64) -> Result<Paired, Error> {
        let digest = hash(code)?;
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let row=sqlx::query("SELECT bot_id,user_id,chat_id FROM pairing_requests WHERE code_hash=? AND expires_at>?")
            .bind(&digest).bind(now).fetch_optional(&mut *tx).await.map_err(|_| Error::Database)?.ok_or(Error::InvalidCode)?;
        let paired = Paired {
            bot_id: row.get("bot_id"),
            user_id: row.get("user_id"),
            chat_id: row.get("chat_id"),
            paired_at: now,
        };
        sqlx::query("INSERT INTO pairings(bot_id,user_id,chat_id,paired_at) VALUES(?,?,?,?) ON CONFLICT(bot_id,user_id) DO UPDATE SET chat_id=excluded.chat_id,paired_at=excluded.paired_at")
            .bind(paired.bot_id).bind(paired.user_id).bind(paired.chat_id).bind(now).execute(&mut *tx).await.map_err(|_| Error::Database)?;
        sqlx::query("DELETE FROM pairing_requests WHERE code_hash=?")
            .bind(digest)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)?;
        Ok(paired)
    }

    pub async fn pending_pairings(&self, now: i64) -> Result<Vec<Pending>, Error> {
        let rows=sqlx::query("SELECT bot_id,user_id,chat_id,expires_at FROM pairing_requests WHERE expires_at>? ORDER BY expires_at")
            .bind(now).fetch_all(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(rows
            .into_iter()
            .map(|r| Pending {
                bot_id: r.get("bot_id"),
                user_id: r.get("user_id"),
                chat_id: r.get("chat_id"),
                expires_at: r.get("expires_at"),
            })
            .collect())
    }

    pub async fn paired_users(&self) -> Result<Vec<Paired>, Error> {
        let rows = sqlx::query(
            "SELECT bot_id,user_id,chat_id,paired_at FROM pairings ORDER BY bot_id,user_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        Ok(rows
            .into_iter()
            .map(|r| Paired {
                bot_id: r.get("bot_id"),
                user_id: r.get("user_id"),
                chat_id: r.get("chat_id"),
                paired_at: r.get("paired_at"),
            })
            .collect())
    }

    pub async fn revoke_pairing(&self, bot: i64, user: u64) -> Result<(), Error> {
        let user = i64::try_from(user).map_err(|_| Error::PrivateOnly)?;
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        sqlx::query("DELETE FROM pairings WHERE bot_id=? AND user_id=?")
            .bind(bot)
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        sqlx::query("DELETE FROM pairing_requests WHERE bot_id=? AND user_id=?")
            .bind(bot)
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        sqlx::query(
            "UPDATE bindings SET active=0,revision=revision+1 WHERE bot_id=? AND chat_id=?",
        )
        .bind(bot)
        .bind(user)
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cli_approval_works_while_service_owns_the_ledger_without_recovering_operations() {
        let dir = tempfile::tempdir().unwrap();
        let service = Store::open(dir.path()).await.unwrap();
        let id = service
            .reserve_operation("live-operation", "host", "thread")
            .await
            .unwrap();
        use session::operation::OperationState as State;
        service
            .transition(&id, State::Received, State::Prepared)
            .await
            .unwrap();
        service
            .transition(&id, State::Prepared, State::Dispatching)
            .await
            .unwrap();
        let code = service
            .pairing()
            .request_pairing(1, 10, 10, true, 100)
            .await
            .unwrap();
        let cli = Registry::open(dir.path()).await.unwrap();
        cli.approve_pairing(&code, 101).await.unwrap();
        assert!(service.pairing().is_paired(1, 10, 10).await.unwrap());
        assert_eq!(
            service.operation_state(&id).await.unwrap(),
            State::Dispatching
        );
    }

    #[tokio::test]
    async fn only_cli_approval_grants_access_and_codes_are_one_time_and_bot_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        assert!(matches!(
            store
                .pairing()
                .request_pairing(1, 10, -10, false, 100)
                .await,
            Err(Error::PrivateOnly)
        ));
        let code = store
            .pairing()
            .request_pairing(1, 10, 10, true, 100)
            .await
            .unwrap();
        assert!(!store.pairing().is_paired(1, 10, 10).await.unwrap());
        store.pairing().approve_pairing(&code, 101).await.unwrap();
        assert!(store.pairing().is_paired(1, 10, 10).await.unwrap());
        assert!(!store.pairing().is_paired(2, 10, 10).await.unwrap());
        assert!(matches!(
            store.pairing().approve_pairing(&code, 102).await,
            Err(Error::InvalidCode)
        ));
        store.pairing().revoke_pairing(1, 10).await.unwrap();
        assert!(!store.pairing().is_paired(1, 10, 10).await.unwrap());
    }

    #[tokio::test]
    async fn expired_codes_cannot_be_approved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let code = store
            .pairing()
            .request_pairing(1, 10, 10, true, 100)
            .await
            .unwrap();
        assert!(matches!(
            store.pairing().approve_pairing(&code, 700).await,
            Err(Error::InvalidCode)
        ));
    }
}
