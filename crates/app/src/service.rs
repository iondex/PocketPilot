use crate::{
    config::{Config, Workspace},
    connections::{self, Connections},
    credentials,
    ledger::{Binding, Outbound, Scope, digest},
    reload::{ConfigFile, Snapshot},
    storage::{Store, StoreError},
};
use serde::{Deserialize, Serialize};
use session::{
    AgentProvider, History, ProviderError, RuntimeStatus,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use telegram::api::{Api, Button, Input};
use tokio::{
    sync::{Mutex, Semaphore, watch},
    task::JoinSet,
    time::Instant,
};
#[path = "management.rs"]
mod management;
#[path = "navigation.rs"]
mod navigation;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configure a Telegram or Discord bot in the root TOML")]
    PlatformsMissing,
    #[error("Discord requires a build with --features discord")]
    DiscordFeature,
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Connection(#[from] connections::Error),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Telegram(#[from] telegram::Error),
    #[error("Telegram 已配置 webhook；请先自行确认接收方式，PocketPilot 未修改 webhook")]
    Webhook,
    #[error("按钮已过期或配置已更新，请重新打开菜单")]
    Expired,
    #[error("当前 topic 尚未绑定会话；请使用 /workspaces")]
    Unbound,
    #[error("此执行实例尚未通过共享和断线验证，当前只读")]
    ReadOnly,
    #[error("请求结果未知，没有自动重发；请核对会话状态")]
    Unknown,
    #[error("输入已过期，请刷新状态后重新发送")]
    Stale,
    #[error("请在 BotFather 开启此 bot 的私聊 Topics 模式，再添加会话")]
    TopicsDisabled,
    #[error("a bridge worker stopped unexpectedly; pending writes were not replayed")]
    WorkerStopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Home,
    Host {
        id: String,
    },
    NewPrompt {
        workspace: String,
    },
    Workspace {
        id: String,
        cursor: Option<String>,
    },
    Open {
        workspace: String,
        thread: String,
    },
    Create {
        workspace: String,
        text: String,
    },
    SubmitDraft {
        text: String,
        steer_only: bool,
        instance: String,
        thread: String,
        revision: i64,
    },
}

pub struct Service {
    pub config: Arc<ConfigFile>,
    pub store: Store,
    pub connections: Arc<Connections>,
    api: Api,
    bot: i64,
    topics_enabled: bool,
    pair_attempts: Mutex<HashMap<(u64, bool), Instant>>,
    history_slots: Semaphore,
    started_at: i64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub async fn run(path: PathBuf) -> Result<(), Error> {
    // Telegram timestamps have one-second precision. Treat the start second
    // conservatively so a pre-start message from that second cannot be replayed.
    let started_at = now().saturating_add(1);
    let config = Arc::new(ConfigFile::open(path)?);
    let snapshot = config.snapshot();
    if snapshot.config.telegram.is_none() && snapshot.config.discord.is_none() {
        return Err(Error::PlatformsMissing);
    }
    #[cfg(not(feature = "discord"))]
    if snapshot.config.discord.is_some() {
        return Err(Error::DiscordFeature);
    }
    let telegram = if let Some(settings) = &snapshot.config.telegram {
        let api = Api::new(credentials::read(settings)?)?;
        let identity = api.identity().await?;
        if identity.webhook_enabled {
            return Err(Error::Webhook);
        }
        Some((api, identity))
    } else {
        None
    };
    let store = Store::open(&snapshot.config.service.data_dir).await?;
    let connections = Arc::new(Connections::new(store.clone()));
    let (stop, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    tasks.spawn(config.clone().watch(receiver.clone()));
    if let Some((api, identity)) = telegram {
        let bot = i64::try_from(identity.bot_id).map_err(|_| Error::ReadOnly)?;
        if identity.user_topic_creation_enabled {
            tracing::warn!(
                "Telegram clients can auto-create topics for /start; enable Disallow users to create new threads in BotFather to keep a fixed management entrance"
            );
        }
        store.recover_inbox(bot).await?;
        let service = Arc::new(Service {
            config: config.clone(),
            store: store.clone(),
            connections: connections.clone(),
            api,
            bot,
            topics_enabled: identity.topics_enabled,
            pair_attempts: Mutex::new(HashMap::new()),
            history_slots: Semaphore::new(4),
            started_at,
        });
        tasks.spawn(service.clone().poll(receiver.clone()));
        tasks.spawn(service.clone().work(receiver.clone()));
        tasks.spawn(service.clone().deliver(receiver.clone()));
        tasks.spawn(service.observe(receiver.clone()));
        tracing::info!(
            bot_id = bot,
            private_topics = identity.topics_enabled,
            "Telegram ready; session lifecycle remains owned by the app-server"
        );
    }
    #[cfg(feature = "discord")]
    if snapshot.config.discord.is_some() {
        let config = config.clone();
        let receiver = receiver.clone();
        tasks.spawn(async move {
            if let Err(error) =
                crate::discord_service::run(config, store, connections, receiver).await
            {
                tracing::error!(%error,"Discord prototype worker exited");
            }
        });
    }
    let failed = tokio::select! {
        _=shutdown_signal()=>false,
        exit=tasks.join_next()=>{ tracing::error!(normal=matches!(exit,Some(Ok(()))),"a service task exited; stopping bridge transports"); true },
    };
    let _ = stop.send(true);
    // Dropping clients closes only proxy connections. No unsubscribe, interrupt,
    // remote daemon command, or unfinished input replay is issued during shutdown.
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    if failed {
        Err(Error::WorkerStopped)
    } else {
        Ok(())
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=signal.recv()=>{} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

impl Service {
    async fn poll(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut failures = 0_u32;
        loop {
            let cursor = match self.store.cursor(self.bot, now()).await {
                Ok(cursor) => cursor,
                Err(_) => {
                    tracing::error!("poll cursor unavailable");
                    break;
                }
            };
            let result = tokio::select! { _=stop.changed()=>break, result=self.api.updates(cursor.1)=>result };
            match result {
                Ok(updates) => {
                    let updates = updates
                        .iter()
                        .map(|u| {
                            let mut input = telegram::api::input(u);
                            if failures > 0
                                && let Some(input) = input.as_mut()
                            {
                                input.requires_confirmation = true;
                            }
                            (u.id.0, input)
                        })
                        .collect::<Vec<_>>();
                    if self
                        .store
                        .ingest(self.bot, cursor.0, &updates, now())
                        .await
                        .is_err()
                    {
                        tracing::error!(
                            "inbox persistence failed; fetched updates not acknowledged"
                        );
                        break;
                    }
                    failures = 0;
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let delay = match error {
                        telegram::Error::RetryAfter(s) => s,
                        _ => 2_u64.pow(failures.min(6)),
                    };
                    tracing::warn!(%error,"Telegram poll failed; retrying reads only");
                    tokio::select! { _=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(delay))=>{} }
                }
            }
        }
    }

    async fn work(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut jobs = JoinSet::new();
        let mut busy = std::collections::HashSet::new();
        loop {
            if jobs.len() < 16 {
                let peers = busy.iter().copied().collect::<Vec<_>>();
                match self.store.claim_available_input(self.bot, &peers).await {
                    Ok(Some(inbox)) => {
                        let key = (inbox.input.chat, inbox.input.topic);
                        busy.insert(key);
                        let service = self.clone();
                        jobs.spawn(async move {
                            let input = &inbox.input;
                            if let Some(id) = &input.callback_id {
                                let _ = service.api.answer_callback(id).await;
                            }
                            let result = service.handle(input).await;
                            if let Err(error) = result {
                                tracing::warn!(%error,"input failed without replay");
                                let mut response_input = input.clone();
                                if service
                                    .store
                                    .binding(service.bot, input.chat, input.topic)
                                    .await
                                    .ok()
                                    .flatten()
                                    .is_none()
                                    && let Ok(Some(topic)) = service
                                        .store
                                        .management_topic(service.bot, input.chat)
                                        .await
                                {
                                    response_input.topic = topic;
                                }
                                let _ = service
                                    .respond(
                                        &response_input,
                                        &format!("error:{}", input.event_key),
                                        Outbound::text(error.user_message()),
                                    )
                                    .await;
                            }
                            let _ = service
                                .store
                                .finish_input(service.bot, inbox.epoch, inbox.id)
                                .await;
                            key
                        });
                        continue;
                    }
                    Ok(None) => {}
                    Err(_) => {
                        tracing::error!("inbox unavailable");
                        break;
                    }
                }
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next(), if !jobs.is_empty()=>{
                    match result {
                        Some(Ok(key))=>{busy.remove(&key);},
                        Some(Err(_))=>{tracing::error!("input worker failed; pending request will be marked uncertain on restart");break;},
                        None=>{},
                    }
                },
                _=tokio::time::sleep(Duration::from_millis(200))=>{},
            }
        }
        jobs.abort_all();
    }

    async fn handle(&self, input: &Input) -> Result<(), Error> {
        if telegram::access(input.user, input.chat, true, false) == telegram::Access::Denied {
            return Ok(());
        }
        let paired = self
            .store
            .pairing()
            .is_paired(self.bot, input.user, input.chat)
            .await
            .map_err(|_| StoreError::Database)?;
        if !paired {
            let starting = input.text.split_whitespace().next() == Some("/start");
            let mut attempts = self.pair_attempts.lock().await;
            let key = (input.user, starting);
            if attempts
                .get(&key)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
            {
                return Ok(());
            }
            attempts.retain(|_, at| at.elapsed() < Duration::from_secs(600));
            if attempts.len() >= 4096 {
                return Ok(());
            }
            attempts.insert(key, Instant::now());
            drop(attempts);
            let mut body = Outbound::text(
                "请先发送 /start 申请配对，并在运行 PocketPilot 的机器上用 CLI 批准。",
            );
            body.require_pairing = false;
            if starting {
                let code = self
                    .store
                    .pairing()
                    .request_pairing(self.bot, input.user, input.chat, true, now())
                    .await
                    .map_err(|_| StoreError::Database)?;
                body.text = format!(
                    "配对码（10 分钟有效）\n{code}\n\n请在运行 PocketPilot 的机器上执行：\npocketpilot pair approve {code}\n\n批准后再次发送 /start。"
                );
                body.expires_at = Some(now() + 600);
            }
            return self
                .store
                .enqueue(self.bot, input.chat, input.topic, &input.event_key, &body)
                .await
                .map_err(Into::into);
        }
        let snapshot = self.config.snapshot();
        let mut scoped = input.clone();
        scoped.config_hash = Some(snapshot.hash.clone());
        let input = &scoped;
        if input.callback.is_some() {
            if input.requires_confirmation {
                let mut routed = input.clone();
                routed.callback = None;
                routed.callback_id = None;
                let routed = self.management_input(&routed, &snapshot).await?;
                return self.workspaces(&routed, &snapshot).await;
            }
            let body = self
                .store
                .consume_callback(self.bot, input, &snapshot.hash, now())
                .await?;
            let Some(body) = body else {
                let mut routed = input.clone();
                routed.callback = None;
                routed.callback_id = None;
                let routed = self.management_input(&routed, &snapshot).await?;
                return self.workspaces(&routed, &snapshot).await;
            };
            let action: Action = serde_json::from_str(&body).map_err(|_| Error::Expired)?;
            return match action {
                Action::Home => self.workspaces(input, &snapshot).await,
                Action::Host { id } => self.host_workspaces(input, &snapshot, &id).await,
                Action::NewPrompt { .. } => self.workspaces(input, &snapshot).await,
                Action::Workspace { id, cursor } => {
                    self.sessions(input, &snapshot, &id, cursor).await
                }
                Action::Open { workspace, thread } => {
                    self.open(input, &snapshot, &workspace, &thread).await
                }
                Action::Create { workspace, text } => {
                    self.create(input, &snapshot, &workspace, &text).await
                }
                Action::SubmitDraft {
                    text,
                    steer_only,
                    instance,
                    thread,
                    revision,
                } => {
                    let binding = self.bound(input).await?;
                    if binding.instance_id != instance
                        || binding.thread_id != thread
                        || binding.revision != revision
                    {
                        return Err(Error::Expired);
                    }
                    self.submit(input, &snapshot, &text, steer_only).await
                }
            };
        }
        let (command, args) = input
            .text
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((input.text.trim(), ""));
        let command = command.split('@').next().unwrap_or(command);
        let management;
        let input = if matches!(
            command,
            "/start"
                | "/workspaces"
                | "/sessions"
                | "/open"
                | "/new"
                | "/help"
                | "/refresh"
                | "/back"
        ) {
            management = self.management_input(input, &snapshot).await?;
            &management
        } else {
            input
        };
        if self
            .store
            .binding(self.bot, input.chat, input.topic)
            .await?
            .is_none()
            && !matches!(
                command,
                "/start"
                    | "/workspaces"
                    | "/sessions"
                    | "/open"
                    | "/new"
                    | "/help"
                    | "/refresh"
                    | "/back"
            )
        {
            return self.default_message(input, &snapshot).await;
        }
        match command {
            "/start"|"/workspaces"=>self.workspaces(input,&snapshot).await,
            "/refresh"|"/back"=>self.select_navigation(input,&snapshot).await,
            "/help"=>self.respond(input,&input.event_key,Outbound::text("回复序号选择主机、工作区和会话。\n0 返回 · /start 首页 · /refresh 刷新\n/new 工作区 任务 新建会话\n\n在会话 topic 直接发送任务。\n/status 状态 · /history 历史\n/rename 名称 · /unbind 解除关联")).await,
            "/sessions"=>self.sessions(input,&snapshot,args.trim(),None).await,
            "/open"=>{
                let (workspace,thread)=args.trim().split_once(char::is_whitespace).ok_or(Error::Unbound)?;
                self.open(input,&snapshot,workspace,thread.trim()).await
            }
            "/new"=>{
                let (workspace,text)=args.trim().split_once(char::is_whitespace).ok_or(Error::Unbound)?;
                let workspace=self.workspace(&snapshot.config,workspace)?;
                if text.trim().is_empty() { return Err(Error::Stale); }
                self.confirm_creation(input,&snapshot,&workspace.id,text.trim()).await
            }
            "/status"|"/info"|"/history"=>{
                let binding=self.bound(input).await?;
                let workspace=self.binding_workspace(&snapshot.config,&binding)?;
                let provider=self.connections.get(&snapshot.config,workspace).await?;
                let current=provider.read(&binding.thread_id).await?;
                connections::require_workspace(&current.cwd,workspace)?;
                tracing::info!(thread_id=%current.reference.thread,workspace_id=%workspace.id,status=?current.status,"session details requested");
                let text=if command=="/history" {
                    let text=session::delivery::agent_history(&self.history(provider.as_ref(),workspace,&binding.thread_id,&snapshot).await?);
                    if text.is_empty(){"暂无回复。".into()}else{text}
                }else {format!("{}\n{}",session::delivery::short_title(&current.title,40),current.status.label())};
                self.respond(input,&input.event_key,Outbound::text(text)).await
            }
            "/unbind"=>{
                self.bound(input).await?;
                self.store.unbind(self.bot,input.chat,input.topic).await?;
                self.respond(input,&input.event_key,Outbound::text("已解除此 topic 的映射，会话继续运行。此 topic 只能重新连接原会话。" )).await
            }
            "/rename"=>{
                let binding=self.bound(input).await?;
                let workspace=self.binding_workspace(&snapshot.config,&binding)?;
                let title=session::delivery::short_title(args,40);
                self.store.rename(self.bot,input.chat,input.topic,&title).await?;
                let mut body=Outbound::text(format!("显示名已设为 {title}"));
                body.topic_title=Some(topic_title(workspace,&title,&binding.thread_id));
                self.respond(input,&input.event_key,body).await
            }
            "/ask"|"/steer"=>{
                if self.delayed(input) {self.draft(input,&snapshot,args.trim(),command=="/steer").await}
                else {self.submit(input,&snapshot,args.trim(),command=="/steer").await}
            },
            text if text.starts_with('/')=>self.respond(input,&input.event_key,Outbound::text("未知命令。使用 /help；如需把斜杠文本发送给 agent，使用 /ask 文本。")).await,
            _=>{
                if self.delayed(input) {self.draft(input,&snapshot,input.text.trim(),false).await}
                else {self.submit(input,&snapshot,input.text.trim(),false).await}
            },
        }
    }

    fn delayed(&self, input: &Input) -> bool {
        input.requires_confirmation
            || input.date < self.started_at
            || !(0..=300).contains(&now().saturating_sub(input.date))
    }
    async fn draft(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        text: &str,
        steer_only: bool,
    ) -> Result<(), Error> {
        if text.trim().is_empty() {
            return Err(Error::Stale);
        }
        let binding = self.bound(input).await?;
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        let button = self
            .button(
                input,
                snapshot,
                "确认发送草稿",
                Action::SubmitDraft {
                    text: text.into(),
                    steer_only,
                    instance: binding.instance_id,
                    thread: binding.thread_id.clone(),
                    revision: binding.revision,
                },
            )
            .await?;
        tracing::info!(workspace_id=%workspace.id,thread_id=%binding.thread_id,"session input requires renewed confirmation");
        let mut body = Outbound::text(format!(
            "⚠️ 这条消息尚未发送\n\n{}\n\n核对会话后确认发送。",
            session::delivery::short_title(text, 80)
        ));
        body.buttons = vec![vec![button]];
        self.respond(input, &format!("draft:{}", input.event_key), body)
            .await
    }

    fn workspace<'a>(&self, config: &'a Config, id: &str) -> Result<&'a Workspace, Error> {
        config
            .workspaces
            .iter()
            .find(|w| w.id == id || w.short_name == id)
            .ok_or(connections::Error::Selection.into())
    }
    fn binding_workspace<'a>(
        &self,
        config: &'a Config,
        binding: &Binding,
    ) -> Result<&'a Workspace, Error> {
        let workspace = self.workspace(
            config,
            binding.workspace_id.as_deref().ok_or(Error::Unbound)?,
        )?;
        if workspace.provider_id != binding.instance_id {
            return Err(connections::Error::Identity.into());
        }
        Ok(workspace)
    }
    async fn bound(&self, input: &Input) -> Result<Binding, Error> {
        self.store
            .binding(self.bot, input.chat, input.topic)
            .await?
            .ok_or(Error::Unbound)
    }
    async fn button(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        label: &str,
        action: Action,
    ) -> Result<Button, Error> {
        let body = serde_json::to_string(&action).map_err(|_| Error::Expired)?;
        let data = self
            .store
            .callback(self.bot, input, &snapshot.hash, &body, now())
            .await?;
        Ok(Button {
            label: label.into(),
            data,
        })
    }
    async fn respond(&self, input: &Input, key: &str, mut body: Outbound) -> Result<(), Error> {
        body.config_hash = input
            .config_hash
            .clone()
            .or_else(|| Some(self.config.snapshot().hash.clone()));
        if let Some(binding) = self
            .store
            .binding(self.bot, input.chat, input.topic)
            .await?
        {
            body.binding_revision = Some(binding.revision);
        }
        self.store
            .enqueue(self.bot, input.chat, input.topic, key, &body)
            .await?;
        Ok(())
    }
    async fn open(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        thread: &str,
    ) -> Result<(), Error> {
        if !self.topics_enabled {
            return Err(Error::TopicsDisabled);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(thread).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        self.authorize(input, &snapshot.hash).await?;
        let topic = match self
            .store
            .topic(self.bot, input.chat, &workspace.provider_id, thread)
            .await?
        {
            Some(topic) => topic,
            None => {
                if !self
                    .store
                    .begin_topic(self.bot, input.chat, &workspace.provider_id, thread)
                    .await?
                {
                    return Err(Error::Unknown);
                }
                let title = topic_title(workspace, &current.title, thread);
                match self.api.create_topic(input.chat, &title).await {
                    Ok(topic) => {
                        self.store
                            .end_topic(
                                self.bot,
                                input.chat,
                                &workspace.provider_id,
                                thread,
                                "accepted",
                                Some(topic),
                            )
                            .await?;
                        topic
                    }
                    Err(error) => {
                        let state = if matches!(
                            error,
                            telegram::Error::Rejected
                                | telegram::Error::RetryAfter(_)
                                | telegram::Error::NoRecipient
                                | telegram::Error::Authentication
                        ) {
                            "rejected"
                        } else {
                            "outcome_unknown"
                        };
                        self.store
                            .end_topic(
                                self.bot,
                                input.chat,
                                &workspace.provider_id,
                                thread,
                                state,
                                None,
                            )
                            .await?;
                        return Err(error.into());
                    }
                }
            }
        };
        self.authorize(input, &snapshot.hash).await?;
        self.store
            .bind(self.bot, input.chat, topic, &workspace.provider_id, thread)
            .await?;
        self.store
            .set_workspace(self.bot, input.chat, topic, &workspace.id)
            .await?;
        let binding = self
            .store
            .binding(self.bot, input.chat, topic)
            .await?
            .ok_or(Error::Unbound)?;
        let binding_actor = self
            .connections
            .actor(
                &workspace.provider_id,
                &format!("__binding:{}:{topic}", input.chat),
            )
            .await;
        let _binding_guard = binding_actor.lock().await;
        let history = self
            .history(provider.as_ref(), workspace, thread, snapshot)
            .await;
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                self.history_unavailable(&binding, workspace, &current.cwd, &error, snapshot)
                    .await?;
                self.refresh_status(&binding, workspace, &current, snapshot)
                    .await?;
                return self.opened(input, snapshot, workspace, topic).await;
            }
        };
        let after = provider.read(thread).await?;
        connections::require_workspace(&after.cwd, workspace)?;
        let text = if matches!(current.status, RuntimeStatus::Active { .. })
            || matches!(after.status, RuntimeStatus::Active { .. })
        {
            session::delivery::agent_history(&history)
        } else {
            session::delivery::latest_agent_answer(&history)
        };
        let mut body = Outbound::text(text);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
        body.binding_revision = Some(binding.revision);
        self.store
            .publish_observation(
                &binding,
                &item_hashes(&history),
                &format!("history:{}", input.event_key),
                &body,
            )
            .await?;
        let mut card = Outbound::text(status_text(
            workspace,
            binding.display_name.as_deref().unwrap_or(&after.title),
            after.status.label(),
            thread,
        ));
        card.scope = Some(self.output_scope(&snapshot.config, workspace, &after.cwd)?);
        card.binding_revision = Some(binding.revision);
        self.store.enqueue_status(&binding, &card).await?;
        self.opened(input, snapshot, workspace, topic).await
    }
    async fn authorize(&self, input: &Input, hash: &str) -> Result<(), Error> {
        if self.config.snapshot().hash != hash {
            return Err(Error::Expired);
        }
        if input.callback.is_some()
            && !self
                .store
                .callback_live(self.bot, input, hash, now())
                .await?
        {
            return Err(Error::Expired);
        }
        if !self
            .store
            .pairing()
            .is_paired(self.bot, input.user, input.chat)
            .await
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::NotPaired.into());
        }
        Ok(())
    }
    fn output_scope(
        &self,
        config: &Config,
        workspace: &Workspace,
        cwd: &str,
    ) -> Result<Scope, Error> {
        let provider = config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .ok_or(connections::Error::Selection)?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or(connections::Error::Selection)?;
        Ok(Scope {
            workspace: workspace.id.clone(),
            cwd: cwd.into(),
            instance: provider.id.clone(),
            endpoint: connections::fingerprint(host),
        })
    }
    fn scope_allowed(&self, config: &Config, scope: &Scope) -> bool {
        let Ok(workspace) = self.workspace(config, &scope.workspace) else {
            return false;
        };
        if workspace.provider_id != scope.instance
            || connections::require_workspace(&scope.cwd, workspace).is_err()
        {
            return false;
        }
        self.output_scope(config, workspace, &scope.cwd)
            .is_ok_and(|current| current.endpoint == scope.endpoint)
    }
    async fn mutate(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
        key: &str,
        thread: &str,
        mutation: Mutation,
    ) -> Result<Receipt, Error> {
        self.authorize(input, &snapshot.hash).await?;
        if input.callback.is_none() && self.delayed(input) {
            return Err(Error::Stale);
        }
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        if !provider.capabilities().control_verified {
            return Err(Error::ReadOnly);
        }
        let id = self
            .store
            .reserve_operation(
                &format!("telegram:{}:{}:{}", self.bot, input.event_key, key),
                &workspace.provider_id,
                thread,
            )
            .await?;
        let state = self.store.operation_state(&id).await?;
        if state == State::Accepted {
            return self.store.receipt(&id).await?.ok_or(Error::Unknown);
        }
        if state != State::Received {
            return Err(Error::Unknown);
        }
        self.store
            .transition(&id, State::Received, State::Prepared)
            .await?;
        // Recheck after a potentially slow SSH connect and immediately before dispatch.
        self.authorize(input, &snapshot.hash).await?;
        if input.callback.is_none() && self.delayed(input) {
            return Err(Error::Stale);
        }
        let binding = if matches!(key, "resume" | "input") {
            Some(self.bound(input).await?)
        } else {
            None
        };
        if binding
            .as_ref()
            .is_some_and(|b| b.thread_id != thread || b.instance_id != workspace.provider_id)
        {
            return Err(Error::Expired);
        }
        self.store
            .dispatch_authorized(&id, self.bot, input, binding.as_ref())
            .await?;
        match provider.execute(mutation).await {
            Ok(receipt) => {
                self.store.accept_receipt(&id, &receipt).await?;
                Ok(receipt)
            }
            Err(error) => {
                let rejected = matches!(
                    error,
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
                    .await?;
                Err(if rejected {
                    error.into()
                } else {
                    Error::Unknown
                })
            }
        }
    }
    async fn create(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        text: &str,
    ) -> Result<(), Error> {
        if !self.topics_enabled {
            return Err(Error::TopicsDisabled);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let actor = self
            .connections
            .actor(&workspace.provider_id, "__create")
            .await;
        let _guard = actor.lock().await;
        let receipt = self
            .mutate(
                input,
                snapshot,
                workspace,
                "create",
                "__new",
                Mutation::Create {
                    cwd: workspace.cwd.clone(),
                },
            )
            .await?;
        let thread = &receipt.thread;
        // Each step has its own receipt. A crash never replays create/resume/start.
        self.mutate(
            input,
            snapshot,
            workspace,
            "first-turn",
            thread,
            Mutation::Start {
                thread: thread.clone(),
                text: text.into(),
                client_id: format!("pp:{}:{}", self.bot, input.event_key),
            },
        )
        .await?;
        self.open(input, snapshot, id, thread).await
    }
    async fn submit(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        text: &str,
        steer_only: bool,
    ) -> Result<(), Error> {
        if text.trim().is_empty() {
            return Err(Error::Stale);
        }
        let binding = self.bound(input).await?;
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        let actor = self
            .connections
            .actor(&binding.instance_id, &binding.thread_id)
            .await;
        let _guard = actor.lock().await;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(&binding.thread_id).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        let fresh = self.bound(input).await?;
        if fresh.revision != binding.revision {
            return Err(Error::Expired);
        }
        let client_id = format!("pp:{}:{}", self.bot, input.event_key);
        let (mutation, label) = match current.status {
            RuntimeStatus::Active {
                waiting_on_approval,
                waiting_on_user_input,
            } => {
                let expected = match current.active_turn {
                    Some(turn) => turn,
                    None => match provider.active_turn(&binding.thread_id).await? {
                        Some(turn) => turn,
                        None => return self.draft(input, snapshot, text, steer_only).await,
                    },
                };
                (
                    Mutation::Steer {
                        thread: binding.thread_id.clone(),
                        expected_turn: expected,
                        text: text.into(),
                        client_id,
                    },
                    if waiting_on_approval || waiting_on_user_input {
                        "已追加；原端的审批/用户问答仍需处理"
                    } else {
                        "已追加到运行中的轮次"
                    },
                )
            }
            RuntimeStatus::Idle if !steer_only => (
                Mutation::Start {
                    thread: binding.thread_id.clone(),
                    text: text.into(),
                    client_id,
                },
                "已开始新一轮",
            ),
            RuntimeStatus::NotLoaded if !steer_only => {
                self.mutate(
                    input,
                    snapshot,
                    workspace,
                    "resume",
                    &binding.thread_id,
                    Mutation::Resume {
                        thread: binding.thread_id.clone(),
                    },
                )
                .await?;
                (
                    Mutation::Start {
                        thread: binding.thread_id.clone(),
                        text: text.into(),
                        client_id,
                    },
                    "已继续会话",
                )
            }
            _ => return self.draft(input, snapshot, text, steer_only).await,
        };
        let result = self
            .mutate(
                input,
                snapshot,
                workspace,
                "input",
                &binding.thread_id,
                mutation,
            )
            .await;
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(Error::Provider(ProviderError::Conflict)) => {
                return self.draft(input, snapshot, text, steer_only).await;
            }
            Err(error) => return Err(error),
        };
        tracing::info!(thread_id=%binding.thread_id,turn_id=?receipt.turn,action=label,"session input accepted");
        Ok(())
    }

    async fn deliver(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut next_chat = HashMap::<i64, Instant>::new();
        let mut global = Instant::now();
        loop {
            let deliveries = match self.store.pending_deliveries(self.bot, now()).await {
                Ok(rows) => rows,
                Err(_) => break,
            };
            for row in deliveries {
                if *stop.borrow() {
                    return;
                }
                let snapshot = self.config.snapshot();
                let body: Outbound = match serde_json::from_str(&row.payload) {
                    Ok(body) => body,
                    Err(_) => break,
                };
                let paired = self
                    .store
                    .pairing()
                    .is_paired(self.bot, row.chat_id as u64, row.chat_id)
                    .await
                    .unwrap_or(false);
                let binding_valid = if let Some(revision) = body.binding_revision {
                    self.store
                        .binding(self.bot, row.chat_id, row.topic_id)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|b| b.revision == revision)
                } else {
                    true
                };
                if (body.require_pairing && !paired)
                    || row.chat_id <= 0
                    || !binding_valid
                    || body.expires_at.is_some_and(|expiry| expiry <= now())
                    || body
                        .scope
                        .as_ref()
                        .is_some_and(|scope| !self.scope_allowed(&snapshot.config, scope))
                    || body
                        .config_hash
                        .as_ref()
                        .is_some_and(|hash| *hash != snapshot.hash)
                {
                    let _ = self
                        .store
                        .delivery_state(&row.id, "pending", "rejected", None, 0)
                        .await;
                    continue;
                }
                if global > Instant::now()
                    || next_chat
                        .get(&row.chat_id)
                        .is_some_and(|at| *at > Instant::now())
                {
                    continue;
                }
                if self
                    .store
                    .delivery_state(&row.id, "pending", "dispatching", None, 0)
                    .await
                    .is_err()
                {
                    continue;
                }
                let result = async {
                    if let Some(title) = &body.topic_title {
                        self.api
                            .rename_topic(row.chat_id, row.topic_id, title)
                            .await?;
                        if let Some(binding) = self
                            .store
                            .binding(self.bot, row.chat_id, row.topic_id)
                            .await
                            .ok()
                            .flatten()
                        {
                            let _ = self.store.topic_title_receipt(&binding, title).await;
                        }
                    }
                    if let Some(message) = body.edit_message {
                        match self
                            .api
                            .edit_formatted(
                                row.chat_id,
                                message,
                                &body.text,
                                body.buttons.clone(),
                                body.markdown,
                            )
                            .await
                        {
                            Ok(()) => Ok(message),
                            Err(telegram::Error::MessageUnavailable)
                                if body.navigation_id.is_some()
                                    || row.source_key.starts_with("status:") =>
                            {
                                self.api
                                    .send_formatted(
                                        row.chat_id,
                                        row.topic_id,
                                        &body.text,
                                        body.buttons.clone(),
                                        body.markdown,
                                    )
                                    .await
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        self.api
                            .send_formatted(
                                row.chat_id,
                                row.topic_id,
                                &body.text,
                                body.buttons.clone(),
                                body.markdown,
                            )
                            .await
                    }
                }
                .await;
                global = Instant::now() + Duration::from_millis(40);
                next_chat.insert(
                    row.chat_id,
                    Instant::now()
                        + Duration::from_millis(
                            60000
                                / u64::from(snapshot.config.delivery.telegram_messages_per_minute),
                        ),
                );
                match result {
                    Ok(message) => {
                        if let Some(id) = &body.navigation_id {
                            let _ = self.store.navigation_receipt(id, message, now()).await;
                        }
                        let _ = self
                            .store
                            .delivery_state(&row.id, "dispatching", "delivered", Some(message), 0)
                            .await;
                        if row.source_key.starts_with("status:")
                            && let Ok(Some(binding)) = self
                                .store
                                .binding(self.bot, row.chat_id, row.topic_id)
                                .await
                        {
                            let _ = self.store.status_card(&binding, message, &body.text).await;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error,"Telegram delivery failed");
                        if matches!(error, telegram::Error::TopicUnavailable) {
                            let management = self
                                .store
                                .management_topic(self.bot, row.chat_id)
                                .await
                                .ok()
                                .flatten()
                                == Some(row.topic_id);
                            let _ = self
                                .store
                                .unavailable_topic(self.bot, row.chat_id, row.topic_id)
                                .await;
                            let mut notice = Outbound::text(if management {
                                "管理 topic 已关闭或删除。下次发送 /start 将重新创建管理入口；Codex 会话不受影响。"
                            } else {
                                "会话 topic 已关闭或删除；Codex 会话继续运行。请从 /sessions 重新添加，会创建一个新 topic。"
                            });
                            notice.scope = body.scope.clone();
                            let _ = self
                                .store
                                .enqueue(
                                    self.bot,
                                    row.chat_id,
                                    0,
                                    &format!("topic-unavailable:{}", row.id),
                                    &notice,
                                )
                                .await;
                        }
                        if matches!(error, telegram::Error::NoRecipient) {
                            let _ = self.store.disable_chat(self.bot, row.chat_id).await;
                        }
                        if matches!(error, telegram::Error::TooLarge) {
                            let mut fallback = Outbound::text(
                                "历史超过 Telegram 单个附件的大小限制，未发送不完整的历史。请在本机使用 sessions history --output 导出完整集合。",
                            );
                            fallback.scope = body.scope.clone();
                            fallback.binding_revision = body.binding_revision;
                            let _ = self
                                .store
                                .enqueue(
                                    self.bot,
                                    row.chat_id,
                                    row.topic_id,
                                    &format!("oversize:{}", row.id),
                                    &fallback,
                                )
                                .await;
                        }
                        if let telegram::Error::RetryAfter(seconds) = &error {
                            global = Instant::now() + Duration::from_secs(*seconds);
                        }
                        let (state, retry) = match error {
                            telegram::Error::RetryAfter(s) => (
                                "pending",
                                now().saturating_add(s.min(i64::MAX as u64) as i64),
                            ),
                            telegram::Error::Rejected
                            | telegram::Error::Authentication
                            | telegram::Error::NoRecipient
                            | telegram::Error::TopicUnavailable
                            | telegram::Error::TooLarge => ("rejected", 0),
                            _ => ("outcome_unknown", 0),
                        };
                        let _ = self
                            .store
                            .delivery_state(&row.id, "dispatching", state, None, retry)
                            .await;
                    }
                }
            }
            tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
    }

    async fn observe(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        type Key = (i64, i32);
        let mut workers = HashMap::<Key, tokio::task::AbortHandle>::new();
        let mut jobs = JoinSet::new();
        loop {
            let bindings = match self.store.bindings(self.bot).await {
                Ok(rows) => rows,
                Err(_) => break,
            };
            let active = bindings
                .iter()
                .map(|b| (b.chat_id, b.topic_id))
                .collect::<std::collections::HashSet<_>>();
            workers.retain(|key, handle| {
                if active.contains(key) {
                    true
                } else {
                    handle.abort();
                    false
                }
            });
            for binding in bindings {
                let key = (binding.chat_id, binding.topic_id);
                if workers.contains_key(&key) {
                    continue;
                }
                let service = self.clone();
                let handle=jobs.spawn(async move {
                    loop {
                        if service.store.binding(service.bot,key.0,key.1).await.ok().flatten().is_none() {break;}
                        if let Err(error)=service.refresh_binding(&binding).await {tracing::debug!(%error,"session observation unavailable; no mutation requested");let _=service.offline_status(&binding,&error).await;}
                        let delay=service.config.snapshot().config.delivery.burst_window_ms;
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    key
                });
                workers.insert(key, handle);
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next_with_id(), if !jobs.is_empty()=>{
                    match result {
                        Some(Ok((id,key)))=>{if workers.get(&key).is_some_and(|handle|handle.id()==id) {workers.remove(&key);}},
                        Some(Err(error))=>{workers.retain(|_,handle|handle.id()!=error.id());},
                        None=>{},
                    }
                },
                _=tokio::time::sleep(Duration::from_secs(2))=>{},
            }
        }
        jobs.abort_all();
    }
    async fn history(
        &self,
        provider: &dyn AgentProvider,
        workspace: &Workspace,
        thread: &str,
        snapshot: &Snapshot,
    ) -> Result<History, Error> {
        let local = self.connections.history_limit(&workspace.provider_id).await;
        let work = async {
            let _local = local
                .acquire()
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            let _global = self
                .history_slots
                .acquire()
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            provider.history(thread).await
        };
        Ok(tokio::time::timeout(
            Duration::from_secs(snapshot.config.delivery.history_barrier_timeout_seconds),
            work,
        )
        .await
        .map_err(|_| ProviderError::Timeout)??)
    }
    async fn offline_status(&self, binding: &Binding, error: &Error) -> Result<(), Error> {
        let binding = self
            .store
            .binding(self.bot, binding.chat_id, binding.topic_id)
            .await?
            .ok_or(Error::Unbound)?;
        let snapshot = self.config.snapshot();
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        tracing::debug!(thread_id=%binding.thread_id,%error,"session status unavailable");
        let status = "⚫ 暂时离线，正在重连。".to_owned();
        if binding.status_text.as_deref() == Some(&status) {
            return Ok(());
        }
        let mut body = Outbound::text(status);
        body.edit_message = binding.status_message_id;
        body.binding_revision = Some(binding.revision);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?);
        self.store.enqueue_status(&binding, &body).await?;
        Ok(())
    }
    async fn history_unavailable(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        cwd: &str,
        error: &Error,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        tracing::warn!(%error,"session history unavailable; keeping independently observed runtime");
        let mut body = Outbound::text("⚠️ 历史暂不可用，请稍后 /history。");
        body.scope = Some(self.output_scope(&snapshot.config, workspace, cwd)?);
        body.binding_revision = Some(binding.revision);
        self.store
            .enqueue(
                self.bot,
                binding.chat_id,
                binding.topic_id,
                &format!(
                    "history-unavailable:{}:{}",
                    binding.thread_id, binding.revision
                ),
                &body,
            )
            .await?;
        Ok(())
    }
    async fn refresh_status(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        current: &session::Session,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let status = status_text(
            workspace,
            binding.display_name.as_deref().unwrap_or(&current.title),
            current.status.label(),
            &binding.thread_id,
        );
        let title = topic_title(
            workspace,
            binding.display_name.as_deref().unwrap_or(&current.title),
            &binding.thread_id,
        );
        let title_changed = binding.topic_title.as_deref() != Some(&title);
        if binding.status_text.as_deref() != Some(&status) || title_changed {
            let mut body = Outbound::text(status);
            body.edit_message = binding.status_message_id;
            body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
            body.binding_revision = Some(binding.revision);
            if title_changed {
                body.topic_title = Some(title);
            }
            // Frequent runtime changes edit one card; Telegram emits a service
            // message for every topic rename, so status never belongs in its name.
            self.store.enqueue_status(binding, &body).await?;
        }
        Ok(())
    }
    async fn refresh_binding(&self, binding: &Binding) -> Result<(), Error> {
        let actor = self
            .connections
            .actor(
                &binding.instance_id,
                &format!("__binding:{}:{}", binding.chat_id, binding.topic_id),
            )
            .await;
        let _guard = actor.lock().await;
        let binding = self
            .store
            .binding(self.bot, binding.chat_id, binding.topic_id)
            .await?
            .ok_or(Error::Unbound)?;
        let binding = &binding;
        if !self
            .store
            .pairing()
            .is_paired(self.bot, binding.chat_id as u64, binding.chat_id)
            .await
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::NotPaired.into());
        }
        let snapshot = self.config.snapshot();
        let workspace = self.binding_workspace(&snapshot.config, binding)?;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(&binding.thread_id).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        self.refresh_status(binding, workspace, &current, &snapshot)
            .await?;
        let history = self
            .history(provider.as_ref(), workspace, &binding.thread_id, &snapshot)
            .await;
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                return self
                    .history_unavailable(binding, workspace, &current.cwd, &error, &snapshot)
                    .await;
            }
        };
        if !binding.history_ready {
            let text = if matches!(current.status, RuntimeStatus::Active { .. }) {
                session::delivery::agent_history(&history)
            } else {
                session::delivery::latest_agent_answer(&history)
            };
            let mut body = Outbound::text(text);
            body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
            body.binding_revision = Some(binding.revision);
            self.store
                .publish_observation(
                    binding,
                    &item_hashes(&history),
                    &format!("initial-history:{}", binding.revision),
                    &body,
                )
                .await?;
            return Ok(());
        }
        let old: BTreeMap<String, String> =
            serde_json::from_str(&binding.observed_items).map_err(|_| StoreError::Database)?;
        let mut new = old.clone();
        new.extend(item_hashes(&history));
        let changed = history
            .items
            .iter()
            .filter(|i| old.get(&item_key(i)) != new.get(&item_key(i)))
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return Ok(());
        }
        let text = changed
            .iter()
            .filter(|i| i.role == session::Role::Agent)
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let visible_members = changed
            .iter()
            .filter(|i| i.role == session::Role::Agent)
            .count();
        if text.is_empty() {
            self.store.observe(binding, &new).await?;
            return Ok(());
        }
        let mut body = Outbound::text(text);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
        body.binding_revision = Some(binding.revision);
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Stale)?
            .as_millis()
            .min(i64::MAX as u128) as i64;
        self.store
            .publish_batch(
                binding,
                &new,
                body,
                visible_members,
                &snapshot.config.delivery,
                time_ms,
            )
            .await?;
        Ok(())
    }
}

