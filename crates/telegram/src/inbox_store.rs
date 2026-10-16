//! Durable polling cursor and input claims.
use super::*;

impl Store {
    /// Persist the whole fetched batch and its confirmation offset in one transaction.
    pub async fn ingest(
        &self,
        bot: i64,
        epoch: i64,
        updates: &[(u32, Option<Input>)],
        now: i64,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        for (id, input) in updates {
            if let Some(input) = input {
                let payload = serde_json::to_string(input).map_err(|_| Error::Database)?;
                sqlx::query("INSERT INTO inbound_updates(bot_id,polling_epoch,update_id,event_key,payload,received_at,chat_id,topic_id) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING")
                    .bind(bot).bind(epoch).bind(i64::from(*id)).bind(&input.event_key).bind(payload).bind(now).bind(input.chat).bind(input.topic)
                    .execute(&mut *tx).await.map_err(|_| Error::Database)?;
            }
        }
        if let Some((id, _)) = updates.last() {
            let next = i64::from(*id) + 1;
            sqlx::query("UPDATE polling_cursors SET next_offset=?,last_poll_at=? WHERE bot_id=? AND polling_epoch=?")
                .bind(next).bind(now).bind(bot).bind(epoch).execute(&mut *tx).await.map_err(|_| Error::Database)?;
        } else {
            sqlx::query(
                "UPDATE polling_cursors SET last_poll_at=? WHERE bot_id=? AND polling_epoch=?",
            )
            .bind(now)
            .bind(bot)
            .bind(epoch)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        }
        tx.commit().await.map_err(|_| Error::Database)
    }

