use super::{Result, Service, error};
use crate::Api;
use crate::Input;
use crate::storage::Binding;
use engine::config::Config;
use engine::config::Workspace;
use engine::connections;
use engine::now;
use engine::reload::Snapshot;

impl Service {
    pub(super) fn workspace<'a>(&self, config: &'a Config, id: &str) -> Result<&'a Workspace> {
        config
            .workspaces
            .iter()
            .find(|w| w.id == id || w.short_name == id)
            .ok_or_else(|| "discord.error.workspace_missing".into())
    }

    pub(super) async fn fresh(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        write: bool,
    ) -> Result<()> {
        if *self.stop.borrow() {
            return Err("discord.error.stopping".into());
        }
        if self.config.snapshot().hash != snapshot.hash {
            return Err("discord.error.config_changed".into());
        }
        if write && !(0..=300).contains(&now().saturating_sub(input.date)) {
            return Err("discord.error.stale".into());
        }
        api.authorize(input).await.map_err(error)
    }

    pub(super) fn validate_binding<'a>(
        &self,
        config: &'a Config,
        b: &Binding,
    ) -> Result<&'a Workspace> {
        let ws = self.workspace(config, &b.workspace)?;
        let provider = config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .ok_or("discord.error.provider_removed")?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or("discord.error.host_removed")?;
        if b.instance != ws.provider_id
            || connections::require_workspace(&b.cwd, ws).is_err()
            || b.endpoint != connections::fingerprint(host, provider)
        {
            return Err("discord.error.scope_changed".into());
        }
        Ok(ws)
    }

    pub(super) async fn bound(&self, input: &Input) -> Result<Binding> {
        let b = self
            .store
            .discord_binding(input.bot, input.guild, input.channel)
            .await
            .map_err(error)?
            .ok_or("discord.error.unbound")?;
        if b.owner != input.user.to_string() {
            return Err("discord.error.owner_changed".into());
        }
        Ok(b)
    }
}
