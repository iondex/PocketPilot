//! Retain observed provider errors across bridge restarts, separate from native history.
use crate::telegram::storage::{Store, StoreError};
use session::{History, TextItem};

impl Store {
    pub(crate) async fn retain_errors(
        &self,
        instance: &str,
        thread: &str,
        mut history: History,
    ) -> Result<History, StoreError> {
        history = history.without_transient_errors();
        let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
        for item in history.items.iter().filter(|i| i.is_error()) {
            let payload = serde_json::to_string(item).map_err(|_| StoreError::Database)?;
            sqlx::query(
                "INSERT INTO session_errors(instance_id,thread_id,turn_id,item_id,\
            payload) VALUES(?,?,?,?,?) ON CONFLICT(instance_id,thread_id,turn_id,\
            item_id) DO UPDATE SET payload=excluded.payload WHERE \
            session_errors.payload<>excluded.payload",
            )
            .bind(instance)
            .bind(thread)
            .bind(&item.turn_id)
            .bind(&item.item_id)
            .bind(payload)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
        }
        sqlx::query(
            "DELETE FROM session_errors WHERE instance_id=? AND thread_id=? AND \
            rowid NOT IN (SELECT rowid FROM session_errors WHERE instance_id=? AND \
            thread_id=? ORDER BY observed_at DESC,rowid DESC LIMIT 256)",
        )
        .bind(instance)
        .bind(thread)
        .bind(instance)
        .bind(thread)
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Database)?;
        let saved: Vec<String> = sqlx::query_scalar(
            "SELECT payload FROM session_errors WHERE instance_id=? AND thread_id=? \
            ORDER BY observed_at,rowid",
        )
        .bind(instance)
        .bind(thread)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| StoreError::Database)?;
        tx.commit().await.map_err(|_| StoreError::Database)?;
        let mut orphaned = Vec::new();
        for payload in saved {
            let error: TextItem =
                serde_json::from_str(&payload).map_err(|_| StoreError::Database)?;
            if !error.is_error() {
                continue;
            }
            if history.items.iter().any(|i| {
                i.is_error()
                    && i.turn_id == error.turn_id
                    && (i.item_id == error.item_id
                        || (i.text == error.text && i.phase == error.phase))
            }) {
                continue;
            }
            // A partial history must not invent an ordering for a missing turn.
            if let Some(index) = history
                .items
                .iter()
                .rposition(|i| i.turn_id == error.turn_id)
            {
                history.items.insert(index + 1, error);
            } else if history.all_pages_read {
                orphaned.push(error);
            }
        }
        history.items.splice(0..0, orphaned);
        Ok(history)
    }
}
