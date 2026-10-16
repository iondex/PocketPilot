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
            .ok_or_else(|| "工作区未配置。".into())
    }

    pub(super) async fn fresh(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        write: bool,
    ) -> Result<()> {
        if *self.stop.borrow() {
            return Err("桥接正在退出，未发送新请求。".into());
        }
        if self.config.snapshot().hash != snapshot.hash {
            return Err("配置已更新，请重新确认。".into());
        }
        if write && !(0..=300).contains(&now().saturating_sub(input.date)) {
            return Err("输入已过期，请重新发送。".into());
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
            .ok_or("执行实例已移除。")?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or("机器已移除。")?;
        if b.instance != ws.provider_id
            || connections::require_workspace(&b.cwd, ws).is_err()
            || b.endpoint != connections::fingerprint(host, provider)
        {
            return Err("映射范围已改变，请重新添加会话。".into());
        }
        Ok(ws)
    }

    pub(super) async fn bound(&self, input: &Input) -> Result<Binding> {
        let b = self
            .store
            .discord_binding(input.bot, input.guild, input.channel)
            .await
            .map_err(error)?
            .ok_or("当前 channel 尚未绑定会话。")?;
        if b.owner != input.user.to_string() {
            return Err("此 channel 的 owner 已改变，请重新添加。".into());
        }
        Ok(b)
    }
}
