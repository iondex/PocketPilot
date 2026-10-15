//! Opt-in Discord prototype, sharing native connections, session locks and write receipts.
use crate::{
    config::{Config, Workspace},
    connections::{self, Connections},
    discord_store::Binding,
    ledger::digest,
    reload::{ConfigFile, Snapshot},
    service::now,
    storage::Store,
};
use discord::{Api, Backend, Command, Input};
use session::{
    AgentProvider, History, ProviderError, RuntimeStatus,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet};

type Result<T> = std::result::Result<T, String>;
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct Service {
    config: Arc<ConfigFile>,
    store: Store,
    connections: Arc<Connections>,
    stop: watch::Receiver<bool>,
}
struct Target<'a> {
    workspace: &'a Workspace,
    thread: &'a str,
    binding: Option<&'a Binding>,
}

pub async fn run(
    config: Arc<ConfigFile>,
    store: Store,
    connections: Arc<Connections>,
    mut stop: watch::Receiver<bool>,
) -> std::result::Result<(), discord::Error> {
    let snapshot = config.snapshot();
    let settings = snapshot
        .config
        .discord
        .as_ref()
        .ok_or(discord::Error::Denied)?;
    let token = secrecy::SecretString::from(settings.token.clone());
    let api = Api::connect(&token, settings.guild_id).await?;
    store
        .discord_recover()
        .await
        .map_err(|_| discord::Error::Backend)?;
    let (shutdown, receiver) = watch::channel(false);
    let service = Arc::new(Service {
        config,
        store,
        connections,
        stop: receiver.clone(),
    });
    let mut jobs = JoinSet::new();
    jobs.spawn(discord::run(
        token,
        api.clone(),
        service.clone(),
        receiver.clone(),
    ));
    jobs.spawn(service.clone().observe(api.clone(), receiver.clone()));
    jobs.spawn(service.deliver(api, receiver));
    let result = tokio::select! {
        _=stop.changed()=>Ok(()),
        result=jobs.join_next()=>result.ok_or(discord::Error::Backend)?.map_err(|_|discord::Error::Backend)?,
    };
    // Let the gateway shut its shards down on the shared stop signal. Never sends
    // an agent lifecycle operation on exit, even if command futures are aborted.
    let _ = shutdown.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while jobs.join_next().await.is_some() {}
    })
    .await;
    jobs.abort_all();
    result
}

#[async_trait::async_trait]
impl Backend for Service {
    async fn handle(&self, api: &Api, input: Input) -> Result<String> {
        let snapshot = self.config.snapshot();
        let output = self.command(api, &input, &snapshot).await?;
        self.fresh(api, &input, &snapshot, false).await?;
        Ok(output)
    }
}

