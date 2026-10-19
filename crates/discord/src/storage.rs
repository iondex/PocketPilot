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

    pub async fn discord_publish(
        &self,
        b: &Binding,
        source: &str,
        text: &str,
        observed: &str,
    ) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(|_| Error::Database)?;
        let n=sqlx::query("UPDATE discord_bindings SET observed_items=?,history_ready=1 WHERE bot=? AND guild=? AND channel=? AND revision=? AND active=1 AND observed_items=?")
            .bind(observed).bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(b.revision).bind(&b.observed_items).execute(&mut *tx).await.map_err(|_|Error::Database)?.rows_affected();
        if n == 1 && !text.is_empty() {
            sqlx::query("INSERT INTO discord_deliveries(id,bot,guild,channel,source_key,revision,payload,state) VALUES(?,?,?,?,?,?,?,'pending') ON CONFLICT DO NOTHING")
                .bind(uuid::Uuid::now_v7().to_string()).bind(&b.bot).bind(&b.guild).bind(&b.channel).bind(source).bind(b.revision).bind(text)
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

    pub async fn prepare_parts(&self, row: &Delivery, parts: &[String]) -> Result<(), Error> {
        sqlx::query("UPDATE discord_deliveries SET rendered_parts=? WHERE id=? AND state='pending' AND next_part=0 AND rendered_parts IS NULL")
            .bind(serde_json::to_string(parts).map_err(|_| Error::Database)?).bind(&row.id)
            .execute(&self.pool).await.map_err(|_| Error::Database)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn provisioning_and_unknown_sends_are_not_replayed_and_containers_cannot_change_identity()
    {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        assert!(
            store
                .discord_reserve(1, 2, "ws", None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.discord_reserve(1, 2, "ws", None).await.is_err());
        store.discord_category_receipt(1, 2, "ws", 3).await.unwrap();
        assert_eq!(
            store.discord_reserve(1, 2, "ws", None).await.unwrap(),
            Some(3)
        );
        let mut b = Binding {
            bot: "1".into(),
            guild: "2".into(),
            channel: "4".into(),
            owner: "5".into(),
            workspace: "ws".into(),
            instance: "instance".into(),
            thread: "thread".into(),
            cwd: "/ws".into(),
            endpoint: "endpoint".into(),
            active: true,
            revision: 1,
            observed_items: "{}".into(),
            history_ready: false,
            card_message: None,
            card_text: None,
        };
        store.discord_bind(&b).await.unwrap();
        store
            .discord_publish(&b, "history", "visible history", "{\"item\":\"hash\"}")
            .await
            .unwrap();
        let out = store.discord_pending(1, 2, 0).await.unwrap();
        assert_eq!(out.len(), 1);
        store
            .discord_delivery_state(&out[0].id, "pending", "dispatching")
            .await
            .unwrap();
        let parts = vec!["first".to_string(), "second".to_string()];
        // Freeze the exact layout before a send; completed prefixes survive recovery.
        sqlx::query("UPDATE discord_deliveries SET rendered_parts=? WHERE id=?")
            .bind(serde_json::to_string(&parts).unwrap())
            .bind(&out[0].id)
            .execute(&store.pool)
            .await
            .unwrap();
        store.acknowledge_part(&out[0], true).await.unwrap();
        store.discord_recover().await.unwrap();
        let remaining = store
            .discord_pending(1, 2, engine::now() + 10)
            .await
            .unwrap();
        assert_eq!(remaining[0].next_part, 1);
        assert_eq!(
            remaining[0].rendered_parts.as_deref(),
            Some(serde_json::to_string(&parts).unwrap().as_str())
        );
        store
            .discord_delivery_state(&remaining[0].id, "pending", "dispatching")
            .await
            .unwrap();
        store.discord_recover().await.unwrap();
        assert!(store.discord_pending(1, 2, 0).await.unwrap().is_empty());
        b.thread = "other".into();
        assert!(matches!(
            store.discord_bind(&b).await,
            Err(Error::ContainerConflict)
        ));
    }
}
