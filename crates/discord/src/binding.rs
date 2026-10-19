use super::{Result, Service, error, hashes};
use crate::Api;
use crate::Input;
use crate::storage::Binding;
use engine::config::Workspace;
use engine::connections;
use engine::reload::Snapshot;
use session::AgentProvider;
use session::History;

impl Service {
    pub(super) async fn history(
        &self,
        provider: &dyn AgentProvider,
        ws: &Workspace,
        thread: &str,
    ) -> Result<History> {
        let limit = self.connections.history_limit(&ws.provider_id).await;
        let _permit = limit
            .acquire()
            .await
            .map_err(|_| i18n::Message::new("error.discord.backend"))?;
        provider
            .history(thread)
            .await
            .map(session::History::without_transient_errors)
            .map_err(error)
    }

    pub(super) async fn open(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &str,
        thread: &str,
    ) -> Result<String> {
        let locale = self.locale(input);
        let ws = self.workspace(&snapshot.config, workspace)?;
        let actor = self.connections.actor(&ws.provider_id, thread).await;
        let _guard = actor.lock().await;
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        let current = provider.read(thread).await.map_err(error)?;
        connections::require_workspace(&current.cwd, ws).map_err(error)?;
        let history = self.history(provider.as_ref(), ws, thread).await?;
        self.fresh(api, input, snapshot, true).await?;
        let category = match self
            .store
            .discord_reserve(input.bot, input.guild, &ws.id, None)
            .await
            .map_err(error)?
        {
            Some(id) => id,
            None => {
                let id = api
                    .category(&format!("PP · {}", ws.name), input.user)
                    .await
                    .map_err(error)?;
                self.store
                    .discord_category_receipt(input.bot, input.guild, &ws.id, id)
                    .await
                    .map_err(error)?;
                id
            }
        };
        self.fresh(api, input, snapshot, true).await?;
        let channel = match self
            .store
            .discord_reserve(
                input.bot,
                input.guild,
                &ws.id,
                Some((&ws.provider_id, thread)),
            )
            .await
            .map_err(error)?
        {
            Some(id) => id,
            None => api.channel(category, &current.title).await.map_err(error)?,
        };
        self.fresh(api, input, snapshot, true).await?;
        let provider = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .ok_or("discord.error.provider_removed")?;
        let host = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .and_then(|p| snapshot.config.hosts.iter().find(|h| h.id == p.host_id))
            .ok_or("discord.error.host_removed")?;
        let b = Binding {
            bot: input.bot.to_string(),
            guild: input.guild.to_string(),
            channel: channel.to_string(),
            owner: input.user.to_string(),
            workspace: ws.id.clone(),
            instance: ws.provider_id.clone(),
            thread: thread.into(),
            cwd: current.cwd,
            endpoint: connections::fingerprint(host, provider),
            active: true,
            revision: 1,
            observed_items: "{}".into(),
            history_ready: false,
            card_message: None,
            card_text: None,
        };
        self.store.discord_bind(&b).await.map_err(error)?;
        let b = self
            .store
            .discord_binding(input.bot, input.guild, channel)
            .await
            .map_err(error)?
            .ok_or("discord.error.binding_missing")?;
        if !b.history_ready {
            let text = crate::render::Conversation::latest(&history, &locale)
                .map_err(|_| i18n::Message::new("discord.history.too_large"))?;
            self.store
                .discord_publish(&b, "initial-history", &text, &hashes(&history))
                .await
                .map_err(error)?;
        }
        Ok(i18n::t!(
            "discord.session.added",
            locale = &locale,
            channel = channel,
            state = current.status.text_in(&locale)
        ))
    }
}
