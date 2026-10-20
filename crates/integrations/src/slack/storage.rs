use crate::slack::{Error, Result, model::*};
use engine::storage::Store as CoreStore;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Clone)]
pub struct Store {
    pub core: CoreStore,
    pub namespace: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Binding {
    pub id: String,
    pub app: String,
    pub team: String,
    pub bot: String,
    pub channel: String,
    pub root: String,
    pub owner: String,
    pub workspace: String,
    pub instance: String,
    pub thread: String,
    pub cwd: String,
    pub endpoint: String,
    pub active: bool,
    pub revision: i64,
    pub observed: String,
    pub card: String,
    pub history_ready: bool,
}

#[derive(sqlx::FromRow)]
pub struct Inbox {
    pub id: String,
    pub request: String,
    pub config_hash: String,
    pub state: String,
}

#[derive(Clone, sqlx::FromRow)]
pub struct Outbox {
    pub id: String,
    pub channel: String,
    pub root: Option<String>,
    pub binding: Option<String>,
    pub revision: Option<i64>,
    pub content: String,
    pub config_hash: String,
    pub next_part: i64,
    pub rendered_parts: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SavedAction {
    #[serde(default)]
    menu_message: Option<String>,
    #[serde(flatten)]
    action: Action,
}

impl Store {
    pub fn new(core: CoreStore, app: &str, team: &str, bot: &str) -> Self {
        Self {
            core,
            namespace: format!("{app}:{team}:{bot}"),
        }
    }

