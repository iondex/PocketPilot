//! Telegram integration assembly; interaction and persistence live in focused modules.
use crate::ledger::digest;
use crate::{
    api::{Api, Input},
    storage::Store,
};
use engine::{config::Workspace, storage::StoreError};
use engine::{
    connections::{self, Connections},
    now,
    reload::ConfigFile,
};
use serde::{Deserialize, Serialize};
use session::{History, ProviderError};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
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
    #[error(transparent)]
    Config(#[from] engine::config::ConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Connection(#[from] connections::Error),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Telegram(#[from] crate::Error),
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

#[path = "binding.rs"]
mod binding;

#[path = "commands.rs"]
mod commands;

#[path = "delivery.rs"]
mod delivery;

#[path = "inbox.rs"]
mod inbox;

#[path = "input.rs"]
mod input;

#[path = "observation.rs"]
mod observation;

#[path = "routing.rs"]
mod routing;

pub struct Service {
    pub(crate) config: Arc<ConfigFile>,
    pub(crate) store: Store,
    pub(crate) connections: Arc<Connections>,
    api: Api,
    bot: i64,
    topics_enabled: bool,
    pair_attempts: Mutex<HashMap<(u64, bool), Instant>>,
    history_slots: Semaphore,
    started_at: i64,
}

pub struct Telegram {
    settings: crate::config::Config,
}

impl Telegram {
    pub fn new(settings: crate::config::Config) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl engine::integrations::Integration for Telegram {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn run(
        &self,
        context: engine::integrations::Context,
        mut stop: watch::Receiver<bool>,
    ) -> Result<(), engine::integrations::IntegrationError> {
        run_adapter(&self.settings, context, &mut stop)
            .await
            .map_err(|e| engine::integrations::IntegrationError {
                platform: "telegram",
                message: e.to_string(),
            })
    }
}

async fn run_adapter(
    settings: &crate::config::Config,
    context: engine::integrations::Context,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let api = Api::new(crate::credentials::read(settings)?)?;
    let identity = api.identity().await?;
    if identity.webhook_enabled {
        return Err(Error::Webhook);
    }
    let bot = i64::try_from(identity.bot_id).map_err(|_| Error::ReadOnly)?;
    if identity.user_topic_creation_enabled {
        tracing::warn!(
            "Telegram clients can auto-create topics; disable user-created threads in BotFather"
        );
    }
    let store = Store::new(context.store);
    store.recover_deliveries().await?;
    store.recover_inbox(bot).await?;
    let service = Arc::new(Service {
        config: context.config,
        store,
        connections: context.connections,
        api,
        bot,
        topics_enabled: identity.topics_enabled,
        pair_attempts: Mutex::new(HashMap::new()),
        history_slots: Semaphore::new(4),
        started_at: now().saturating_add(1),
    });
    let mut tasks = JoinSet::new();
    tasks.spawn(service.clone().poll(stop.clone()));
    tasks.spawn(service.clone().work(stop.clone()));
    tasks.spawn(service.clone().deliver(stop.clone()));
    tasks.spawn(service.observe(stop.clone()));
    tracing::info!(
        bot_id = bot,
        private_topics = identity.topics_enabled,
        "Telegram ready; session lifecycle remains owned by the app-server"
    );
    let failed = tokio::select! {_=stop.changed()=>false,_=tasks.join_next()=>true};
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    if failed {
        Err(Error::WorkerStopped)
    } else {
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
