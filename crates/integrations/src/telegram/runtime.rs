//! Telegram integration assembly; interaction and persistence live in focused modules.
use crate::telegram::{
    api::{Api, Input},
    storage::Store,
};
use engine::observation::{unchanged, visible_hash};
use engine::{config::Workspace, storage::StoreError};
use engine::{
    now,
    reload::ConfigFile,
    runtimes::{self, RuntimeRegistry},
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
    Connection(#[from] runtimes::Error),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Telegram(#[from] crate::telegram::Error),
    #[error("Telegram 已配置 webhook；请先自行确认接收方式，PocketPilot 未修改 webhook")]
    Webhook,
    #[error("按钮已过期或配置已更新，请重新打开菜单")]
    Expired,
    #[error("当前 topic 尚未绑定会话；请使用 /workspaces")]
    Unbound,
    #[error("workspace name is ambiguous; choose a host or use its workspace ID")]
    AmbiguousWorkspace,
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

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Webhook => ::i18n::Message::new("error.telegram_runtime.webhook"),
            Self::Expired => ::i18n::Message::new("error.telegram_runtime.expired"),
            Self::Unbound => ::i18n::Message::new("error.telegram_runtime.unbound"),
            Self::AmbiguousWorkspace => {
                ::i18n::Message::new("error.telegram_runtime.ambiguous_workspace")
            }
            Self::ReadOnly => ::i18n::Message::new("error.telegram_runtime.read_only"),
            Self::Unknown => ::i18n::Message::new("error.telegram_runtime.unknown"),
            Self::Stale => ::i18n::Message::new("error.telegram_runtime.stale"),
            Self::TopicsDisabled => ::i18n::Message::new("error.telegram_runtime.topics_disabled"),
            Self::WorkerStopped => ::i18n::Message::new("error.telegram_runtime.worker_stopped"),
            Self::Config(error) => ::i18n::LocalizedError::message(error),
            Self::Store(error) => ::i18n::LocalizedError::message(error),
            Self::Connection(error) => ::i18n::LocalizedError::message(error),
            Self::Provider(error) => ::i18n::LocalizedError::message(error),
            Self::Telegram(error) => ::i18n::LocalizedError::message(error),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Home,
    HomePage {
        offset: usize,
    },
    NewHostsPage {
        offset: usize,
    },
    Host {
        id: String,
    },
    HostPage {
        id: String,
        offset: usize,
    },
    NewSession,
    NewHost {
        id: String,
    },
    NewHostPage {
        id: String,
        offset: usize,
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
    Bindings {
        offset: usize,
    },
    Binding {
        target: crate::bindings::Target,
    },
    BindingConfirm {
        target: crate::bindings::Target,
    },
    BindingChange {
        target: crate::bindings::Target,
        version: crate::bindings::Version,
        change: crate::bindings::Change,
    },
    Topics {
        after: i32,
    },
    ConfirmTopicRemoval {
        topic: i32,
    },
    RemoveTopic {
        topic: i32,
    },
    ConfirmClearTopics,
    ClearTopics {
        topics: Vec<i32>,
    },
    SubmitDraft {
        #[serde(default)]
        uploads: Vec<session::files::Incoming>,
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

#[path = "history_view.rs"]
mod history_view;

#[path = "messages.rs"]
mod messages;

#[path = "session_errors.rs"]
mod session_errors;

#[path = "routing.rs"]
mod routing;

#[path = "presence.rs"]
mod presence;

pub struct Service {
    presence: presence::Presence,
    pub(crate) default_locale: String,
    pub(crate) config: Arc<ConfigFile>,
    pub(crate) store: Store,
    pub(crate) runtimes: Arc<RuntimeRegistry>,
    api: Api,
    pub(crate) bot: i64,
    topics_enabled: bool,
    pair_attempts: Mutex<HashMap<(u64, bool), Instant>>,
    history_slots: Semaphore,
    started_at: i64,
}

pub struct Telegram {
    settings: crate::telegram::config::Config,
}

impl Telegram {
    pub fn new(settings: crate::telegram::config::Config) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl crate::Integration for Telegram {
    fn platform(&self) -> crate::Platform {
        crate::Platform::Telegram
    }

    fn capabilities(&self) -> crate::rendering::Capabilities {
        crate::rendering::Capabilities::for_platform(self.platform())
    }

    async fn run(
        &self,
        context: crate::Context,
        mut stop: watch::Receiver<bool>,
    ) -> Result<(), crate::IntegrationError> {
        run_adapter(&self.settings, context, &mut stop)
            .await
            .map_err(|e| crate::IntegrationError {
                platform: "telegram",
                message: ::i18n::LocalizedError::message(&e),
            })
    }
}

async fn run_adapter(
    settings: &crate::telegram::config::Config,
    context: crate::Context,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let api = Api::with_proxy(
        crate::telegram::credentials::read(settings)?,
        &settings.proxy,
    )?;
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
        presence: presence::Presence::default(),
        default_locale: settings.default_locale.clone(),
        config: context.config,
        store,
        runtimes: context.runtimes,
        api,
        bot,
        topics_enabled: identity.topics_enabled,
        pair_attempts: Mutex::new(HashMap::new()),
        history_slots: Semaphore::new(4),
        started_at: now().saturating_add(1),
    });
    let mut tasks = JoinSet::new();
    tokio::select! {
        _ = stop.changed() => return Ok(()),
        result = service.api.register_commands(&settings.default_locale) => {
            match result {
                Ok(()) => tracing::info!("Telegram private command menus registered"),
                Err(error) => tracing::warn!(%error,"Telegram command registration unavailable; typed commands remain active"),
            }
        }
    }
    tasks.spawn(service.clone().poll(stop.clone()));
    tasks.spawn(service.clone().work(stop.clone()));
    tasks.spawn(service.clone().deliver(stop.clone()));
    tasks.spawn(service.clone().typing(stop.clone()));
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

fn topic_title(_workspace: &Workspace, title: &str, _thread: &str) -> String {
    crate::telegram::presentation::title(title)
}

fn item_key(item: &session::TextItem) -> String {
    format!("{}:{}{}", item.turn_id.len(), item.turn_id, item.item_id)
}

fn item_hashes(history: &History) -> BTreeMap<String, String> {
    history
        .items
        .iter()
        .map(|item| (item_key(item), visible_hash(item)))
        .collect()
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