    pub async fn recover(&self) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        sqlx::query("UPDATE slack_inbox SET state='draft' WHERE namespace=? AND state IN ('pending','awaiting_confirmation')")
            .bind(&self.namespace)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE slack_inbox SET state='uncertain' WHERE namespace=? AND state='processing'",
        )
        .bind(&self.namespace)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE slack_outbox SET state='outcome_unknown',content='',rendered_parts=NULL WHERE namespace=? AND state='dispatching'")
            .bind(&self.namespace).execute(&mut *tx).await?;
        sqlx::query("UPDATE slack_provisions SET state='outcome_unknown' WHERE app||':'||team||':'||bot=? AND state='dispatching'")
            .bind(&self.namespace).execute(&mut *tx).await?;
        sqlx::query("UPDATE slack_actions SET consumed=1 WHERE namespace=?")
            .bind(&self.namespace)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn enqueue(
        &self,
        source: &str,
        request: &Request,
        hash: &str,
        draft: bool,
    ) -> Result<bool> {
        let mut tx = self.core.pool().begin().await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_inbox WHERE namespace=? AND state IN ('pending','processing','draft')")
            .bind(&self.namespace).fetch_one(&mut *tx).await?;
        if count >= 512 {
            return Err(Error::Ui("slack.error.busy"));
        }
        let n = sqlx::query("INSERT INTO slack_inbox(id,namespace,source,request,config_hash,state) VALUES(?,?,?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(&request.id).bind(&self.namespace).bind(source).bind(serde_json::to_string(request)?)
            .bind(hash).bind(if draft { "draft" } else { "pending" }).execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
        Ok(n == 1)
    }

    pub async fn pending(&self) -> Result<Option<Inbox>> {
        Ok(sqlx::query_as("SELECT id,request,config_hash,state FROM slack_inbox WHERE namespace=? AND state='pending' ORDER BY rowid LIMIT 1")
            .bind(&self.namespace).fetch_optional(self.core.pool()).await?)
    }

    pub async fn inbox(&self, id: &str) -> Result<Option<Inbox>> {
        Ok(sqlx::query_as(
            "SELECT id,request,config_hash,state FROM slack_inbox WHERE namespace=? AND id=?",
        )
        .bind(&self.namespace)
        .bind(id)
        .fetch_optional(self.core.pool())
        .await?)
    }

    pub async fn drafts(&self) -> Result<Vec<Inbox>> {
        Ok(sqlx::query_as("SELECT id,request,config_hash,state FROM slack_inbox WHERE namespace=? AND state IN ('draft','uncertain') ORDER BY rowid LIMIT 64")
            .bind(&self.namespace).fetch_all(self.core.pool()).await?)
    }

    pub async fn state(&self, id: &str, from: &str, to: &str) -> Result<bool> {
        Ok(sqlx::query("UPDATE slack_inbox SET state=?,request=CASE WHEN ?='done' THEN '' ELSE request END WHERE namespace=? AND id=? AND state=?")
            .bind(to).bind(to).bind(&self.namespace).bind(id).bind(from).execute(self.core.pool()).await?.rows_affected() == 1)
    }

    pub async fn action(&self, request: &Request, action: Action, hash: &str) -> Result<String> {
        sqlx::query("DELETE FROM slack_actions WHERE namespace=? AND expires<?")
            .bind(&self.namespace)
            .bind(engine::now())
            .execute(self.core.pool())
            .await?;
        let ttl = match &action {
            Action::Run(c) if !c.writes() => 86400 * 30,
            _ => 300,
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO slack_actions(nonce,namespace,channel,owner,root,binding,revision,action,config_hash,expires) VALUES(?,?,?,?,?,?,?,?,?,?)")
            .bind(&nonce).bind(&self.namespace).bind(&request.channel).bind(&request.user).bind(&request.root)
            .bind(&request.binding).bind(request.revision).bind(serde_json::to_string(&SavedAction { action, menu_message: request.menu_message.clone() })?)
            .bind(hash).bind(engine::now()+ttl).execute(self.core.pool()).await?;
        Ok(nonce)
    }

    pub async fn consume(
        &self,
        nonce: &str,
        channel: Option<&str>,
        user: &str,
        hash: &str,
        title: Option<&str>,
        source: InteractionSource<'_>,
    ) -> Result<Option<(Request, Action)>> {
        let mut tx = self.core.pool().begin().await?;
        let row = sqlx::query("SELECT * FROM slack_actions WHERE nonce=? AND namespace=? AND owner=? AND consumed=0 AND expires>=? AND config_hash=?")
            .bind(nonce).bind(&self.namespace).bind(user).bind(engine::now()).bind(hash).fetch_optional(&mut *tx).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let saved_channel: String = row.get("channel");
        if channel.is_some_and(|c| c != saved_channel) {
            return Ok(None);
        }
        let action: SavedAction = serde_json::from_str(row.get("action"))?;
        let reusable = matches!(&action.action,Action::Run(command) if !command.writes());
        if !reusable {
            sqlx::query("UPDATE slack_actions SET consumed=1 WHERE nonce=? AND consumed=0")
                .bind(nonce)
                .execute(&mut *tx)
                .await?;
        }
        let mut request = Request {
            uploads: Vec::new(),
            menu_message: None,
            id: if reusable {
                format!("action:{nonce}:{}", source.id.unwrap_or("read"))
            } else {
                format!("action:{nonce}")
            },
            channel: saved_channel,
            user: user.into(),
            root: row.get("root"),
            binding: row.get("binding"),
            revision: row.get("revision"),
            date: engine::now(),
            command: Command::Menu,
        };
        if request.binding.is_none() {
            request.menu_message = source.message.map(str::to_owned).or(action.menu_message);
        }
        let command = match &action.action {
            Action::Run(command) => Some(command.clone()),
            Action::NewSubmit { workspace } => title.map(|title| Command::New {
                workspace: workspace.clone(),
                title: title.into(),
            }),
            _ => None,
        };
        if let Some(command) = command {
            request.command = command;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_inbox WHERE namespace=? AND state IN ('pending','processing','draft')")
                .bind(&self.namespace).fetch_one(&mut *tx).await?;
            if count >= 512 {
                return Err(Error::Ui("slack.error.busy"));
            }
            sqlx::query("INSERT INTO slack_inbox(id,namespace,source,request,config_hash,state) VALUES(?,?,?,?,?,'pending') ON CONFLICT DO NOTHING")
                .bind(&request.id).bind(&self.namespace).bind(&request.id).bind(serde_json::to_string(&request)?).bind(hash).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(Some((request, action.action)))
    }

    pub async fn binding(&self, id: &str) -> Result<Option<Binding>> {
        Ok(sqlx::query_as(
            "SELECT * FROM slack_bindings WHERE id=? AND app||':'||team||':'||bot=? AND active=1",
        )
        .bind(id)
        .bind(&self.namespace)
        .fetch_optional(self.core.pool())
        .await?)
    }

    pub async fn at(&self, channel: &str, root: &str) -> Result<Option<Binding>> {
        Ok(sqlx::query_as("SELECT * FROM slack_bindings WHERE app||':'||team||':'||bot=? AND channel=? AND root=? AND active=1")
            .bind(&self.namespace).bind(channel).bind(root).fetch_optional(self.core.pool()).await?)
    }

    pub async fn existing(
        &self,
        channel: &str,
        instance: &str,
        thread: &str,
    ) -> Result<Option<Binding>> {
        Ok(sqlx::query_as("SELECT * FROM slack_bindings WHERE app||':'||team||':'||bot=? AND channel=? AND instance=? AND thread=?")
            .bind(&self.namespace).bind(channel).bind(instance).bind(thread).fetch_optional(self.core.pool()).await?)
    }

    pub async fn bindings(&self) -> Result<Vec<Binding>> {
        Ok(sqlx::query_as(
            "SELECT * FROM slack_bindings WHERE app||':'||team||':'||bot=? AND active=1 LIMIT 512",
        )
        .bind(&self.namespace)
        .fetch_all(self.core.pool())
        .await?)
    }

    pub async fn reserve(&self, b: &Binding) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        if !b.channel.starts_with('D') {
            return Err(Error::Ui("slack.error.scope"));
        }
        let n = sqlx::query("INSERT INTO slack_provisions(app,team,bot,channel,instance,thread,state) VALUES(?,?,?,?,?,?,'dispatching') ON CONFLICT DO NOTHING")
            .bind(&b.app).bind(&b.team).bind(&b.bot).bind(&b.channel).bind(&b.instance).bind(&b.thread).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Ui("slack.error.unknown"));
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn bind(&self, b: &Binding) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        sqlx::query("INSERT INTO slack_bindings(id,app,team,bot,channel,root,owner,workspace,instance,thread,cwd,endpoint) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&b.id).bind(&b.app).bind(&b.team).bind(&b.bot).bind(&b.channel).bind(&b.root).bind(&b.owner)
            .bind(&b.workspace).bind(&b.instance).bind(&b.thread).bind(&b.cwd).bind(&b.endpoint).execute(&mut *tx).await?;
        sqlx::query("UPDATE slack_provisions SET state='accepted' WHERE app=? AND team=? AND bot=? AND channel=? AND instance=? AND thread=?")
            .bind(&b.app).bind(&b.team).bind(&b.bot).bind(&b.channel).bind(&b.instance).bind(&b.thread).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn activate(&self, b: &Binding, active: bool) -> Result<()> {
        sqlx::query(
            "UPDATE slack_bindings SET active=?,revision=revision+1 WHERE id=? AND revision=?",
        )
        .bind(active)
        .bind(&b.id)
        .bind(b.revision)
        .execute(self.core.pool())
        .await?;
        Ok(())
    }

    pub async fn publish(
        &self,
        b: &Binding,
        content: &Content,
        observed: &str,
        hash: &str,
    ) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        let n = sqlx::query("UPDATE slack_bindings SET observed=?,history_ready=1 WHERE id=? AND revision=? AND active=1 AND observed=?")
            .bind(observed).bind(&b.id).bind(b.revision).bind(&b.observed).execute(&mut *tx).await?.rows_affected();
        if n == 1 && !content.text.is_empty() {
            let r = Request {
                uploads: Vec::new(),
                menu_message: None,
                id: format!("observe:{}:{}", b.id, engine::digest(observed)),
                channel: b.channel.clone(),
                user: b.owner.clone(),
                root: Some(b.root.clone()),
                binding: Some(b.id.clone()),
                revision: Some(b.revision),
                date: engine::now(),
                command: Command::Status,
            };
            Self::insert_output(&mut tx, &self.namespace, &r, content, hash).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn output(&self, r: &Request, content: &Content, hash: &str) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        Self::insert_output(&mut tx, &self.namespace, r, content, hash).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn insert_output(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        ns: &str,
        r: &Request,
        c: &Content,
        hash: &str,
    ) -> Result<()> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_outbox WHERE namespace=? AND state IN ('pending','dispatching')")
            .bind(ns).fetch_one(&mut **tx).await?;
        if count >= 2048 {
            return Err(Error::Ui("slack.error.busy"));
        }
        if c.text.len() > 8 * 1024 * 1024 {
            return Err(Error::Ui("slack.error.history"));
        }
        sqlx::query("INSERT INTO slack_outbox(id,namespace,channel,root,binding,revision,source,content,config_hash) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING")
            .bind(uuid::Uuid::now_v7().to_string()).bind(ns).bind(&r.channel).bind(&r.root).bind(&r.binding).bind(r.revision)
            .bind(&r.id).bind(serde_json::to_string(c)?).bind(hash).execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn outgoing(&self) -> Result<Vec<Outbox>> {
        Ok(sqlx::query_as("SELECT id,channel,root,binding,revision,content,config_hash,next_part,rendered_parts FROM slack_outbox o WHERE namespace=? AND state='pending' AND not_before<=? AND NOT EXISTS(SELECT 1 FROM slack_outbox p WHERE p.namespace=o.namespace AND p.channel=o.channel AND p.state IN ('pending','dispatching') AND p.rowid<o.rowid) ORDER BY rowid LIMIT 32")
            .bind(&self.namespace).bind(engine::now()).fetch_all(self.core.pool()).await?)
    }

    pub async fn claim_output(&self, o: &Outbox) -> Result<bool> {
        let mut tx = self.core.pool().begin().await?;
        let next: Option<i64> = sqlx::query_scalar(
            "SELECT next_send FROM slack_channel_budget WHERE namespace=? AND channel=?",
        )
        .bind(&self.namespace)
        .bind(&o.channel)
        .fetch_optional(&mut *tx)
        .await?;
        if next.is_some_and(|n| n > engine::now()) {
            return Ok(false);
        }
        let n = sqlx::query(
            "UPDATE slack_outbox SET state='dispatching' WHERE id=? AND state='pending' AND next_part=? AND not_before<=unixepoch()",
        )
        .bind(&o.id)
        .bind(o.next_part)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            sqlx::query("INSERT INTO slack_channel_budget(namespace,channel,next_send) VALUES(?,?,?) ON CONFLICT(namespace,channel) DO UPDATE SET next_send=excluded.next_send")
                .bind(&self.namespace).bind(&o.channel).bind(engine::now()+2).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(n == 1)
    }

    pub async fn rate_limit(&self, id: &str, delay: u64) -> Result<()> {
        let mut tx = self.core.pool().begin().await?;
        sqlx::query("UPDATE slack_outbox SET state='pending' WHERE namespace=? AND id=? AND state='dispatching'")
            .bind(&self.namespace).bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE slack_outbox SET not_before=max(not_before,?) WHERE namespace=? AND state='pending'")
            .bind(engine::now()+delay.clamp(1,3600) as i64).bind(&self.namespace).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_output(&self, id: &str, state: &str, delay: i64) -> Result<()> {
        sqlx::query("UPDATE slack_outbox SET state=?,not_before=?,content=CASE WHEN ?='pending' THEN content ELSE '' END,rendered_parts=CASE WHEN ?='pending' THEN rendered_parts ELSE NULL END WHERE id=?")
            .bind(state).bind(engine::now()+delay).bind(state).bind(state).bind(id).execute(self.core.pool()).await?;
        Ok(())
    }

    pub async fn acknowledge_part(&self, o: &Outbox, more: bool) -> Result<()> {
        let changed = sqlx::query("UPDATE slack_outbox SET next_part=next_part+1,state=?,not_before=?,content=CASE WHEN ? THEN content ELSE '' END,rendered_parts=CASE WHEN ? THEN rendered_parts ELSE NULL END WHERE id=? AND state='dispatching' AND next_part=?")
            .bind(if more { "pending" } else { "delivered" })
            .bind(engine::now()+2).bind(more).bind(more).bind(&o.id).bind(o.next_part)
            .execute(self.core.pool()).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Request);
        }
        Ok(())
    }

    pub async fn prepare_parts(&self, o: &Outbox, parts: &[Content]) -> Result<()> {
        sqlx::query("UPDATE slack_outbox SET rendered_parts=? WHERE id=? AND state='pending' AND next_part=0 AND rendered_parts IS NULL")
            .bind(serde_json::to_string(parts)?).bind(&o.id).execute(self.core.pool()).await?;
        Ok(())
    }

    pub async fn card(&self, b: &Binding, text: &str) -> Result<()> {
        sqlx::query("UPDATE slack_bindings SET card=? WHERE id=? AND revision=? AND active=1")
            .bind(text)
            .bind(&b.id)
            .bind(b.revision)
            .execute(self.core.pool())
            .await?;
        Ok(())
    }
}