    pub async fn cursor(&self, bot: i64, now: i64) -> Result<(i64, Option<i32>), Error> {
        sqlx::query("INSERT INTO polling_cursors(bot_id,polling_epoch,generation_lower_bound,last_poll_at) VALUES(?,1,?,?) ON CONFLICT DO NOTHING")
            .bind(bot).bind(now).bind(now).execute(&self.pool).await.map_err(|_| Error::Database)?;
        // Telegram may randomize update IDs after a week without updates. Reset only after
        // a complete week with no received event; event_key still deduplicates old messages.
        let row = sqlx::query("SELECT polling_epoch,next_offset,generation_lower_bound FROM polling_cursors WHERE bot_id=?")
            .bind(bot).fetch_one(&self.pool).await.map_err(|_| Error::Database)?;
        let mut epoch: i64 = row.get("polling_epoch");
        let last: Option<i64> =
            sqlx::query_scalar("SELECT MAX(received_at) FROM inbound_updates WHERE bot_id=?")
                .bind(bot)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| Error::Database)?;
        let since = last.unwrap_or(0).max(row.get("generation_lower_bound"));
        let offset: Option<i64> = row.get("next_offset");
        if now.saturating_sub(since) > 7 * 24 * 3600 && offset.is_some() {
            epoch += 1;
            sqlx::query("UPDATE polling_cursors SET polling_epoch=?,next_offset=NULL,generation_lower_bound=? WHERE bot_id=?")
                .bind(epoch).bind(now).bind(bot).execute(&self.pool).await.map_err(|_| Error::Database)?;
            return Ok((epoch, None));
        }
        let offset = offset
            .map(i32::try_from)
            .transpose()
            .map_err(|_| Error::Database)?;
        Ok((epoch, offset))
    }

    pub async fn claim_input(&self, bot: i64) -> Result<Option<Inbox>, Error> {
        self.claim_available_input(bot, &[]).await
    }

    pub async fn claim_available_input(
        &self,
        bot: i64,
        busy: &[(i64, i32)],
    ) -> Result<Option<Inbox>, Error> {
        let mut query = sqlx::QueryBuilder::new(
            "UPDATE inbound_updates SET processed=2 WHERE rowid=(SELECT rowid FROM inbound_updates WHERE bot_id=",
        );
        query.push_bind(bot).push(" AND processed=0");
        for (chat, topic) in busy {
            query
                .push(" AND NOT (chat_id=")
                .push_bind(chat)
                .push(" AND topic_id=")
                .push_bind(topic)
                .push(")");
        }
        query.push(
            " ORDER BY received_at,update_id LIMIT 1) RETURNING polling_epoch,update_id,payload",
        );
        let row = query
            .build()
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        row.map(|row| {
            Ok(Inbox {
                epoch: row.get("polling_epoch"),
                id: row.get("update_id"),
                input: serde_json::from_str(row.get("payload")).map_err(|_| Error::Database)?,
            })
        })
        .transpose()
    }

    pub async fn finish_input(&self, bot: i64, epoch: i64, id: i64) -> Result<(), Error> {
        sqlx::query("UPDATE inbound_updates SET processed=1 WHERE bot_id=? AND polling_epoch=? AND update_id=?")
            .bind(bot).bind(epoch).bind(id).execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }

    pub async fn recover_inbox(&self, bot: i64) -> Result<(), Error> {
        sqlx::query("UPDATE management_topics SET state='outcome_unknown' WHERE bot_id=? AND state='dispatching'")
            .bind(bot).execute(&self.pool).await.map_err(|_|Error::Database)?;
        // Old queued status updates may still contain the previous name/status
        // coupling. Do not replay those renames after an upgrade. Old keyboard
        // nonces are invalidated below, so their unsent menus must be refreshed.
        let queued = sqlx::query(
            "SELECT id,source_key,payload FROM deliveries WHERE bot_id=? AND state='pending'",
        )
        .bind(bot)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        for row in queued {
            let mut body: Outbound =
                serde_json::from_str(row.get("payload")).map_err(|_| Error::Database)?;
            let id: String = row.get("id");
            if !body.buttons.is_empty() {
                sqlx::query(
                    "UPDATE deliveries SET state='rejected' WHERE id=? AND state='pending'",
                )
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(|_| Error::Database)?;
            } else if row.get::<String, _>("source_key").starts_with("status:")
                && body.topic_title.take().is_some()
            {
                sqlx::query("UPDATE deliveries SET payload=? WHERE id=? AND state='pending'")
                    .bind(serde_json::to_string(&body).map_err(|_| Error::Database)?)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(|_| Error::Database)?;
            }
        }
        // A process restart does not authorize still-pending agent input. Keep
        // its original timestamp/text and require a new interaction to send it.
        let pending=sqlx::query("SELECT polling_epoch,update_id,payload FROM inbound_updates WHERE bot_id=? AND processed=0")
            .bind(bot).fetch_all(&self.pool).await.map_err(|_|Error::Database)?;
        for row in pending {
            let mut input: Input =
                serde_json::from_str(row.get("payload")).map_err(|_| Error::Database)?;
            input.requires_confirmation = true;
            sqlx::query("UPDATE inbound_updates SET payload=? WHERE bot_id=? AND polling_epoch=? AND update_id=? AND processed=0")
                .bind(serde_json::to_string(&input).map_err(|_|Error::Database)?).bind(bot)
                .bind(row.get::<i64,_>("polling_epoch")).bind(row.get::<i64,_>("update_id"))
                .execute(&self.pool).await.map_err(|_|Error::Database)?;
        }
        // Callback queries have no click timestamp. Invalidating the previous
        // process's nonces prevents queued old clicks from authorizing writes.
        sqlx::query("UPDATE callbacks SET consumed=1 WHERE bot_id=? AND consumed=0")
            .bind(bot)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        let rows = sqlx::query("SELECT polling_epoch,update_id,payload FROM inbound_updates WHERE bot_id=? AND processed=2")
            .bind(bot).fetch_all(&self.pool).await.map_err(|_| Error::Database)?;
        for row in rows {
            let input: Input =
                serde_json::from_str(row.get("payload")).map_err(|_| Error::Database)?;
            self.enqueue(
                bot,
                input.chat,
                input.topic,
                &format!("recovered:{}", input.event_key),
                &Outbound::text(
                    "上次请求的处理结果未确认。没有自动重发；请先 /status 核对，必要时重新发送。",
                ),
            )
            .await?;
            self.finish_input(bot, row.get("polling_epoch"), row.get("update_id"))
                .await?;
        }
        sqlx::query("UPDATE topic_requests SET state='outcome_unknown' WHERE state='dispatching'")
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        Ok(())
    }
}
