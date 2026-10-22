//! Supervise configured integrations and read-only discovery under one shutdown signal.

use engine::{reload::ConfigFile, runtimes::RuntimeRegistry, storage::Store};
use integrations::{Context, Integration, IntegrationError};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinSet};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] engine::config::ConfigError),
    #[error(transparent)]
    Store(#[from] engine::storage::StoreError),
    #[error(transparent)]
    Integration(#[from] IntegrationError),
    #[error("configure a Telegram, Discord, Slack, Feishu or Lark integration")]
    Missing,
    #[error("a runtime task stopped unexpectedly")]
    WorkerStopped,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Missing => ::i18n::Message::new("error.runtime.missing"),
            Self::WorkerStopped => ::i18n::Message::new("error.runtime.worker"),
            Self::Config(error) => ::i18n::LocalizedError::message(error),
            Self::Store(error) => ::i18n::LocalizedError::message(error),
            Self::Integration(error) => ::i18n::LocalizedError::message(error),
        }
    }
}

pub async fn run(path: PathBuf) -> Result<(), Error> {
    integrations::network::initialize_tls();
    let root = crate::config::load(&path)?.config;
    let config = Arc::new(ConfigFile::open(path, crate::config::runtime_parser)?);
    if config.snapshot().hash != root.source_hash() {
        return Err(engine::config::ConfigError::Invalid.into());
    }
    let store = Store::open(&root.service.data_dir).await?;
    let mut initial = config.snapshot().config.clone();
    engine::catalog::Catalog::new(store.pool().clone())
        .load(&mut initial)
        .await?;
    config.publish_catalog(initial)?;
    let runtimes = Arc::new(RuntimeRegistry::new(store.clone()));
    let context = Context {
        config: config.clone(),
        store,
        runtimes,
    };
    let integrations = configured_integrations(root);
    if integrations.is_empty() {
        return Err(Error::Missing);
    }
    let (stop, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let catalog_context = context.clone();
    let catalog_stop = receiver.clone();
    tasks.spawn(async move {
        crate::inventory::watch(catalog_context, catalog_stop).await;
        Ok::<(), IntegrationError>(())
    });
    let watcher = config.clone();
    let watch_stop = receiver.clone();
    tasks.spawn(async move {
        watcher.watch(watch_stop).await;
        Ok::<(), IntegrationError>(())
    });
    for integration in integrations {
        let context = context.clone();
        let receiver = receiver.clone();
        tracing::info!(integration = integration.name(), "starting integration");
        tasks.spawn(async move { integration.run(context, receiver).await });
    }
    let result = tokio::select! {
        _=shutdown_signal()=>Ok(()),
        exit=tasks.join_next()=>match exit {Some(Ok(Err(error)))=>Err(Error::Integration(error)),_=>Err(Error::WorkerStopped)},
    };
    let _ = stop.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(6), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    result
}

fn configured_integrations(root: crate::config::Config) -> Vec<Arc<dyn Integration>> {
    let mut integrations: Vec<Arc<dyn Integration>> = Vec::new();
    if let Some(settings) = root.telegram {
        integrations.push(Arc::new(integrations::telegram::Telegram::new(settings)));
    }
    if let Some(settings) = root.discord {
        integrations.push(Arc::new(integrations::discord::Discord::new(settings)));
    }
    if let Some(settings) = root.slack {
        integrations.push(Arc::new(integrations::slack::Slack::new(settings)));
    }
    for (region, settings) in [
        (integrations::feishu::config::Region::Feishu, root.feishu),
        (integrations::feishu::config::Region::Lark, root.lark),
    ] {
        if let Some(settings) = settings {
            integrations.push(Arc::new(integrations::feishu::Feishu::for_region(
                region, settings,
            )));
        }
    }
    integrations
}

pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = signal.recv() => {},
            };
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
