//! Durable Telegram inbox, bindings and outbox. No remote request is issued here.
use crate::api::{Button, Input};
use crate::storage::{Store, StoreError as Error};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;

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
    pub text_id: Option<String>,
    #[serde(default)]
    pub agent_markdown: bool,
    #[serde(default)]
    pub document: bool,
    #[serde(default)]
    pub appearance: Option<crate::presentation::Appearance>,
    #[serde(default)]
    pub icon_lookup_deferred: bool,
    #[serde(default)]
    pub remove_status: Option<i32>,
    #[serde(default)]
    pub buttons: Vec<Vec<Button>>,
    #[serde(default = "default_pairing_required")]
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
    #[serde(default)]
    pub markdown: bool,
    #[serde(default)]
    pub navigation_id: Option<String>,
}
// Missing legacy fields must never disable private-chat authorization.
fn default_pairing_required() -> bool {
    true
}

impl Outbound {
    pub fn ui(id: impl Into<String>) -> Self {
        let mut body = Self::text("");
        body.text_id = Some(id.into());
        body
    }

    pub fn agent(text: impl Into<String>) -> Self {
        let mut body = Self::text(text);
        body.agent_markdown = true;
        body
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            text_id: None,
            agent_markdown: false,
            document: false,
            appearance: None,
            icon_lookup_deferred: false,
            remove_status: None,
            buttons: vec![],
            require_pairing: true,
            edit_message: None,
            config_hash: None,
            topic_title: None,
            binding_revision: None,
            expires_at: None,
            scope: None,
            markdown: false,
            navigation_id: None,
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

#[path = "inbox_store.rs"]
mod inbox_store;

#[path = "outbox.rs"]
mod outbox;

impl Store {
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
        self.publish_cursor(binding, items, source, body, true)
            .await
    }

    pub(crate) async fn publish_notice(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
        source: &str,
        body: &Outbound,
    ) -> Result<(), Error> {
        self.publish_cursor(binding, items, source, body, binding.history_ready)
            .await
    }

    async fn publish_cursor(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
        source: &str,
        body: &Outbound,
        history_ready: bool,
    ) -> Result<(), Error> {
        let items = serde_json::to_string(items).map_err(|_| Error::Database)?;
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let n=sqlx::query("UPDATE bindings SET observed_items=?,history_ready=? WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(items).bind(history_ready).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision)
            .execute(&mut *tx).await.map_err(|_| Error::Database)?.rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        if !body.text.is_empty() {
            sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(source).bind(payload)
            .execute(&mut *tx).await.map_err(|_| Error::Database)?;
        }
        tx.commit().await.map_err(|_| Error::Database)
    }

    pub async fn publish_batch(
        &self,
        binding: &Binding,
        items: &BTreeMap<String, String>,
        mut body: Outbound,
        members: usize,
        config: &engine::config::Delivery,
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
                body.text = format!("{}\n\n{}", previous.text, body.text);
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

    pub async fn appearance_receipt(
        &self,
        binding: &Binding,
        appearance: &crate::presentation::Appearance,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE bindings SET topic_title=?,status_text=?,status_message_id=NULL WHERE bot_id=? AND chat_id=? AND topic_id=? AND revision=? AND active=1")
            .bind(&appearance.title).bind(appearance.receipt()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(binding.revision).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
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

    pub async fn enqueue_appearance(
        &self,
        binding: &Binding,
        body: &Outbound,
    ) -> Result<(), Error> {
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO UPDATE SET payload=CASE WHEN deliveries.state='pending' AND json_extract(deliveries.payload,'$.icon_lookup_deferred')=1 THEN json_set(excluded.payload,'$.icon_lookup_deferred',json('true')) ELSE excluded.payload END,state='pending' WHERE deliveries.state<>'dispatching' AND deliveries.payload<>excluded.payload")
            .bind(uuid::Uuid::now_v7().to_string()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id).bind(format!("appearance:v2:{}:{}",binding.thread_id,binding.revision)).bind(payload).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }

    pub async fn enqueue_status(&self, binding: &Binding, body: &Outbound) -> Result<(), Error> {
        let payload = serde_json::to_string(body).map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO deliveries(id,bot_id,chat_id,topic_id,source_key,payload,state) VALUES(?,?,?,?,?,?,'pending') ON CONFLICT(bot_id,chat_id,topic_id,source_key) DO UPDATE SET payload=excluded.payload,state='pending' WHERE deliveries.state IN ('pending','delivered','rejected') AND deliveries.payload<>excluded.payload")
            .bind(uuid::Uuid::now_v7().to_string()).bind(binding.bot_id).bind(binding.chat_id).bind(binding.topic_id)
            .bind(format!("status:{}",binding.thread_id)).bind(payload).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
