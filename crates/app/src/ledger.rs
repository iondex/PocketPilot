//! Durable Telegram inbox, bindings and outbox. No remote request is issued here.
use crate::storage::{Store, StoreError as Error};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use telegram::api::{Button, Input};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub workspace: String,
    pub cwd: String,
    pub instance: String,
    pub endpoint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outbound {
    pub text: String,
    #[serde(default)]
    pub buttons: Vec<Vec<Button>>,
    #[serde(default = "yes")]
    pub require_pairing: bool,
    pub edit_message: Option<i32>,
    #[serde(default)]
    pub config_hash: Option<String>,
    #[serde(default)]
    pub topic_title: Option<String>,
    #[serde(default)]
    pub binding_revision: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub scope: Option<Scope>,
}
fn yes() -> bool {
    true
}
impl Outbound {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            buttons: vec![],
            require_pairing: true,
            edit_message: None,
            config_hash: None,
            topic_title: None,
            binding_revision: None,
            expires_at: None,
            scope: None,
        }
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Binding {
    pub bot_id: i64,
    pub chat_id: i64,
    pub topic_id: i32,
    pub instance_id: String,
    pub thread_id: String,
    pub workspace_id: Option<String>,
    pub revision: i64,
    pub display_name: Option<String>,
    pub status_message_id: Option<i32>,
    pub status_text: Option<String>,
    pub topic_title: Option<String>,
    pub observed_items: String,
    pub history_ready: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Delivery {
    pub id: String,
    pub chat_id: i64,
    pub topic_id: i32,
    pub payload: String,
    pub source_key: String,
}

#[derive(Debug)]
pub struct Inbox {
    pub epoch: i64,
    pub id: i64,
    pub input: Input,
}

pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

impl Store {
    pub(crate) async fn redact_pairing_codes(&self) -> Result<(), Error> {
        let rows=sqlx::query("SELECT id,payload FROM deliveries WHERE state IN ('delivered','rejected','outcome_unknown')")
            .fetch_all(&self.pool).await.map_err(|_|Error::Database)?;
        for row in rows {
            if let Ok(mut body) = serde_json::from_str::<Outbound>(row.get("payload"))
                && !body.require_pairing
                && body.expires_at.is_some()
                && body.text != "配对申请已投递或失效。"
            {
                body.text = "配对申请已投递或失效。".into();
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
    pub async fn check_instance(&self, id: &str, hash: &str, register: bool) -> Result<(), Error> {
        if register {
            sqlx::query(
                "INSERT INTO instances(id,endpoint_hash) VALUES(?,?) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(hash)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        }
        let saved: Option<String> =
            sqlx::query_scalar("SELECT endpoint_hash FROM instances WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| Error::Database)?;
        if saved.as_deref().is_some_and(|saved| saved != hash) {
            return Err(Error::InstanceConflict);
        }
        Ok(())
    }
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
    pub async fn enqueue(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        source: &str,
        body: &Outbound,
    ) -> Result<(), Error> {
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(bot).bind(chat).bind(topic).bind(source).bind(payload)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn pending_deliveries(&self, bot: i64, now: i64) -> Result<Vec<Delivery>, Error> {
        sqlx::query_as("SELECT id,chat_id,topic_id,payload,source_key FROM deliveries d WHERE bot_id=? AND state='pending' AND not_before<=? AND NOT EXISTS(SELECT 1 FROM deliveries earlier WHERE earlier.bot_id=d.bot_id AND earlier.chat_id=d.chat_id AND earlier.state IN ('pending','dispatching') AND earlier.rowid<d.rowid) ORDER BY rowid LIMIT 100")
            .bind(bot).bind(now).fetch_all(&self.pool).await.map_err(|_| Error::Database)
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
                    body.text = "配对申请已投递或失效。".into();
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
    pub async fn bindings(&self, bot: i64) -> Result<Vec<Binding>, Error> {
        sqlx::query_as("SELECT * FROM bindings WHERE bot_id=? AND active=1")
            .bind(bot)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| Error::Database)
    }
    pub async fn binding(&self, bot: i64, chat: i64, topic: i32) -> Result<Option<Binding>, Error> {
        sqlx::query_as(
            "SELECT * FROM bindings WHERE bot_id=? AND chat_id=? AND topic_id=? AND active=1",
        )
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| Error::Database)
    }
    pub async fn set_workspace(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
        workspace: &str,
    ) -> Result<(), Error> {
        sqlx::query(
            "UPDATE bindings SET workspace_id=? WHERE bot_id=? AND chat_id=? AND topic_id=?",
        )
        .bind(workspace)
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .execute(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn unbind(&self, bot: i64, chat: i64, topic: i32) -> Result<(), Error> {
        sqlx::query("UPDATE bindings SET active=0,revision=revision+1 WHERE bot_id=? AND chat_id=? AND topic_id=?")
            .bind(bot).bind(chat).bind(topic).execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn disable_chat(&self, bot: i64, chat: i64) -> Result<(), Error> {
        sqlx::query("UPDATE bindings SET active=0,revision=revision+1 WHERE bot_id=? AND chat_id=? AND active=1")
            .bind(bot).bind(chat).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }
    pub async fn unavailable_topic(&self, bot: i64, chat: i64, topic: i32) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        sqlx::query("UPDATE bindings SET active=0,revision=revision+1 WHERE bot_id=? AND chat_id=? AND topic_id=?")
            .bind(bot).bind(chat).bind(topic).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        sqlx::query("UPDATE topic_requests SET state='rejected',topic_id=NULL WHERE bot_id=? AND chat_id=? AND topic_id=? AND state='accepted'")
            .bind(bot).bind(chat).bind(topic).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        sqlx::query("UPDATE management_topics SET state='rejected',topic_id=NULL WHERE bot_id=? AND chat_id=? AND topic_id=? AND state='accepted'")
            .bind(bot).bind(chat).bind(topic).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        sqlx::query("UPDATE deliveries SET state='rejected' WHERE bot_id=? AND chat_id=? AND topic_id=? AND state='pending'")
            .bind(bot).bind(chat).bind(topic).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)
    }
    pub async fn rename(&self, bot: i64, chat: i64, topic: i32, title: &str) -> Result<(), Error> {
        sqlx::query(
            "UPDATE bindings SET display_name=? WHERE bot_id=? AND chat_id=? AND topic_id=?",
        )
        .bind(title)
        .bind(bot)
        .bind(chat)
        .bind(topic)
        .execute(&self.pool)
        .await
        .map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn observe(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
    ) -> Result<(), Error> {
        let items = serde_json::to_string(items).map_err(|_| Error::Database)?;
        sqlx::query("UPDATE bindings SET observed_items=?,history_ready=1 WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(items).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn publish_observation(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
        source: &str,
        body: &Outbound,
    ) -> Result<(), Error> {
        let items = serde_json::to_string(items).map_err(|_| Error::Database)?;
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let n=sqlx::query("UPDATE bindings SET observed_items=?,history_ready=1 WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(items).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision)
            .execute(&mut *tx).await.map_err(|_| Error::Database)?.rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(source).bind(payload)
            .execute(&mut *tx).await.map_err(|_| Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)
    }
    pub async fn publish_batch(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
        mut body: Outbound,
        members: usize,
        config: &crate::config::Delivery,
        now_ms: i64,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let prior=sqlx::query("SELECT id,payload,batch_opened_ms,batch_members FROM deliveries WHERE bot_id=? AND chat_id=? AND topic_id=? AND state='pending' AND batch_opened_ms IS NOT NULL ORDER BY rowid DESC LIMIT 1")
            .bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).fetch_optional(&mut *tx).await.map_err(|_|Error::Database)?;
        let mut existing = None;
        let mut opened = now_ms;
        let mut count = members;
        if let Some(prior) = prior {
            let previous: Outbound =
                serde_json::from_str(prior.get("payload")).map_err(|_| Error::Database)?;
            let at: i64 = prior.get("batch_opened_ms");
            if previous.config_hash == body.config_hash
                && previous.scope == body.scope
                && previous.binding_revision == body.binding_revision
                && now_ms.saturating_sub(at) < config.max_flush_delay_ms.min(i64::MAX as u64) as i64
            {
                opened = at;
                count = count.saturating_add(prior.get::<i64, _>("batch_members") as usize);
                body.text = format!("{}\n\n────────\n\n{}", previous.text, body.text);
                existing = Some(prior.get::<String, _>("id"));
            }
        }
        let delay = if count >= config.burst_threshold {
            config.max_flush_delay_ms
        } else {
            config.burst_window_ms
        };
        let due_ms = opened.saturating_add(delay.min(i64::MAX as u64) as i64);
        let due = due_ms.saturating_add(999) / 1000;
        let payload = serde_json::to_string(&body).map_err(|_| Error::Database)?;
        if let Some(id) = existing {
            sqlx::query("UPDATE deliveries SET payload=?,batch_members=?,not_before=? WHERE id=? AND state='pending'")
                .bind(payload).bind(count as i64).bind(due).bind(id).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        } else {
            let id = uuid::Uuid::now_v7().to_string();
            sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state,not_before,batch_opened_ms,batch_members) VALUES(?,?,?,?,?,?,'pending',?,?,?)")
                .bind(&id).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(format!("live:{id}")).bind(payload).bind(due).bind(opened).bind(count as i64)
                .execute(&mut *tx).await.map_err(|_|Error::Database)?;
        }
        let items = serde_json::to_string(items).map_err(|_| Error::Database)?;
        let n=sqlx::query("UPDATE bindings SET observed_items=?,history_ready=1 WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(items).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision)
            .execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        tx.commit().await.map_err(|_| Error::Database)
    }
    pub async fn status_card(
        &self,
        binding: &Binding,
        message: i32,
        text: &str,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE bindings SET status_message_id=?,status_text=? WHERE bot_id=? AND chat_id=? AND topic_id=? AND active=1")
            .bind(message).bind(text).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn topic_title_receipt(&self, binding: &Binding, title: &str) -> Result<(), Error> {
        sqlx::query("UPDATE bindings SET topic_title=? WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(title).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision)
            .execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }
    pub async fn callback(
        &self,
        bot: i64,
        input: &Input,
        config_hash: &str,
        body: &str,
        now: i64,
    ) -> Result<String, Error> {
        let token = uuid::Uuid::new_v4().simple().to_string();
        sqlx::query("INSERT INTO callbacks(token,bot_id,user_id,chat_id,topic_id,config_hash,payload,expires_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&token).bind(bot).bind(input.user as i64).bind(input.chat).bind(input.topic).bind(config_hash).bind(body).bind(now+600)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(token)
    }
    pub async fn consume_callback(
        &self,
        bot: i64,
        input: &Input,
        hash: &str,
        now: i64,
    ) -> Result<Option<String>, Error> {
        sqlx::query_scalar("UPDATE callbacks SET consumed=1 WHERE token=? AND bot_id=? AND user_id=? AND chat_id=? AND topic_id=? AND config_hash=? AND expires_at>? AND consumed=0 RETURNING payload")
            .bind(input.callback.as_deref()).bind(bot).bind(input.user as i64).bind(input.chat).bind(input.topic).bind(hash).bind(now)
            .fetch_optional(&self.pool).await.map_err(|_| Error::Database)
    }
    pub async fn callback_live(
        &self,
        bot: i64,
        input: &Input,
        hash: &str,
        now: i64,
    ) -> Result<bool, Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM callbacks WHERE token=? AND bot_id=? AND user_id=? AND chat_id=? AND topic_id=? AND config_hash=? AND expires_at>? AND consumed=1)")
            .bind(input.callback.as_deref()).bind(bot).bind(input.user as i64).bind(input.chat).bind(input.topic).bind(hash).bind(now)
            .fetch_one(&self.pool).await.map_err(|_|Error::Database)
    }
    pub async fn dispatch_authorized(
        &self,
        id: &str,
        bot: i64,
        input: &Input,
        binding: Option<&Binding>,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let paired: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pairings WHERE bot_id=? AND user_id=? AND chat_id=?)",
        )
        .bind(bot)
        .bind(input.user as i64)
        .bind(input.chat)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Error::Database)?;
        if !paired {
            return Err(Error::NotPaired);
        }
        if let Some(binding) = binding {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bindings WHERE bot_id=? AND chat_id=? AND topic_id=? AND instance_id=? AND thread_id=? AND revision=? AND active=1)")
                .bind(bot).bind(input.chat).bind(input.topic).bind(&binding.instance_id).bind(&binding.thread_id).bind(binding.revision)
                .fetch_one(&mut *tx).await.map_err(|_|Error::Database)?;
            if !active {
                return Err(Error::ContainerConflict);
            }
        }
        let n=sqlx::query("UPDATE operations SET state='dispatching',updated_at=unixepoch() WHERE id=? AND state='prepared'")
            .bind(id).execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        tx.commit().await.map_err(|_| Error::Database)
    }
    pub async fn begin_topic(
        &self,
        bot: i64,
        chat: i64,
        instance: &str,
        thread: &str,
    ) -> Result<bool, Error> {
        let n=sqlx::query("INSERT INTO topic_requests(bot_id,chat_id,instance_id,thread_id,state) VALUES(?,?,?,?,'dispatching') ON CONFLICT(bot_id,chat_id,instance_id,thread_id) DO UPDATE SET state='dispatching' WHERE topic_requests.state='rejected'")
            .bind(bot).bind(chat).bind(instance).bind(thread).execute(&self.pool).await.map_err(|_| Error::Database)?.rows_affected();
        Ok(n == 1)
    }
    pub async fn end_topic(
        &self,
        bot: i64,
        chat: i64,
        instance: &str,
        thread: &str,
        state: &str,
        topic: Option<i32>,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE topic_requests SET state=?,topic_id=? WHERE bot_id=? AND chat_id=? AND instance_id=? AND thread_id=?")
            .bind(state).bind(topic).bind(bot).bind(chat).bind(instance).bind(thread).execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
    pub async fn topic(
        &self,
        bot: i64,
        chat: i64,
        instance: &str,
        thread: &str,
    ) -> Result<Option<i32>, Error> {
        sqlx::query_scalar("SELECT topic_id FROM topic_requests WHERE bot_id=? AND chat_id=? AND instance_id=? AND thread_id=? AND state='accepted'")
            .bind(bot).bind(chat).bind(instance).bind(thread).fetch_optional(&self.pool).await.map_err(|_| Error::Database)
    }
    pub async fn accept_receipt(
        &self,
        id: &str,
        receipt: &session::provider::Receipt,
    ) -> Result<(), Error> {
        let receipt = serde_json::to_string(receipt).map_err(|_| Error::Database)?;
        let changed=sqlx::query("UPDATE operations SET receipt=?,state='accepted',updated_at=unixepoch() WHERE id=? AND state='dispatching'")
            .bind(receipt)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?.rows_affected();
        if changed != 1 {
            return Err(Error::Transition);
        }
        Ok(())
    }
    pub async fn enqueue_status(&self, binding: &Binding, body: &Outbound) -> Result<(), Error> {
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO UPDATE SET payload=excluded.payload,state='pending' WHERE deliveries.state IN ('pending','delivered','rejected') AND deliveries.payload<>excluded.payload")
            .bind(uuid::Uuid::now_v7().to_string()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id)
            .bind(format!("status:{}",binding.thread_id)).bind(payload).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }
    pub async fn receipt(&self, id: &str) -> Result<Option<session::provider::Receipt>, Error> {
        let text: Option<String> = sqlx::query_scalar("SELECT receipt FROM operations WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        text.map(|text| serde_json::from_str(&text).map_err(|_| Error::Database))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn terminal_pairing_delivery_drops_the_plaintext_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let mut body = Outbound::text("fixture-once-only-code");
        body.require_pairing = false;
        body.expires_at = Some(600);
        store.enqueue(1, 10, 0, "pairing", &body).await.unwrap();
        let rows = store.pending_deliveries(1, 1).await.unwrap();
        let id = &rows[0].id;
        store
            .delivery_state(id, "pending", "dispatching", None, 0)
            .await
            .unwrap();
        store
            .delivery_state(id, "dispatching", "delivered", Some(1), 0)
            .await
            .unwrap();
        let payload: String = sqlx::query_scalar("SELECT payload FROM deliveries WHERE id=?")
            .bind(id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(!payload.contains("fixture-once-only-code"));
    }
    async fn setup() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let code = store
            .pairing()
            .request_pairing(1, 10, 10, true, 100)
            .await
            .unwrap();
        store.pairing().approve_pairing(&code, 101).await.unwrap();
        (dir, store)
    }
    fn input(id: u32) -> Input {
        Input {
            update_id: id,
            event_key: format!("message:10:{id}"),
            user: 10,
            chat: 10,
            topic: 11,
            date: 100,
            text: "hello".into(),
            callback: None,
            callback_id: None,
            message_id: id as i32,
            config_hash: None,
            requires_confirmation: false,
        }
    }
    #[tokio::test]
    async fn a_busy_topic_preserves_its_input_order_without_blocking_other_topics() {
        let (_dir, store) = setup().await;
        store.cursor(1, 100).await.unwrap();
        let mut other = input(3);
        other.topic = 12;
        store
            .ingest(
                1,
                1,
                &[(1, Some(input(1))), (2, Some(input(2))), (3, Some(other))],
                100,
            )
            .await
            .unwrap();
        let first = store.claim_available_input(1, &[]).await.unwrap().unwrap();
        assert_eq!(first.id, 1);
        let independent = store
            .claim_available_input(1, &[(10, 11)])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(independent.id, 3);
        assert!(
            store
                .claim_available_input(1, &[(10, 11)])
                .await
                .unwrap()
                .is_none()
        );
        store.finish_input(1, first.epoch, first.id).await.unwrap();
        assert_eq!(
            store
                .claim_available_input(1, &[])
                .await
                .unwrap()
                .unwrap()
                .id,
            2
        );
    }
    #[tokio::test]
    async fn inbox_confirmation_is_durable_and_message_identity_survives_polling_epochs() {
        let (_dir, store) = setup().await;
        let (epoch, offset) = store.cursor(1, 100).await.unwrap();
        assert_eq!(offset, None);
        let first = input(100);
        store
            .ingest(1, epoch, &[(100, Some(first.clone())), (101, None)], 100)
            .await
            .unwrap();
        assert_eq!(store.cursor(1, 101).await.unwrap(), (epoch, Some(102)));
        let received = store.claim_input(1).await.unwrap().unwrap();
        assert_eq!(received.input.event_key, first.event_key);
        store
            .finish_input(1, received.epoch, received.id)
            .await
            .unwrap();
        let (next, offset) = store.cursor(1, 100 + 8 * 24 * 3600).await.unwrap();
        assert_eq!(next, epoch + 1);
        assert_eq!(offset, None);
        store
            .ingest(1, next, &[(100, Some(first))], 100 + 8 * 24 * 3600)
            .await
            .unwrap();
        assert!(store.claim_input(1).await.unwrap().is_none());
        assert_eq!(
            store.cursor(1, 100 + 8 * 24 * 3600 + 1).await.unwrap().0,
            next
        );
    }
    #[tokio::test]
    async fn interrupted_input_is_reported_without_replaying_it_and_unknown_topic_is_not_recreated()
    {
        let (_dir, store) = setup().await;
        store.cursor(1, 100).await.unwrap();
        store
            .ingest(1, 1, &[(1, Some(input(1)))], 100)
            .await
            .unwrap();
        store.claim_input(1).await.unwrap().unwrap();
        assert!(store.begin_topic(1, 10, "host", "thread").await.unwrap());
        store.recover_inbox(1).await.unwrap();
        assert!(store.claim_input(1).await.unwrap().is_none());
        assert_eq!(store.pending_deliveries(1, 101).await.unwrap().len(), 1);
        assert!(!store.begin_topic(1, 10, "host", "thread").await.unwrap());
        store
            .end_topic(1, 10, "host", "thread", "rejected", None)
            .await
            .unwrap();
        assert!(store.begin_topic(1, 10, "host", "thread").await.unwrap());
    }
    #[tokio::test]
    async fn callbacks_are_single_use_private_peer_and_snapshot_bound() {
        let (_dir, store) = setup().await;
        let mut event = input(1);
        let token = store
            .callback(1, &event, "version1", "action", 100)
            .await
            .unwrap();
        event.callback = Some(token);
        assert!(
            store
                .consume_callback(2, &event, "version1", 101)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .consume_callback(1, &event, "version2", 101)
                .await
                .unwrap()
                .is_none()
        );
        event.topic = 12;
        assert!(
            store
                .consume_callback(1, &event, "version1", 101)
                .await
                .unwrap()
                .is_none()
        );
        event.topic = 11;
        assert_eq!(
            store
                .consume_callback(1, &event, "version1", 101)
                .await
                .unwrap()
                .as_deref(),
            Some("action")
        );
        assert!(
            store
                .consume_callback(1, &event, "version1", 101)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn history_and_observation_cursor_publish_atomically_and_never_after_unbind() {
        let (_dir, store) = setup().await;
        store.bind(1, 10, 11, "host", "thread").await.unwrap();
        let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
        let items = BTreeMap::from([("item".into(), "hash".into())]);
        store
            .publish_observation(&binding, &items, "history", &Outbound::text("full history"))
            .await
            .unwrap();
        let current = store.binding(1, 10, 11).await.unwrap().unwrap();
        assert!(current.history_ready);
        assert_eq!(
            serde_json::from_str::<BTreeMap<String, String>>(&current.observed_items).unwrap(),
            items
        );
        store.unbind(1, 10, 11).await.unwrap();
        assert!(
            store
                .publish_observation(
                    &binding,
                    &items,
                    "later",
                    &Outbound::text("should not leak")
                )
                .await
                .is_err()
        );
        assert_eq!(store.pending_deliveries(1, 100).await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn burst_members_share_one_durable_batch_with_a_fixed_deadline() {
        let (dir, store) = setup().await;
        store.bind(1, 10, 11, "host", "thread").await.unwrap();
        let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
        let mut items = BTreeMap::new();
        let config = crate::config::Delivery::default();
        for (at, text) in [
            (100000, "one"),
            (101000, "two"),
            (101500, "three"),
            (104000, "four"),
        ] {
            items.insert(text.into(), text.into());
            store
                .publish_batch(&binding, &items, Outbound::text(text), 1, &config, at)
                .await
                .unwrap();
        }
        assert!(store.pending_deliveries(1, 104).await.unwrap().is_empty());
        store.pool.close().await;
        drop(store);
        let restarted = Store::open(dir.path()).await.unwrap();
        let rows = restarted.pending_deliveries(1, 105).await.unwrap();
        assert_eq!(rows.len(), 1);
        let body: Outbound = serde_json::from_str(&rows[0].payload).unwrap();
        for text in ["one", "two", "three", "four"] {
            assert!(body.text.contains(text));
        }
    }
    #[tokio::test]
    async fn revoked_pairing_fails_at_the_dispatch_transaction() {
        let (_dir, store) = setup().await;
        store.bind(1, 10, 11, "host", "thread").await.unwrap();
        let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
        let id = store
            .reserve_operation("pending", "host", "thread")
            .await
            .unwrap();
        store
            .transition(
                &id,
                session::operation::OperationState::Received,
                session::operation::OperationState::Prepared,
            )
            .await
            .unwrap();
        store.pairing().revoke_pairing(1, 10).await.unwrap();
        assert!(matches!(
            store
                .dispatch_authorized(&id, 1, &input(1), Some(&binding))
                .await,
            Err(Error::NotPaired)
        ));
        assert_eq!(
            store.operation_state(&id).await.unwrap(),
            session::operation::OperationState::Prepared
        );
    }
    #[tokio::test]
    async fn an_uncertain_delivery_is_not_automatically_sent_again() {
        let (_dir, store) = setup().await;
        store
            .enqueue(1, 10, 11, "once", &Outbound::text("body"))
            .await
            .unwrap();
        let row = store
            .pending_deliveries(1, 100)
            .await
            .unwrap()
            .pop()
            .unwrap();
        store
            .delivery_state(&row.id, "pending", "dispatching", None, 0)
            .await
            .unwrap();
        store
            .delivery_state(&row.id, "dispatching", "outcome_unknown", None, 0)
            .await
            .unwrap();
        store
            .enqueue(1, 10, 11, "once", &Outbound::text("body"))
            .await
            .unwrap();
        assert!(store.pending_deliveries(1, 10000).await.unwrap().is_empty());
    }
}
