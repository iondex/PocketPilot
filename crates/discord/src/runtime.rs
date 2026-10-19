//! Discord integration assembly.
use crate::storage::Binding;
use crate::{Api, Backend, Input, storage::Store};
use engine::config::Workspace;
use engine::digest;
use engine::{connections::Connections, reload::ConfigFile, storage::Store as CoreStore};
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

pub struct Discord {
    settings: crate::config::Config,
}

impl Discord {
    pub fn new(settings: crate::config::Config) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl engine::integrations::Integration for Discord {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn run(
        &self,
        context: engine::integrations::Context,
        stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), engine::integrations::IntegrationError> {
        run(
            context.config,
            context.store,
            context.connections,
            &self.settings,
            stop,
        )
        .await
        .map_err(|e| engine::integrations::IntegrationError {
            platform: "discord",
            message: i18n::LocalizedError::message(&e),
        })
    }
}

pub async fn run(
    config: Arc<ConfigFile>,
    store: CoreStore,
    connections: Arc<Connections>,
    settings: &crate::config::Config,
    mut stop: watch::Receiver<bool>,
) -> std::result::Result<(), crate::Error> {
    let store = Store::new(store);
    let token = secrecy::SecretString::from(settings.token.clone());
    let api = Api::connect(&token, settings.guild_id, &settings.proxy).await?;
    store
        .discord_recover()
        .await
        .map_err(|_| crate::Error::Backend)?;
    let (shutdown, receiver) = watch::channel(false);
    let service = Arc::new(Service {
        config,
        store,
        connections,
        stop: receiver.clone(),
    });
    let mut jobs = JoinSet::new();
    jobs.spawn(crate::run(
        token,
        api.clone(),
        service.clone(),
        receiver.clone(),
    ));
    jobs.spawn(service.clone().observe(api.clone(), receiver.clone()));
    jobs.spawn(service.deliver(api, receiver));
    let result = tokio::select! {
        _=stop.changed()=>Ok(()),
        result=jobs.join_next()=>result.ok_or(crate::Error::Backend)?.map_err(|_|crate::Error::Backend)?,
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
    pub(super) fn locale(&self, input: &Input) -> String {
        input
            .locale
            .clone()
            .unwrap_or_else(|| self.config.snapshot().config.language.clone())
    }

    #[cfg(feature = "test-support")]
    pub fn local_fixture(
        config: Arc<ConfigFile>,
        store: CoreStore,
        connections: Arc<Connections>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            config,
            store: Store::new(store),
            connections,
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
            .map(|i| (item_key(i), digest(&i.text)))
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap_or_else(|_| "{}".into())
}
