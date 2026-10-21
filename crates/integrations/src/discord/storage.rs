//! Discord-specific container provenance and output ledger; reuses the operation ledger.
use engine::storage::{Store as CoreStore, StoreError as Error};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct Store {
    core: CoreStore,
    pool: SqlitePool,
}

impl std::ops::Deref for Store {
    type Target = CoreStore;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl Store {
    pub fn new(core: CoreStore) -> Self {
        Self {
            pool: core.pool().clone(),
            core,
        }
    }

    #[cfg(test)]
    pub async fn open(path: &std::path::Path) -> Result<Self, Error> {
        Ok(Self::new(CoreStore::open(path).await?))
    }
}
use sqlx::Row;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Binding {
    pub bot: String,
    pub guild: String,
    pub channel: String,
    pub owner: String,
    pub workspace: String,
    pub instance: String,
    pub thread: String,
    pub cwd: String,
    pub endpoint: String,
    pub active: bool,
    pub revision: i64,
    pub observed_items: String,
    pub history_ready: bool,
    pub card_message: Option<String>,
    pub card_text: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct Delivery {
    pub id: String,
    pub channel: String,
    pub revision: i64,
    pub payload: String,
    pub next_part: i64,
    pub rendered_parts: Option<String>,
}

impl Store {
    pub async fn ui_action(
        &self,
        input: &crate::discord::Input,
        hash: &str,
        action: crate::discord::ui::Action,
    ) -> Result<String, Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        sqlx::query("DELETE FROM discord_ui_actions WHERE expires<?")
            .bind(engine::now())
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        // Keep the latest screens bounded even when the owner repeatedly refreshes.
        sqlx::query("DELETE FROM discord_ui_actions WHERE rowid IN (SELECT rowid FROM discord_ui_actions WHERE bot=? AND guild=? AND owner=? ORDER BY rowid DESC LIMIT -1 OFFSET 1024)")
            .bind(input.bot.to_string()).bind(input.guild.to_string()).bind(input.user.to_string())
            .execute(&mut *tx).await.map_err(|_| Error::Database)?;
        let ttl = if matches!(
            &action,
            crate::discord::ui::Action::Navigate(crate::discord::ui::View::BindingChange { .. })
        ) {
            300
        } else {
            86400
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO discord_ui_actions(nonce,bot,guild,owner,channel,config_hash,action,expires) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&nonce).bind(input.bot.to_string()).bind(input.guild.to_string()).bind(input.user.to_string()).bind(input.channel.to_string())
            .bind(hash).bind(serde_json::to_string(&action).map_err(|_| Error::Database)?)
            .bind(engine::now()+ttl).execute(&mut *tx).await.map_err(|_| Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)?;
        Ok(nonce)
    }

    pub async fn ui_resolve(
        &self,
        input: &crate::discord::Input,
        hash: &str,
        nonce: &str,
        value: Option<&str>,
    ) -> Result<Option<crate::discord::ui::View>, Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let row = sqlx::query("SELECT action FROM discord_ui_actions WHERE nonce=? AND bot=? AND guild=? AND owner=? AND channel=? AND config_hash=? AND expires>=? AND consumed=0")
            .bind(nonce).bind(input.bot.to_string()).bind(input.guild.to_string()).bind(input.user.to_string()).bind(input.channel.to_string())
            .bind(hash).bind(engine::now()).fetch_optional(&mut *tx).await.map_err(|_| Error::Database)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let action: crate::discord::ui::Action =
            serde_json::from_str(row.get("action")).map_err(|_| Error::Database)?;
        let view = match (action, value) {
            (crate::discord::ui::Action::Navigate(view), None) => Some(view),
            (crate::discord::ui::Action::Select(choices), Some(value)) => value
                .parse::<usize>()
                .ok()
                .and_then(|index| choices.get(index).cloned()),
            _ => None,
        };
        if matches!(
            view,
            Some(
                crate::discord::ui::View::Open { .. }
                    | crate::discord::ui::View::BindingChange { .. }
            )
        ) {
            sqlx::query("UPDATE discord_ui_actions SET consumed=1 WHERE nonce=? AND consumed=0")
                .bind(nonce)
                .execute(&mut *tx)
                .await
                .map_err(|_| Error::Database)?;
        }
        tx.commit().await.map_err(|_| Error::Database)?;
        Ok(view)
    }

