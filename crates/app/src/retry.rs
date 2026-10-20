//! Supervise bounded provider scans, waking on native events and periodic discovery.
use engine::{
    catalog::Catalog, reload::ConfigFile, retry::Monitor, runtimes::RuntimeRegistry, storage::Store,
};
use integrations::Context;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] engine::config::ConfigError),
    #[error(transparent)]
    Store(#[from] engine::storage::StoreError),
    #[error("a retry task stopped unexpectedly")]
    WorkerStopped,
}

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        match self {
            Self::Config(error) => error.message(),
            Self::Store(error) => error.message(),
            Self::WorkerStopped => i18n::Message::new("error.runtime.worker"),
        }
    }
}

pub async fn run(path: PathBuf) -> Result<(), Error> {
    let root = crate::config::load(&path)?.config;
    let config = Arc::new(ConfigFile::open(path, crate::config::runtime_parser)?);
    if config.snapshot().hash != root.source_hash() {
        return Err(engine::config::ConfigError::Invalid.into());
    }
    // The catalog is shared with CLI/serve; retry owns a separate operation ledger and lock.
    let catalog = Catalog::open(&root.service.data_dir).await?;
    let store = Store::open(&root.service.data_dir.join("retry")).await?;
    let mut initial = config.snapshot().config.clone();
    catalog.load(&mut initial).await?;
    config.publish_catalog(initial)?;
    let context = Context {
        config: config.clone(),
        runtimes: Arc::new(RuntimeRegistry::new(store.clone())),
        store,
    };
    let (stop, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    tasks.spawn(watch(context.clone(), receiver.clone()));
    tasks.spawn(crate::inventory::watch_catalog(
        context,
        catalog,
        receiver.clone(),
    ));
    tasks.spawn(config.watch(receiver));
    let result = tokio::select! {
        _=crate::runtime::shutdown_signal()=>Ok(()),
        _=tasks.join_next()=>Err(Error::WorkerStopped),
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

async fn watch(context: Context, mut stop: watch::Receiver<bool>) {
    let mut monitors = HashMap::<String, Monitor>::new();
    let mut queue = VecDeque::<String>::new();
    let mut busy = HashSet::new();
    let mut jobs = JoinSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    tracing::info!(delay_seconds = 60, "retry monitor enabled by operator");
    loop {
        let snapshot = context.config.snapshot();
        let ids: HashSet<_> = snapshot
            .config
            .providers
            .iter()
            .map(|p| p.id.clone())
            .collect();
        monitors.retain(|id, _| ids.contains(id));
        queue.retain(|id| ids.contains(id));
        for provider in &snapshot.config.providers {
            if !busy.contains(&provider.id) && !queue.contains(&provider.id) {
                queue.push_back(provider.id.clone());
            }
        }
        while jobs.len() < 4 {
            let Some(id) = queue.pop_front() else {
                break;
            };
            busy.insert(id.clone());
            let mut monitor = monitors.remove(&id).unwrap_or_default();
            let context = context.clone();
            jobs.spawn(async move {
                let snapshot = context.config.snapshot();
                let mut changes = context.runtimes.get_provider(&snapshot.config, &id).await
                    .ok().and_then(|p| p.session_changes());
                if let Some(changes) = changes.as_mut() { changes.borrow_and_update(); }
                if let Err(error) = monitor.scan(&context.config, &context.runtimes, &context.store, &id).await {
                    tracing::warn!(provider_id=%id,%error,"retry scan unavailable; reads will retry");
                }
                // Native notifications may be scoped or lost. Always enumerate again.
                tokio::select! {
                    _=tokio::time::sleep(Duration::from_secs(5))=>{},
                    _=async { match changes.as_mut() {
                        Some(changes) => { let _ = changes.changed().await; },
                        None => std::future::pending::<()>().await,
                    }}=>{},
                }
                (id, monitor)
            });
        }
        tokio::select! {
            _=stop.changed()=>break,
            _=interval.tick()=>{},
            Some(result)=jobs.join_next(),if !jobs.is_empty()=>match result {
                Ok((id,monitor))=> {
                    busy.remove(&id);
                    monitors.insert(id,monitor);
                },
                Err(error)=> {
                    tracing::error!(%error,"retry worker stopped");
                    // Return to the supervisor instead of silently leaving a provider unmonitored.
                    break;
                },
            },
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
}
