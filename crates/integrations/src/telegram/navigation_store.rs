//! Navigation snapshots, publication receipts and atomic consumption.
use super::Menu;
use crate::telegram::{api::Input, storage::Store};
use engine::storage::StoreError;
use sqlx::Row;

pub(super) struct SavedMenu {
    pub(super) id: String,
    pub(super) menu: Menu,
    pub(super) after: i32,
    pub(super) message: Option<i32>,
    pub(super) published: Option<i64>,
    pub(super) consumed: bool,
}

impl Store {
    pub(super) async fn navigation(
        &self,
        bot: i64,
        input: &Input,
    ) -> Result<Option<SavedMenu>, StoreError> {
        let row = sqlx::query(
            "SELECT id, payload, after_message_id, message_id, published_at, consumed
             FROM navigation
             WHERE bot_id=? AND chat_id=? AND topic_id=? AND user_id=?",
        )
        .bind(bot)
        .bind(input.chat)
        .bind(input.topic)
        .bind(input.user as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        row.map(|row| {
            Ok(SavedMenu {
                id: row.get("id"),
                menu: serde_json::from_str(row.get("payload")).map_err(|_| StoreError::Database)?,
                after: row.get("after_message_id"),
                message: row.get("message_id"),
                published: row.get("published_at"),
                consumed: row.get("consumed"),
            })
        })
        .transpose()
    }

    pub(super) async fn save_navigation(
        &self,
        bot: i64,
        input: &Input,
        id: &str,
        menu: &Menu,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_string(menu).map_err(|_| StoreError::Database)?;
        sqlx::query(
            "INSERT INTO navigation(bot_id, chat_id, topic_id, user_id, id, payload, after_message_id)
             VALUES(?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(bot_id, chat_id, topic_id) DO UPDATE SET
                 user_id=excluded.user_id, id=excluded.id, payload=excluded.payload,
                 after_message_id=excluded.after_message_id, published_at=NULL, consumed=0",
        )
        .bind(bot)
        .bind(input.chat)
        .bind(input.topic)
        .bind(input.user as i64)
        .bind(id)
        .bind(payload)
        .bind(input.message_id)
        .execute(&self.pool)
        .await
        .map_err(|_| StoreError::Database)?;
        Ok(())
    }

    pub async fn navigation_receipt(
        &self,
        id: &str,
        message: i32,
        time: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE navigation SET message_id=?,published_at=? WHERE id=?")
            .bind(message)
            .bind(time)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        Ok(())
    }

    pub(crate) async fn navigation_current(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        id: &str,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM navigation
             WHERE bot_id=? AND chat_id=? AND topic_id=? AND id=?)",
        )
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| StoreError::Database)
    }

    pub(super) async fn navigation_pending(&self, bot: i64, id: &str) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM deliveries
             WHERE bot_id=? AND state IN ('pending', 'dispatching')
             AND json_extract(payload, '$.navigation_id')=?)",
        )
        .bind(bot)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| StoreError::Database)
    }

    pub(super) async fn claim_navigation(&self, id: &str) -> Result<bool, StoreError> {
        Ok(
            sqlx::query("UPDATE navigation SET consumed=1 WHERE id=? AND consumed=0")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|_| StoreError::Database)?
                .rows_affected()
                == 1,
        )
    }
}
