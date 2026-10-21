//! Platform-local SQL journals inbox claims, callback nonces and frozen native deliveries.
use super::{
    Error, Result,
    config::Config,
    model::{Action, Binding, Destination, Input},
    render::Card,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Store {
    pub core: engine::storage::Store,
    pub namespace: String,
}

#[derive(sqlx::FromRow)]
pub struct Inbox {
    pub id: String,
    pub payload: String,
    pub config_hash: String,
}

#[derive(sqlx::FromRow)]
pub struct Outbox {
    pub id: String,
    pub destination: String,
    pub edit: Option<String>,
    pub binding: Option<String>,
    pub revision: Option<i64>,
    pub content: String,
    pub nonces: String,
    pub automatic: bool,
    pub config_hash: String,
    pub created: i64,
}

#[derive(Serialize, Deserialize)]
pub struct SavedAction {
    pub action: Action,
    pub hash: String,
}

impl Store {
    pub fn new(core: engine::storage::Store, config: &Config) -> Self {
        Self {
            core,
            namespace: config.namespace(),
        }
    }

    pub async fn recover(&self) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        sqlx::query("UPDATE feishu_inbox SET state=CASE WHEN state='processing' THEN 'unknown' ELSE 'stale' END WHERE namespace=? AND state IN ('pending','processing')").bind(&self.namespace).execute(&mut *tx).await?;
        sqlx::query("UPDATE feishu_outbox SET state='unknown',content='' WHERE namespace=? AND state='dispatching'").bind(&self.namespace).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE feishu_bindings SET state='unknown' WHERE namespace=? AND state='provisioning'",
        )
        .bind(&self.namespace)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE feishu_actions SET consumed=1 WHERE namespace=?")
            .bind(&self.namespace)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn enqueue(&self, input: &Input, hash: &str) -> Result<bool> {
        let mut tx = self.core.pool().begin().await?;
        self.capacity(&mut tx).await?;
        let n=sqlx::query("INSERT INTO feishu_inbox(namespace,id,payload,config_hash) VALUES(?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(&self.namespace).bind(&input.id).bind(serde_json::to_string(input)?).bind(hash).execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
        Ok(n == 1)
    }

    async fn capacity(&self, tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<()> {
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM feishu_inbox WHERE namespace=? AND state IN ('pending','processing')").bind(&self.namespace).fetch_one(&mut **tx).await?;
        if count >= 512 {
            return Err(Error::Ui("feishu.error.busy"));
        }
        Ok(())
    }

    pub async fn callback(
        &self,
        nonce: &str,
        message: &str,
        input: &Input,
        hash: &str,
        title: Option<String>,
    ) -> Result<bool> {
        let mut tx = self.core.pool().begin().await?;
        self.capacity(&mut tx).await?;
        let action:Option<String>=sqlx::query_scalar("UPDATE feishu_actions SET consumed=1 WHERE nonce=? AND namespace=? AND chat=? AND message=? AND config_hash=? AND expires>=? AND consumed=0 RETURNING payload")
            .bind(nonce).bind(&self.namespace).bind(&input.chat).bind(message).bind(hash).bind(engine::now()).fetch_optional(&mut *tx).await?;
        let Some(action) = action else {
            return Ok(false);
        };
        let saved: SavedAction = serde_json::from_str(&action)?;
        let mut input = input.clone();
        input.command = match saved.action {
            Action::Create { project } => super::model::Command::Create {
                project,
                title: title
                    .filter(|t| {
                        !t.trim().is_empty()
                            && t.chars().count() <= 80
                            && !t.contains(['\0', '\r', '\n'])
                    })
                    .ok_or(Error::Ui("feishu.error.title"))?,
            },
            action => super::model::Command::Menu(action),
        };
        let n=sqlx::query("INSERT INTO feishu_inbox(namespace,id,payload,config_hash) VALUES(?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(&self.namespace).bind(&input.id).bind(serde_json::to_string(&input)?).bind(hash).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn action(&self, chat: &str, action: Action, hash: &str) -> Result<String> {
        sqlx::query("DELETE FROM feishu_actions WHERE namespace=? AND expires<?")
            .bind(&self.namespace)
            .bind(engine::now())
            .execute(self.core.pool())
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM feishu_actions WHERE namespace=?")
                .bind(&self.namespace)
                .fetch_one(self.core.pool())
                .await?;
        if count >= 8192 {
            return Err(Error::Ui("feishu.error.busy"));
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO feishu_actions(nonce,namespace,chat,payload,config_hash,expires) VALUES(?,?,?,?,?,?)")
            .bind(&nonce).bind(&self.namespace).bind(chat).bind(serde_json::to_string(&SavedAction {action,hash:hash.into()})?).bind(hash).bind(engine::now()+300).execute(self.core.pool()).await?;
        Ok(nonce)
    }

    pub async fn next(&self) -> Result<Option<Inbox>> {
        Ok(sqlx::query_as("UPDATE feishu_inbox SET state='processing' WHERE namespace=? AND id=(SELECT id FROM feishu_inbox WHERE namespace=? AND state='pending' ORDER BY rowid LIMIT 1) AND state='pending' RETURNING id,payload,config_hash")
            .bind(&self.namespace).bind(&self.namespace).fetch_optional(self.core.pool()).await?)
    }

    pub async fn done(&self, id: &str, failed: bool) -> Result<()> {
        sqlx::query("UPDATE feishu_inbox SET state=?,payload='' WHERE namespace=? AND id=? AND state='processing'").bind(if failed {"failed"}else{"done"}).bind(&self.namespace).bind(id).execute(self.core.pool()).await?;
        Ok(())
    }

    pub async fn binding(&self, id: &str) -> Result<Binding> {
        sqlx::query_as("SELECT id,chat,root,topic,project,instance,session,cwd,endpoint,revision,active,cursor,ready,status,state FROM feishu_bindings WHERE namespace=? AND id=?")
            .bind(&self.namespace).bind(id).fetch_optional(self.core.pool()).await?.ok_or(Error::Ui("feishu.error.scope"))
    }

    pub async fn bound(&self, chat: &str, topic: &str, root: &str) -> Result<Binding> {
        sqlx::query_as("SELECT id,chat,root,topic,project,instance,session,cwd,endpoint,revision,active,cursor,ready,status,state FROM feishu_bindings WHERE namespace=? AND chat=? AND topic=? AND root=? AND state='ready' AND active=1")
            .bind(&self.namespace).bind(chat).bind(topic).bind(root).fetch_optional(self.core.pool()).await?.ok_or(Error::Ui("feishu.error.thread"))
    }

    pub async fn reserve(&self, b: &Binding) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM feishu_bindings WHERE namespace=? AND active=1",
        )
        .bind(&self.namespace)
        .fetch_one(&mut *tx)
        .await?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM feishu_bindings WHERE namespace=?")
                .bind(&self.namespace)
                .fetch_one(&mut *tx)
                .await?;
        if total >= 1024 || count >= 256 {
            return Err(Error::Ui("feishu.error.busy"));
        }
        let n=sqlx::query("INSERT INTO feishu_bindings(id,namespace,chat,project,instance,session,cwd,endpoint) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(namespace,chat,instance,session) DO UPDATE SET id=excluded.id,root='',topic='',active=1,state='provisioning',revision=feishu_bindings.revision+1,cursor='{}',ready=0,status='',cwd=excluded.cwd,endpoint=excluded.endpoint WHERE feishu_bindings.active=0 AND feishu_bindings.state='ready'")
            .bind(&b.id).bind(&self.namespace).bind(&b.chat).bind(&b.project).bind(&b.instance).bind(&b.session).bind(&b.cwd).bind(&b.endpoint).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Ui("feishu.error.bound"));
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn provision(
        &self,
        id: &str,
        root: Option<&str>,
        topic: Option<&str>,
        failed: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE feishu_bindings SET root=COALESCE(?,root),topic=COALESCE(?,topic),state=CASE WHEN ? THEN 'unknown' WHEN ? IS NOT NULL THEN 'ready' ELSE state END WHERE namespace=? AND id=? AND state='provisioning'")
            .bind(root).bind(topic).bind(failed).bind(topic).bind(&self.namespace).bind(id).execute(self.core.pool()).await?;
        Ok(())
    }

    pub async fn ready_bindings(&self) -> Result<Vec<Binding>> {
        Ok(sqlx::query_as("SELECT id,chat,root,topic,project,instance,session,cwd,endpoint,revision,active,cursor,ready,status,state FROM feishu_bindings WHERE namespace=? AND active=1 AND state='ready' ORDER BY rowid LIMIT 256").bind(&self.namespace).fetch_all(self.core.pool()).await?)
    }

    pub async fn output(
        &self,
        destination: &Destination,
        cards: &[Card],
        hash: &str,
        binding: Option<&Binding>,
        edit: Option<&str>,
        nonces: &[String],
    ) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        self.freeze(
            &mut tx,
            destination,
            cards,
            hash,
            binding,
            edit,
            nonces,
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn freeze(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        destination: &Destination,
        cards: &[Card],
        hash: &str,
        binding: Option<&Binding>,
        edit: Option<&str>,
        nonces: &[String],
        automatic: bool,
    ) -> Result<()> {
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM feishu_outbox WHERE namespace=? AND state IN ('pending','dispatching')").bind(&self.namespace).fetch_one(&mut **tx).await?;
        if cards.len() > 256
            || count.saturating_add(cards.len() as i64) > 1024
            || cards.iter().any(|c| !c.bounded())
        {
            return Err(Error::Ui("feishu.error.busy"));
        }
        for card in cards {
            sqlx::query("INSERT INTO feishu_outbox(id,namespace,destination,edit,binding,revision,content,nonces,config_hash,automatic) VALUES(?,?,?,?,?,?,?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(&self.namespace).bind(serde_json::to_string(destination)?).bind(edit).bind(binding.map(|b|&b.id)).bind(binding.map(|b|b.revision)).bind(serde_json::to_string(card)?).bind(serde_json::to_string(nonces)?).bind(hash).bind(automatic).execute(&mut **tx).await?;
        }
        Ok(())
    }

    pub async fn publish(
        &self,
        b: &Binding,
        cards: &[Card],
        cursor: &str,
        status: &str,
        status_card: Option<&Card>,
        hash: &str,
    ) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        let n=sqlx::query("UPDATE feishu_bindings SET cursor=?,ready=1,status=? WHERE namespace=? AND id=? AND revision=? AND active=1 AND state='ready'")
            .bind(cursor).bind(status).bind(&self.namespace).bind(&b.id).bind(b.revision).execute(&mut *tx).await?.rows_affected();
        if n == 0 {
            return Ok(());
        }
        let destination = Destination {
            chat: b.chat.clone(),
            root: Some(b.root.clone()),
        };
        self.freeze(&mut tx, &destination, cards, hash, Some(b), None, &[], true)
            .await?;
        if let Some(card) = status_card {
            self.freeze(
                &mut tx,
                &destination,
                std::slice::from_ref(card),
                hash,
                Some(b),
                Some(&b.root),
                &[],
                true,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn outgoing(&self) -> Result<Option<Outbox>> {
        Ok(sqlx::query_as("UPDATE feishu_outbox SET state='dispatching' WHERE namespace=? AND id=(SELECT id FROM feishu_outbox WHERE namespace=? AND state='pending' ORDER BY rowid LIMIT 1) AND state='pending' RETURNING id,destination,edit,binding,revision,content,nonces,automatic,config_hash,created")
            .bind(&self.namespace).bind(&self.namespace).fetch_optional(self.core.pool()).await?)
    }

    pub async fn delivered(&self, o: &Outbox, message: Option<&str>, failed: bool) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        if let Some(message) = message {
            let nonces: Vec<String> = serde_json::from_str(&o.nonces)?;
            for nonce in nonces {
                sqlx::query("UPDATE feishu_actions SET message=? WHERE namespace=? AND nonce=? AND message IS NULL").bind(message).bind(&self.namespace).bind(nonce).execute(&mut *tx).await?;
            }
        }
        sqlx::query("UPDATE feishu_outbox SET state=?,content='' WHERE namespace=? AND id=? AND state='dispatching'")
            .bind(if failed {"unknown"}else{"sent"}).bind(&self.namespace).bind(&o.id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