impl Error {
    fn user_message(&self) -> &'static str {
        match self {
            Self::Unknown => "⚠️ 结果未确认，请先核对会话，避免重复发送。",
            Self::ReadOnly => "⚪ 此主机暂时只读。",
            Self::Expired | Self::Stale => "请重新确认这条消息。",
            Self::Unbound => "请先在管理 topic 选择会话。",
            Self::TopicsDisabled => "请先为 bot 开启 Topics。",
            _ => "⚠️ 暂时无法处理，请稍后重试。",
        }
    }
}

fn topic_title(workspace: &Workspace, title: &str, thread: &str) -> String {
    format!(
        "{} · {}",
        session::delivery::short_title(&format!("{} · {}", workspace.short_name, title), 32),
        thread
            .chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    )
}
fn status_text(_workspace: &Workspace, title: &str, status: &str, _thread: &str) -> String {
    format!("{}\n{status}", session::delivery::short_title(title, 40))
}

fn item_key(item: &session::TextItem) -> String {
    format!("{}:{}{}", item.turn_id.len(), item.turn_id, item.item_id)
}
fn item_hashes(history: &History) -> BTreeMap<String, String> {
    history
        .items
        .iter()
        .map(|item| (item_key(item), digest(&item.text)))
        .collect()
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
