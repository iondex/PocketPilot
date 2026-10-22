//! Durable sessions and input receipts, independent of any bridge connection.
use session::{
    History, Page, RuntimeStatus, Session, SessionQuery, TextItem, provider::Receipt,
    server::Failure,
};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(directory: &Path) -> Result<Self, Failure> {
        private_directory(directory)?;
        let path = directory.join("runtime.db");
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|_| Failure::Internal)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await
            .map_err(|_| Failure::Internal)?;
        sqlx::raw_sql(include_str!("../../../schema.sql"))
            .execute(&pool)
            .await
            .map_err(|_| Failure::Internal)?;
        sqlx::query("UPDATE runtime_operations SET state='unknown' WHERE state='dispatching'")
            .execute(&pool)
            .await
            .map_err(|_| Failure::Internal)?;
        // A server restart is not evidence that a previous native task finished.
        sqlx::query(
            "UPDATE runtime_sessions SET payload=json_set(payload,'$.status',\
            json('{\"type\":\"notLoaded\"}'),'$.active_turn',NULL)",
        )
        .execute(&pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(Self { pool })
    }

    pub async fn create(&self, session: &Session) -> Result<(), Failure> {
        sqlx::query("INSERT INTO runtime_sessions(id,payload) VALUES(?,?)")
            .bind(&session.reference.thread)
            .bind(serde_json::to_string(session).map_err(|_| Failure::Internal)?)
            .execute(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn status(
        &self,
        id: &str,
        status: RuntimeStatus,
        turn: Option<&str>,
    ) -> Result<(), Failure> {
        sqlx::query(
            "UPDATE runtime_sessions SET payload=json_set(payload,'$.status',json(?),\
            '$.active_turn',?,'$.updated_at',unixepoch()) WHERE id=?",
        )
        .bind(serde_json::to_string(&status).map_err(|_| Failure::Internal)?)
        .bind(turn)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn title(&self, id: &str, title: &str) -> Result<(), Failure> {
        sqlx::query(
            "UPDATE runtime_sessions SET payload=json_set(payload,'$.title',?,\
            '$.native_title',?) WHERE id=? AND json_extract(payload,\
            '$.native_title') IS NULL",
        )
        .bind(title)
        .bind(title)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn started(&self, id: &str) -> Result<bool, Failure> {
        sqlx::query_scalar("SELECT native_started FROM runtime_sessions WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?
            .ok_or(Failure::NotFound)
    }

    pub async fn mark_started(&self, id: &str) -> Result<(), Failure> {
        sqlx::query("UPDATE runtime_sessions SET native_started=1 WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn read(&self, id: &str) -> Result<Session, Failure> {
        let value: String = sqlx::query_scalar("SELECT payload FROM runtime_sessions WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?
            .ok_or(Failure::NotFound)?;
        serde_json::from_str(&value).map_err(|_| Failure::Internal)
    }

    pub async fn list(&self, query: SessionQuery) -> Result<Page<Session>, Failure> {
        if query.archived {
            return Ok(Page {
                data: vec![],
                next_cursor: None,
            });
        }
        let offset: u32 = query
            .cursor
            .as_deref()
            .unwrap_or("0")
            .parse()
            .map_err(|_| Failure::Invalid)?;
        let limit = query.limit.clamp(1, 100);
        let rows = sqlx::query(
            "SELECT payload FROM runtime_sessions WHERE (? IS NULL OR \
            json_extract(payload,'$.cwd')=?) ORDER BY rowid DESC LIMIT ? OFFSET ?",
        )
        .bind(&query.cwd)
        .bind(&query.cwd)
        .bind(limit + 1)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        let next_cursor =
            (rows.len() > limit as usize).then(|| offset.saturating_add(limit).to_string());
        let data = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| serde_json::from_str(row.get("payload")).map_err(|_| Failure::Internal))
            .collect::<Result<_, _>>()?;
        Ok(Page { data, next_cursor })
    }

    pub async fn item(&self, id: &str, item: TextItem) -> Result<(), Failure> {
        sqlx::query(
            "INSERT INTO runtime_items(session,key,payload,ordinal) VALUES(?,?,?,\
            (SELECT COALESCE(MAX(ordinal),0)+1 FROM runtime_items WHERE session=?)) \
            ON CONFLICT(session,key) DO UPDATE SET payload=excluded.payload",
        )
        .bind(id)
        .bind(format!("{}:{}", item.turn_id, item.item_id))
        .bind(serde_json::to_string(&item).map_err(|_| Failure::Internal)?)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    /// Promote the last main assistant message only when the native result ends the turn.
    pub async fn finish(&self, id: &str, turn: &str, phase: &str) -> Result<(), Failure> {
        sqlx::query(
            "UPDATE runtime_items SET payload=json_set(payload,'$.phase',?) WHERE \
            session=? AND key=(SELECT key FROM runtime_items WHERE session=? AND \
            json_extract(payload,'$.turn_id')=? AND json_extract(payload,\
            '$.role')='agent' ORDER BY ordinal DESC LIMIT 1)",
        )
        .bind(phase)
        .bind(id)
        .bind(id)
        .bind(turn)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn history(&self, id: &str) -> Result<History, Failure> {
        self.read(id).await?;
        let mut tx = self.pool.begin().await.map_err(|_| Failure::Internal)?;
        let bounds = sqlx::query(
            "SELECT count(*) AS items,COALESCE(sum(length(CAST(payload AS BLOB))),0) \
            AS bytes FROM runtime_items WHERE session=?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Failure::Internal)?;
        if bounds.get::<i64, _>("items") > 10000 || bounds.get::<i64, _>("bytes") > 32 * 1024 * 1024
        {
            return Err(Failure::Unsupported);
        }
        let rows = sqlx::query(
            "SELECT payload FROM runtime_items WHERE session=? ORDER BY ordinal LIMIT 10001",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| Failure::Internal)?;
        tx.commit().await.map_err(|_| Failure::Internal)?;
        if rows.len() > 10000 {
            return Err(Failure::Unsupported);
        }
        let mut bytes = 0usize;
        let items = rows
            .into_iter()
            .map(|row| {
                let body: &str = row.get("payload");
                bytes = bytes.saturating_add(body.len());
                if bytes > 32 * 1024 * 1024 {
                    return Err(Failure::Unsupported);
                }
                serde_json::from_str(body).map_err(|_| Failure::Internal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(History {
            items,
            all_pages_read: true,
        })
    }

    pub async fn claim(&self, id: &str, digest: &str) -> Result<Option<Receipt>, Failure> {
        let changed = sqlx::query(
            "INSERT INTO runtime_operations(id,digest,state) VALUES(?,?,\
            'dispatching') ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(digest)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?
        .rows_affected();
        if changed == 1 {
            return Ok(None);
        }
        let row = sqlx::query("SELECT digest,state,receipt FROM runtime_operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?;
        if row.get::<String, _>("digest") != digest {
            return Err(Failure::Conflict);
        }
        if row.get::<String, _>("state") != "accepted" {
            return Err(Failure::Unknown);
        }
        let value: String = row.get("receipt");
        serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| Failure::Internal)
    }

    pub async fn accept(&self, id: &str, receipt: &Receipt) -> Result<(), Failure> {
        sqlx::query(
            "UPDATE runtime_operations SET state='accepted',receipt=? WHERE id=? AND \
            state='dispatching'",
        )
        .bind(serde_json::to_string(receipt).map_err(|_| Failure::Internal)?)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|_| Failure::Internal)?;
        Ok(())
    }

    pub async fn own_file(&self, thread: &str, path: &str) -> Result<(), Failure> {
        sqlx::query("INSERT INTO runtime_uploads(path,session) VALUES(?,?) ON CONFLICT DO NOTHING")
            .bind(path)
            .bind(thread)
            .execute(&self.pool)
            .await
            .map_err(|_| Failure::Internal)?;
        self.check_file(thread, path).await
    }

    pub async fn check_file(&self, thread: &str, path: &str) -> Result<(), Failure> {
        let owner: Option<String> =
            sqlx::query_scalar("SELECT session FROM runtime_uploads WHERE path=?")
                .bind(path)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| Failure::Internal)?;
        if owner.as_deref() != Some(thread) {
            return Err(Failure::Unauthorized);
        }
        Ok(())
    }
}

pub fn private_directory(path: &Path) -> Result<(), Failure> {
    std::fs::create_dir_all(path).map_err(|_| Failure::Internal)?;
    if std::fs::symlink_metadata(path)
        .map_err(|_| Failure::Internal)?
        .file_type()
        .is_symlink()
    {
        return Err(Failure::Unauthorized);
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Failure::Internal)?;
    Ok(())
}