impl Service {
    #[cfg(test)]
    pub(crate) fn local_fixture(
        config: Arc<ConfigFile>,
        store: Store,
        connections: Arc<Connections>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            config,
            store,
            connections,
            stop,
        }
    }
    fn workspace<'a>(&self, config: &'a Config, id: &str) -> Result<&'a Workspace> {
        config
            .workspaces
            .iter()
            .find(|w| w.id == id || w.short_name == id)
            .ok_or_else(|| "工作区未配置。".into())
    }
    async fn fresh(
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
    fn validate_binding<'a>(&self, config: &'a Config, b: &Binding) -> Result<&'a Workspace> {
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
            || b.endpoint != connections::fingerprint(host)
        {
            return Err("映射范围已改变，请重新添加会话。".into());
        }
        Ok(ws)
    }
    async fn bound(&self, input: &Input) -> Result<Binding> {
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
    async fn mutate(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        key: &str,
        target: Target<'_>,
        mutation: Mutation,
    ) -> Result<Receipt> {
        let Target {
            workspace: ws,
            thread,
            binding,
        } = target;
        self.fresh(api, input, snapshot, true).await?;
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        if !provider.capabilities().control_verified {
            return Err("此实例尚未完成共享/断线验证，当前只读。".into());
        }
        let id = self
            .store
            .reserve_operation(
                &format!("discord:{}:{}:{key}", input.bot, input.id),
                &ws.provider_id,
                thread,
            )
            .await
            .map_err(error)?;
        let state = self.store.operation_state(&id).await.map_err(error)?;
        if state == State::Accepted {
            return self
                .store
                .receipt(&id)
                .await
                .map_err(error)?
                .ok_or("回执缺失，未重发。".into());
        }
        if state != State::Received {
            return Err("先前请求结果未知，未重发，请核对原客户端。".into());
        }
        self.store
            .transition(&id, State::Received, State::Prepared)
            .await
            .map_err(error)?;
        self.fresh(api, input, snapshot, true).await?;
        if let Some(b) = binding {
            let current = self.bound(input).await?;
            if current.revision != b.revision
                || current.thread != thread
                || current.instance != ws.provider_id
            {
                return Err("映射已更新，请重新发送。".into());
            }
        }
        self.store
            .transition(&id, State::Prepared, State::Dispatching)
            .await
            .map_err(error)?;
        match provider.execute(mutation).await {
            Ok(receipt) => {
                self.store
                    .accept_receipt(&id, &receipt)
                    .await
                    .map_err(error)?;
                Ok(receipt)
            }
            Err(e) => {
                let rejected = matches!(
                    e,
                    ProviderError::Rejected(_)
                        | ProviderError::Conflict
                        | ProviderError::Unsupported
                );
                self.store
                    .transition(
                        &id,
                        State::Dispatching,
                        if rejected {
                            State::Rejected
                        } else {
                            State::OutcomeUnknown
                        },
                    )
                    .await
                    .map_err(error)?;
                if rejected {
                    Err(error(e))
                } else {
                    Err("请求结果未知，未重发，请核对原客户端。".into())
                }
            }
        }
    }
    async fn command(&self, api: &Api, input: &Input, snapshot: &Snapshot) -> Result<String> {
        match &input.command {
            Command::Workspaces => Ok(snapshot
                .config
                .workspaces
                .iter()
                .map(|w| format!("{} · {}", w.short_name, w.name))
                .collect::<Vec<_>>()
                .join("\n")),
            Command::Sessions { workspace, cursor } => {
                let ws = self.workspace(&snapshot.config, workspace)?;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let page = connections::list_workspace(provider.as_ref(), ws, cursor.clone(), 8)
                    .await
                    .map_err(error)?;
                let mut text = page
                    .data
                    .iter()
                    .map(|s| {
                        format!(
                            "{} · {}\n{}",
                            s.status.label(),
                            session::delivery::short_title(&s.title, 45),
                            s.reference.thread
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                text.push_str(&format!(
                    "\n\n添加：/pp-open workspace:{} thread:ID",
                    ws.short_name
                ));
                if let Some(cursor) = page.next_cursor {
                    text.push_str(&format!("\n下一页 cursor：{cursor}"));
                }
                Ok(text)
            }
            Command::Open { workspace, thread } => {
                self.open(api, input, snapshot, workspace, thread).await
            }
            Command::New {
                workspace,
                text,
                confirmed,
            } => {
                if !confirmed || text.trim().is_empty() {
                    return Err("创建需要 confirm:true 和首条输入。".into());
                }
                let ws = self.workspace(&snapshot.config, workspace)?;
                let actor = self.connections.actor(&ws.provider_id, "__create").await;
                let _guard = actor.lock().await;
                let receipt = self
                    .mutate(
                        api,
                        input,
                        snapshot,
                        "create",
                        Target {
                            workspace: ws,
                            thread: "__new",
                            binding: None,
                        },
                        Mutation::Create {
                            cwd: ws.cwd.clone(),
                        },
                    )
                    .await?;
                let actor = self
                    .connections
                    .actor(&ws.provider_id, &receipt.thread)
                    .await;
                {
                    let _guard = actor.lock().await;
                    self.mutate(
                        api,
                        input,
                        snapshot,
                        "first-turn",
                        Target {
                            workspace: ws,
                            thread: &receipt.thread,
                            binding: None,
                        },
                        Mutation::Start {
                            thread: receipt.thread.clone(),
                            text: text.clone(),
                            client_id: format!("pp:discord:{}:{}", input.bot, input.id),
                        },
                    )
                    .await?;
                }
                self.open(api, input, snapshot, workspace, &receipt.thread)
                    .await
            }
            Command::Ask { text } => {
                if text.trim().is_empty() {
                    return Err("请输入消息内容。".into());
                }
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let actor = self.connections.actor(&b.instance, &b.thread).await;
                let _guard = actor.lock().await;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let mut current = provider.read(&b.thread).await.map_err(error)?;
                connections::require_workspace(&current.cwd, ws).map_err(error)?;
                if current.status == RuntimeStatus::NotLoaded {
                    self.mutate(
                        api,
                        input,
                        snapshot,
                        "resume",
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        Mutation::Resume {
                            thread: b.thread.clone(),
                        },
                    )
                    .await?;
                    current = provider.read(&b.thread).await.map_err(error)?;
                }
                let client_id = format!("pp:discord:{}:{}", input.bot, input.id);
                let mutation = match current.status {
                    RuntimeStatus::Idle => Mutation::Start {
                        thread: b.thread.clone(),
                        text: text.clone(),
                        client_id,
                    },
                    RuntimeStatus::Active { .. } => Mutation::Steer {
                        thread: b.thread.clone(),
                        expected_turn: match current.active_turn {
                            Some(id) => id,
                            None => provider
                                .active_turn(&b.thread)
                                .await
                                .map_err(error)?
                                .ok_or("活跃轮尚不可读，请稍后重新确认。")?,
                        },
                        text: text.clone(),
                        client_id,
                    },
                    _ => return Err("当前状态不允许发送，请刷新 /pp-status。".into()),
                };
                let receipt = self
                    .mutate(
                        api,
                        input,
                        snapshot,
                        "input",
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        mutation,
                    )
                    .await?;
                Ok(format!(
                    "已接受 · turn {}",
                    receipt.turn.unwrap_or_default()
                ))
            }
            Command::Unbind => {
                let b = self.bound(input).await?;
                self.fresh(api, input, snapshot, true).await?;
                self.store.discord_unbind(&b).await.map_err(error)?;
                Ok("已解除 channel 映射，会话继续运行。".into())
            }
            Command::Rename { title } => {
                let b = self.bound(input).await?;
                self.validate_binding(&snapshot.config, &b)?;
                self.fresh(api, input, snapshot, true).await?;
                api.rename(input.channel, title, &b.thread)
                    .await
                    .map_err(error)?;
                Ok("channel 显示名已更新。".into())
            }
            Command::Status | Command::History => {
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let current = provider.read(&b.thread).await.map_err(error)?;
                connections::require_workspace(&current.cwd, ws).map_err(error)?;
                if matches!(input.command, Command::History) {
                    Ok(session::delivery::full_history(
                        &self.history(provider.as_ref(), ws, &b.thread).await?,
                    ))
                } else {
                    Ok(format!(
                        "{}\n状态：{}\n工作区：{}\nthread：{}",
                        current.title,
                        current.status.label(),
                        ws.name,
                        b.thread
                    ))
                }
            }
        }
    }
    async fn history(
        &self,
        provider: &dyn AgentProvider,
        ws: &Workspace,
        thread: &str,
    ) -> Result<History> {
        let limit = self.connections.history_limit(&ws.provider_id).await;
        let _permit = limit.acquire().await.map_err(error)?;
        provider.history(thread).await.map_err(error)
    }
    async fn open(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &str,
        thread: &str,
    ) -> Result<String> {
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
            None => api
                .channel(category, &current.title, thread)
                .await
                .map_err(error)?,
        };
        self.fresh(api, input, snapshot, true).await?;
        let host = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .and_then(|p| snapshot.config.hosts.iter().find(|h| h.id == p.host_id))
            .ok_or("机器已移除。")?;
        let b = Binding {
            bot: input.bot.to_string(),
            guild: input.guild.to_string(),
            channel: channel.to_string(),
            owner: input.user.to_string(),
            workspace: ws.id.clone(),
            instance: ws.provider_id.clone(),
            thread: thread.into(),
            cwd: current.cwd,
            endpoint: connections::fingerprint(host),
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
            .ok_or("映射未保存。")?;
        if !b.history_ready {
            let text = if matches!(current.status, RuntimeStatus::Active { .. }) {
                session::delivery::full_history(&history)
            } else {
                session::delivery::latest_interaction(&history)
            };
            self.store
                .discord_publish(&b, "initial-history", &text, &hashes(&history))
                .await
                .map_err(error)?;
        }
        Ok(format!(
            "已添加 <#{channel}> · {}\n在该 channel 使用 /pp-ask text:内容。",
            current.status.label()
        ))
    }
    async fn refresh(&self, api: &Api, b: &Binding) -> Result<()> {
        let snapshot = self.config.snapshot();
        let ws = self.validate_binding(&snapshot.config, b)?;
        let channel = b.channel.parse::<u64>().map_err(error)?;
        let input = Input {
            id: 0,
            bot: api.bot,
            guild: api.guild,
            channel,
            user: b.owner.parse().map_err(error)?,
            date: now(),
            command: Command::Status,
        };
        self.fresh(api, &input, &snapshot, false).await?;
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        let current = provider.read(&b.thread).await.map_err(error)?;
        connections::require_workspace(&current.cwd, ws).map_err(error)?;
        let history = self.history(provider.as_ref(), ws, &b.thread).await?;
        self.fresh(api, &input, &snapshot, false).await?;
        let current_binding = self.bound(&input).await?;
        if current_binding.revision != b.revision {
            return Ok(());
        }
        let old: BTreeMap<String, String> =
            serde_json::from_str(&b.observed_items).map_err(error)?;
        let fresh = hashes(&history);
        let text = if !b.history_ready {
            if matches!(current.status, RuntimeStatus::Active { .. }) {
                session::delivery::full_history(&history)
            } else {
                session::delivery::latest_interaction(&history)
            }
        } else {
            history
                .items
                .iter()
                .filter(|i| old.get(&item_key(i)) != Some(&digest(&i.text)))
                .map(|i| {
                    format!(
                        "{}\n{}",
                        if i.role == session::Role::User {
                            "用户"
                        } else {
                            "Agent"
                        },
                        i.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n────────\n\n")
        };
        if !b.history_ready || !text.is_empty() {
            self.store
                .discord_publish(b, &format!("items:{}", digest(&fresh)), &text, &fresh)
                .await
                .map_err(error)?;
        }
        let card = format!(
            "{} · {}\nthread：{}",
            current.status.label(),
            session::delivery::short_title(&current.title, 80),
            b.thread
        );
        self.update_card(api, b, &card).await
    }
    async fn update_card(&self, api: &Api, b: &Binding, text: &str) -> Result<()> {
        if b.card_text.as_deref() == Some(text) {
            return Ok(());
        }
        let channel = b.channel.parse().map_err(error)?;
        if let Some(id) = &b.card_message {
            if let Ok(id) = id.parse::<u64>()
                && id != 0
            {
                api.card(channel, id, text).await.map_err(error)?;
                self.store
                    .discord_card(b, Some(id), text)
                    .await
                    .map_err(error)?;
            }
        } else {
            // Zero is an unknown-send tombstone, never a valid Discord message ID.
            self.store
                .discord_card(b, Some(0), "状态卡发送中；若结果未知请使用 /pp-status")
                .await
                .map_err(error)?;
            let id = api.send(channel, text).await.map_err(error)?;
            self.store
                .discord_card(b, Some(id), text)
                .await
                .map_err(error)?;
        }
        Ok(())
    }
    async fn observe(
        self: Arc<Self>,
        api: Api,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), discord::Error> {
        let mut jobs = JoinSet::new();
        let mut busy = HashSet::new();
        loop {
            for b in self
                .store
                .discord_bindings(api.bot, api.guild)
                .await
                .map_err(|_| discord::Error::Backend)?
            {
                if jobs.len() >= 8 || !busy.insert(b.channel.clone()) {
                    continue;
                }
                let service = self.clone();
                let api = api.clone();
                jobs.spawn(async move {
                    if service.refresh(&api, &b).await.is_err() {
                        tracing::warn!("Discord observation unavailable; session unchanged");
                        // Only a validated, still-owned binding may receive a stale marker.
                        let snapshot = service.config.snapshot();
                        let input = Input {
                            id: 0,
                            bot: api.bot,
                            guild: api.guild,
                            channel: b.channel.parse().unwrap_or(0),
                            user: b.owner.parse().unwrap_or(0),
                            date: now(),
                            command: Command::Status,
                        };
                        if service.validate_binding(&snapshot.config, &b).is_ok()
                            && service.bound(&input).await.is_ok()
                            && service.fresh(&api, &input, &snapshot, false).await.is_ok()
                        {
                            let _ = service
                                .update_card(&api, &b, "连接未知 · 请刷新 /pp-status")
                                .await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    b.channel
                });
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next(),if !jobs.is_empty()=>{match result {Some(Ok(channel))=>{busy.remove(&channel);},_=>return Err(discord::Error::Backend)}},
                _=tokio::time::sleep(Duration::from_secs(1))=>{},
            }
        }
        jobs.abort_all();
        Ok(())
    }
    async fn deliver(
        self: Arc<Self>,
        api: Api,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), discord::Error> {
        loop {
            let rows = self
                .store
                .discord_pending(api.bot, api.guild, now())
                .await
                .map_err(|_| discord::Error::Backend)?;
            for row in rows.into_iter().take(1) {
                let mut input = Input {
                    id: 0,
                    bot: api.bot,
                    guild: api.guild,
                    channel: row.channel.parse().map_err(|_| discord::Error::Backend)?,
                    user: 0,
                    date: now(),
                    command: Command::Status,
                };
                let snapshot = self.config.snapshot();
                let binding = self
                    .store
                    .discord_binding(api.bot, api.guild, input.channel)
                    .await
                    .map_err(|_| discord::Error::Backend)?;
                let valid = if let Some(b) = binding {
                    input.user = b.owner.parse().unwrap_or(0);
                    b.revision == row.revision
                        && self.validate_binding(&snapshot.config, &b).is_ok()
                } else {
                    false
                };
                if !valid {
                    self.store
                        .discord_delivery_state(&row.id, "pending", "rejected")
                        .await
                        .map_err(|_| discord::Error::Backend)?;
                    continue;
                }
                match api.authorize(&input).await {
                    Ok(()) => {}
                    Err(discord::Error::Denied) => {
                        self.store
                            .discord_delivery_state(&row.id, "pending", "rejected")
                            .await
                            .map_err(|_| discord::Error::Backend)?;
                        continue;
                    }
                    Err(_) => {
                        // A failed read of ownership is not a revocation. Retain
                        // the unsent bundle and recheck when Discord is reachable.
                        tracing::warn!(
                            "Discord owner verification unavailable; output remains pending"
                        );
                        continue;
                    }
                }
                if *self.stop.borrow() {
                    return Ok(());
                }
                if self.config.snapshot().hash != snapshot.hash {
                    continue;
                }
                let current = self
                    .store
                    .discord_binding(api.bot, api.guild, input.channel)
                    .await
                    .map_err(|_| discord::Error::Backend)?;
                if current.is_none_or(|b| b.revision != row.revision) {
                    self.store
                        .discord_delivery_state(&row.id, "pending", "rejected")
                        .await
                        .map_err(|_| discord::Error::Backend)?;
                    continue;
                }
                self.store
                    .discord_delivery_state(&row.id, "pending", "dispatching")
                    .await
                    .map_err(|_| discord::Error::Backend)?;
                let sent = api.send(input.channel, &row.payload).await.is_ok();
                self.store
                    .discord_delivery_state(
                        &row.id,
                        "dispatching",
                        if sent { "delivered" } else { "outcome_unknown" },
                    )
                    .await
                    .map_err(|_| discord::Error::Backend)?;
            }
            tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(4))=>{}}
        }
        Ok(())
    }
}
fn item_key(i: &session::TextItem) -> String {
    format!("{}:{}{}", i.turn_id.len(), i.turn_id, i.item_id)
}
fn hashes(history: &History) -> String {
    serde_json::to_string(
        &history
            .items
            .iter()
            .map(|i| (item_key(i), digest(&i.text)))
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap_or_else(|_| "{}".into())
}
