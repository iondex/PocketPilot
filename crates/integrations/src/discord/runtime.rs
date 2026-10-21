//! Discord integration assembly.
use crate::discord::storage::Binding;
use crate::discord::{Api, Backend, Input, storage::Store};
use engine::config::Workspace;
use engine::{reload::ConfigFile, runtimes::RuntimeRegistry, storage::Store as CoreStore};
use session::History;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinSet};

type Result<T> = std::result::Result<T, i18n::Message>;

fn error(e: impl i18n::LocalizedError) -> i18n::Message {
    e.message()
}

#[path = "binding.rs"]
mod binding;

#[path = "commands.rs"]
mod commands;

#[path = "delivery.rs"]
mod delivery;

#[path = "input.rs"]
mod input;

#[path = "observation.rs"]
mod observation;

#[path = "routing.rs"]
mod routing;

#[path = "menus.rs"]
mod menus;

pub struct Service {
    config: Arc<ConfigFile>,
    store: Store,
    runtimes: Arc<RuntimeRegistry>,
    stop: watch::Receiver<bool>,
}

struct Target<'a> {
    workspace: &'a Workspace,
    thread: &'a str,
    binding: Option<&'a Binding>,
}

pub struct Discord {
    settings: crate::discord::config::Config,
}

impl Discord {
    pub fn new(settings: crate::discord::config::Config) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl crate::Integration for Discord {
    fn platform(&self) -> crate::Platform {
        crate::Platform::Discord
    }

    fn capabilities(&self) -> crate::rendering::Capabilities {
        crate::rendering::Capabilities::for_platform(self.platform())
    }

    async fn run(
        &self,
        context: crate::Context,
        stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), crate::IntegrationError> {
        run(
            context.config,
            context.store,
            context.runtimes,
            &self.settings,
            stop,
        )
        .await
        .map_err(|e| crate::IntegrationError {
            platform: "discord",
            message: i18n::LocalizedError::message(&e),
        })
    }
}

pub async fn run(
    config: Arc<ConfigFile>,
    store: CoreStore,
    runtimes: Arc<RuntimeRegistry>,
    settings: &crate::discord::config::Config,
    mut stop: watch::Receiver<bool>,
) -> std::result::Result<(), crate::discord::Error> {
    let store = Store::new(store);
    let token = secrecy::SecretString::from(settings.token.clone());
    let api = Api::connect(&token, settings.guild_id, &settings.proxy).await?;
    store
        .discord_recover()
        .await
        .map_err(|_| crate::discord::Error::Backend)?;
    let (shutdown, receiver) = watch::channel(false);
    let service = Arc::new(Service {
        config,
        store,
        runtimes,
        stop: receiver.clone(),
    });
    let mut jobs = JoinSet::new();
    jobs.spawn(crate::discord::run(
        token,
        api.clone(),
        service.clone(),
        receiver.clone(),
    ));
    jobs.spawn(service.clone().observe(api.clone(), receiver.clone()));
    jobs.spawn(service.deliver(api, receiver));
    let result = tokio::select! {
        _=stop.changed()=>Ok(()),
        result=jobs.join_next()=>result.ok_or(crate::discord::Error::Backend)?.map_err(|_|crate::discord::Error::Backend)?,
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
    async fn new_session_form(
        &self,
        api: &Api,
        input: Input,
        token: &str,
    ) -> Result<crate::discord::ui::NewSessionForm> {
        self.prepare_new_session_form(api, &input, token).await
    }

    async fn menu(
        &self,
        api: &Api,
        input: Input,
        token: Option<String>,
        value: Option<String>,
    ) -> Result<crate::discord::ui::Screen> {
        self.menu_screen(api, input, token, value).await
    }
    async fn handle(&self, api: &Api, input: Input) -> Result<String> {
        let snapshot = self.config.snapshot();
        let output = self.command(api, &input, &snapshot).await?;
        self.fresh(api, &input, &snapshot, false).await?;
        Ok(output)
    }
}

impl Service {
    pub(super) fn locale(&self, input: &Input) -> String {
        input
            .locale
            .clone()
            .unwrap_or_else(|| self.config.snapshot().config.language.clone())
    }

    #[cfg(test)]
    pub fn local_fixture(
        config: Arc<ConfigFile>,
        store: CoreStore,
        runtimes: Arc<RuntimeRegistry>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            config,
            store: Store::new(store),
            runtimes,
            stop,
        }
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
            .map(|i| (item_key(i), engine::observation::visible_hash(i)))
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap_or_else(|_| "{}".into())
}
