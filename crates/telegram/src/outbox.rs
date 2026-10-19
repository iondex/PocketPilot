//! Durable outbound queue and delivery receipts.
use super::*;

impl Store {
    pub(crate) async fn redact_pairing_codes(&self) -> Result<(), Error> {
        let rows=sqlx::query("SELECT id,payload FROM deliveries WHERE state IN ('delivered','rejected','outcome_unknown')")
            .fetch_all(&self.pool).await.map_err(|_|Error::Database)?;
        for row in rows {
            if let Ok(mut body) = serde_json::from_str::<Outbound>(row.get("payload"))
                && !body.require_pairing
                && body.expires_at.is_some()
                && body.text != "[pairing code redacted]"
            {
                body.text = "[pairing code redacted]".into();
                body.buttons.clear();
                sqlx::query("UPDATE deliveries SET payload=? WHERE id=?")
                    .bind(serde_json::to_string(&body).map_err(|_| Error::Database)?)
                    .bind(row.get::<String, _>("id"))
                    .execute(&self.pool)
                    .await
                    .map_err(|_| Error::Database)?;
            }
        }
        Ok(())
    }

    pub async fn enqueue(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        source: &str,
        body: &Outbound,
    ) -> Result<(), Error> {
        let mut body = body.clone();
        if let Some(id) = body.text_id.take() {
            let locale = self
                .locale(bot, chat as u64)
                .await?
                .unwrap_or_else(|| "zh-CN".into());
            body.text = ::i18n::text(&locale, &id);
        }
        let payload = serde_json::to_string(&body).map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(bot).bind(chat).bind(topic).bind(source).bind(payload)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }

    pub async fn pending_deliveries(&self, bot: i64, now: i64) -> Result<Vec<Delivery>, Error> {
        sqlx::query_as("SELECT id,chat_id,topic_id,payload,source_key,next_part,rendered_parts FROM deliveries d WHERE bot_id=? AND state='pending' AND not_before<=? AND NOT EXISTS(SELECT 1 FROM deliveries earlier WHERE earlier.bot_id=d.bot_id AND earlier.chat_id=d.chat_id AND earlier.state IN ('pending','dispatching') AND earlier.rowid<d.rowid AND NOT (earlier.state='pending' AND earlier.not_before>? AND json_extract(earlier.payload,'$.appearance') IS NOT NULL AND json_extract(earlier.payload,'$.icon_lookup_deferred')=1)) ORDER BY rowid LIMIT 100")
            .bind(bot).bind(now).bind(now).fetch_all(&self.pool).await.map_err(|_| Error::Database)
    }

    pub async fn prepare_parts(&self, row: &Delivery, parts: &[Outbound]) -> Result<bool, Error> {
        Ok(sqlx::query("UPDATE deliveries SET rendered_parts=?,batch_opened_ms=NULL WHERE id=? AND state='pending' AND next_part=0 AND rendered_parts IS NULL AND payload=?")
            .bind(serde_json::to_string(parts).map_err(|_| Error::Database)?).bind(&row.id).bind(&row.payload)
            .execute(&self.pool).await.map_err(|_| Error::Database)?.rows_affected() == 1)
    }

    pub async fn acknowledge_part(
        &self,
        row: &Delivery,
        message: Option<i32>,
        more: bool,
    ) -> Result<(), Error> {
        let changed = sqlx::query("UPDATE deliveries SET next_part=next_part+1,state=?,message_id=?,rendered_parts=CASE WHEN ? THEN rendered_parts ELSE NULL END WHERE id=? AND state='dispatching' AND next_part=?")
            .bind(if more { "pending" } else { "delivered" }).bind(message).bind(more).bind(&row.id).bind(row.next_part)
            .execute(&self.pool).await.map_err(|_| Error::Database)?.rows_affected();
        if changed != 1 {
            return Err(Error::Transition);
        }
        if !more {
            self.redact_pairing_codes().await?;
        }
        Ok(())
    }

    pub async fn delivery_state(
        &self,
        id: &str,
        from: &str,
        to: &str,
        message: Option<i32>,
        not_before: i64,
    ) -> Result<(), Error> {
        if !matches!(
            (from, to),
            ("pending", "dispatching" | "rejected")
                | (
                    "dispatching",
                    "delivered" | "rejected" | "outcome_unknown" | "pending"
                )
        ) {
            return Err(Error::Transition);
        }
        let n = sqlx::query(
            "UPDATE deliveries SET state=?,message_id=?,not_before=? WHERE id=? AND state=?",
        )
        .bind(to)
        .bind(message)
        .bind(not_before)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await
        .map_err(|_| Error::Database)?
        .rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        // The delivery queue briefly needs the pairing code to send it. Remove
        // that plaintext once terminal; the pairing registry retains only a hash.
        if matches!(to, "delivered" | "rejected" | "outcome_unknown") {
            let payload: Option<String> =
                sqlx::query_scalar("SELECT payload FROM deliveries WHERE id=?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|_| Error::Database)?;
            if let Some(payload) = payload {
                let mut body: Outbound =
                    serde_json::from_str(&payload).map_err(|_| Error::Database)?;
                if !body.require_pairing && body.expires_at.is_some() {
                    body.text = "[pairing code redacted]".into();
                    body.buttons.clear();
                    let payload = serde_json::to_string(&body).map_err(|_| Error::Database)?;
                    sqlx::query("UPDATE deliveries SET payload=? WHERE id=? AND state=?")
                        .bind(payload)
                        .bind(id)
                        .bind(to)
                        .execute(&self.pool)
                        .await
                        .map_err(|_| Error::Database)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn defer_icon_lookup(
        &self,
        id: &str,
        body: &Outbound,
        not_before: i64,
    ) -> Result<(), Error> {
        let mut deferred = body.clone();
        deferred.icon_lookup_deferred = true;
        let payload = serde_json::to_string(&deferred).map_err(|_| Error::Database)?;
        let updated = sqlx::query("UPDATE deliveries SET state='pending',payload=?,not_before=? WHERE id=? AND state='dispatching'")
            .bind(payload).bind(not_before).bind(id).execute(&self.pool).await.map_err(|_| Error::Database)?.rows_affected();
        if updated != 1 {
            return Err(Error::Transition);
        }
        Ok(())
    }
}