    pub async fn discord_recover(&self) -> Result<(), Error> {
        for table in [
            "discord_categories",
            "discord_provisions",
            "discord_deliveries",
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET state='outcome_unknown' WHERE state='dispatching'"
            ))
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?;
        }
        Ok(())
    }

    /// Atomically reserve once. Existing unknown resources must be reconciled by
    /// the operator; reconnect never retries Discord resource creation.
    pub async fn discord_reserve(
        &self,
        bot: u64,
        guild: u64,
        workspace: &str,
        session: Option<(&str, &str)>,
    ) -> Result<Option<u64>, Error> {
        let bot = bot.to_string();
        let guild = guild.to_string();
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let (n, row) = if let Some((instance, thread)) = session {
            let n=sqlx::query("INSERT INTO discord_provisions(bot,guild,instance,thread,state) VALUES(?,?,?,?,'dispatching') ON CONFLICT DO NOTHING")
                .bind(&bot).bind(&guild).bind(instance).bind(thread).execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
            let row=sqlx::query("SELECT state,channel FROM discord_provisions WHERE bot=? AND guild=? AND instance=? AND thread=?")
                .bind(&bot).bind(&guild).bind(instance).bind(thread).fetch_one(&mut *tx).await.map_err(|_|Error::Database)?;
            (n, row)
        } else {
            let n=sqlx::query("INSERT INTO discord_categories(bot,guild,workspace,state) VALUES(?,?,?,'dispatching') ON CONFLICT DO NOTHING")
                .bind(&bot).bind(&guild).bind(workspace).execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
            let row=sqlx::query("SELECT state,channel FROM discord_categories WHERE bot=? AND guild=? AND workspace=?")
                .bind(&bot).bind(&guild).bind(workspace).fetch_one(&mut *tx).await.map_err(|_|Error::Database)?;
            (n, row)
        };
        tx.commit().await.map_err(|_| Error::Database)?;
        if n == 1 {
            return Ok(None);
        }
        if row.get::<String, _>("state") != "accepted" {
            return Err(Error::ProvisionUnknown);
        }
        row.get::<Option<String>, _>("channel")
            .and_then(|id| id.parse().ok())
            .map(Some)
            .ok_or(Error::Database)
    }

    pub async fn discord_category_receipt(
        &self,
        bot: u64,
        guild: u64,
        workspace: &str,
        channel: u64,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE discord_categories SET state='accepted',channel=? WHERE bot=? AND guild=? AND workspace=? AND state='dispatching'")
            .bind(channel.to_string()).bind(bot.to_string()).bind(guild.to_string()).bind(workspace).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }

    pub async fn discord_bind(&self, b: &Binding) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        sqlx::query("INSERT INTO discord_bindings(bot,guild,channel,owner,workspace,instance,thread,cwd,endpoint) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(bot,guild,channel) DO NOTHING")
            .bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(&b.owner).bind(&b.workspace).bind(&b.instance).bind(&b.thread).bind(&b.cwd).bind(&b.endpoint)
            .execute(&mut *tx).await.map_err(|_|Error::Database)?;
        let saved: Binding =
            sqlx::query_as("SELECT * FROM discord_bindings WHERE bot=? AND guild=? AND channel=?")
                .bind(&b.bot)
                .bind(&b.guild)
                .bind(&b.channel)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| Error::Database)?;
        if saved.instance != b.instance
            || saved.thread != b.thread
            || saved.owner != b.owner
            || saved.workspace != b.workspace
            || saved.cwd != b.cwd
            || saved.endpoint != b.endpoint
        {
            return Err(Error::ContainerConflict);
        }
        sqlx::query("UPDATE discord_bindings SET active=1,revision=revision+1 WHERE bot=? AND guild=? AND channel=? AND active=0")
            .bind(&b.bot).bind(&b.guild).bind(&b.channel).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        sqlx::query("UPDATE discord_provisions SET state='accepted',channel=? WHERE bot=? AND guild=? AND instance=? AND thread=? AND state='dispatching'")
            .bind(&b.channel).bind(&b.bot).bind(&b.guild).bind(&b.instance).bind(&b.thread).execute(&mut *tx).await.map_err(|_|Error::Database)?;
        tx.commit().await.map_err(|_| Error::Database)
    }

    pub async fn discord_bindings(&self, bot: u64, guild: u64) -> Result<Vec<Binding>, Error> {
        sqlx::query_as("SELECT * FROM discord_bindings WHERE bot=? AND guild=? AND active=1")
            .bind(bot.to_string())
            .bind(guild.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|_| Error::Database)
    }

    pub async fn discord_binding(
        &self,
        bot: u64,
        guild: u64,
        channel: u64,
    ) -> Result<Option<Binding>, Error> {
        sqlx::query_as(
            "SELECT * FROM discord_bindings WHERE bot=? AND guild=? AND channel=? AND active=1",
        )
        .bind(bot.to_string())
        .bind(guild.to_string())
        .bind(channel.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| Error::Database)
    }

    pub async fn discord_unbind(&self, b: &Binding) -> Result<(), Error> {
        sqlx::query("UPDATE discord_bindings SET active=0,revision=revision+1 WHERE bot=? AND guild=? AND channel=? AND revision=?")
            .bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(b.revision).execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }

    pub(crate) async fn discord_publish(
        &self,
        b: &Binding,
        source: &str,
        output: &crate::discord::render::Conversation,
        observed: &str,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let n=sqlx::query("UPDATE discord_bindings SET observed_items=?,history_ready=1 WHERE bot=? AND guild=? AND channel=? AND revision=? AND active=1 AND observed_items=?")
            .bind(observed).bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(b.revision).bind(&b.observed_items).execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
        if n == 1 && !output.parts.is_empty() {
            let native = output
                .parts
                .iter()
                .cloned()
                .map(crate::discord::native::Part::V2)
                .collect::<Vec<_>>();
            let parts = serde_json::to_string(&native).map_err(|_| Error::Database)?;
            sqlx::query("INSERT INTO discord_deliveries(id,bot,guild,channel,source_key,revision,payload,rendered_parts,state) VALUES(?,?,?,?,?,?,?,?,'pending') ON CONFLICT DO NOTHING")
                .bind(uuid::Uuid::now_v7().to_string()).bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(source).bind(b.revision).bind(&output.text).bind(parts)
                .execute(&mut *tx).await.map_err(|_|Error::Database)?;
        }
        tx.commit().await.map_err(|_| Error::Database)
    }

    pub async fn discord_pending(
        &self,
        bot: u64,
        guild: u64,
        now: i64,
    ) -> Result<Vec<Delivery>, Error> {
        sqlx::query_as("SELECT id,channel,revision,payload,next_part,rendered_parts FROM discord_deliveries d WHERE bot=? AND guild=? AND state='pending' AND not_before<=? AND NOT EXISTS(SELECT 1 FROM discord_deliveries old WHERE old.bot=d.bot AND old.guild=d.guild AND old.channel=d.channel AND old.state IN ('pending','dispatching') AND old.rowid<d.rowid) ORDER BY rowid LIMIT 32")
            .bind(bot.to_string()).bind(guild.to_string()).bind(now).fetch_all(&self.pool).await.map_err(|_|Error::Database)
    }

    pub async fn discord_delivery_state(
        &self,
        id: &str,
        from: &str,
        to: &str,
    ) -> Result<(), Error> {
        if !matches!(
            (from, to),
            ("pending", "dispatching" | "rejected")
                | ("dispatching", "delivered" | "outcome_unknown")
        ) {
            return Err(Error::Transition);
        }
        let n = sqlx::query("UPDATE discord_deliveries SET state=? WHERE id=? AND state=?")
            .bind(to)
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await
            .map_err(|_| Error::Database)?
            .rows_affected();
        if n != 1 {
            return Err(Error::Transition);
        }
        Ok(())
    }

    pub async fn discord_card(
        &self,
        b: &Binding,
        message: Option<u64>,
        text: &str,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE discord_bindings SET card_message=?,card_text=? WHERE bot=? AND guild=? AND channel=? AND revision=?")
            .bind(message.map(|id|id.to_string())).bind(text).bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(b.revision)
            .execute(&self.pool).await.map_err(|_|Error::Database)?;
        Ok(())
    }

    pub async fn acknowledge_part(&self, row: &Delivery, more: bool) -> Result<(), Error> {
        let changed = sqlx::query("UPDATE discord_deliveries SET next_part=next_part+1,state=?,not_before=?,payload=CASE WHEN ? THEN payload ELSE '' END,rendered_parts=CASE WHEN ? THEN rendered_parts ELSE NULL END WHERE id=? AND state='dispatching' AND next_part=?")
            .bind(if more { "pending" } else { "delivered" })
            .bind(engine::now()+4).bind(more).bind(more).bind(&row.id).bind(row.next_part)
            .execute(&self.pool).await.map_err(|_| Error::Database)?.rows_affected();
        if changed != 1 {
            return Err(Error::Transition);
        }
        Ok(())
    }

    pub async fn prepare_parts(
        &self,
        row: &Delivery,
        parts: &[crate::discord::native::Part],
    ) -> Result<(), Error> {
        sqlx::query("UPDATE discord_deliveries SET rendered_parts=? WHERE id=? AND state='pending' AND next_part=0 AND rendered_parts IS NULL")
            .bind(serde_json::to_string(parts).map_err(|_| Error::Database)?).bind(&row.id)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

#[path = "binding_store.rs"]
mod binding_store;
