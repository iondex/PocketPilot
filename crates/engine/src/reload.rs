use crate::{
    config::{Config, ConfigError},
    digest,
};
use arc_swap::ArcSwap;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};

pub struct ParsedConfig {
    pub config: Config,
    pub restart_key: String,
}

pub type Parser = fn(&std::path::Path, &str) -> Result<ParsedConfig, ConfigError>;

pub struct Snapshot {
    restart_key: String,
    file_hash: String,
    pub config: Config,
    pub hash: String,
}

pub struct ConfigFile {
    pub path: PathBuf,
    current: ArcSwap<Snapshot>,
    parser: Parser,
    catalog_managed: AtomicBool,
}

impl ConfigFile {
    pub fn open(path: PathBuf, parser: Parser) -> Result<Self, ConfigError> {
        let path = path.canonicalize().map_err(|_| ConfigError::Io)?;
        let current = Self::read(&path, parser)?;
        Ok(Self {
            path,
            parser,
            catalog_managed: AtomicBool::new(false),
            current: ArcSwap::from_pointee(current),
        })
    }

    fn read(path: &std::path::Path, parser: Parser) -> Result<Snapshot, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|_| ConfigError::Io)?;
        let loaded = parser(path, &text)?;
        Ok(Snapshot {
            config: loaded.config,
            restart_key: loaded.restart_key,
            hash: digest(&text),
            file_hash: digest(&text),
        })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }

    /// Single-file atomic reload. Runtime-owned data directory and bot-account changes
    /// require a service restart, which only disconnects PocketPilot's transport.
    pub fn reload(&self) -> Result<bool, ConfigError> {
        let mut next = Self::read(&self.path, self.parser)?;
        let old = self.snapshot();
        if next.file_hash == old.file_hash {
            return Ok(false);
        }
        if next.config.service.data_dir != old.config.service.data_dir {
            return Err(ConfigError::Invalid);
        }
        if next.restart_key != old.restart_key {
            return Err(ConfigError::Invalid);
        }
        if self.catalog_managed.load(Ordering::Acquire) {
            next.config.providers = old.config.providers.clone();
            next.config.workspaces = old.config.workspaces.clone();
            crate::catalog::restrict(&mut next.config);
            next.config.validate()?;
            next.hash = catalog_hash(&next.config, &next.file_hash)?;
        }
        let previous = self.current.compare_and_swap(&old, Arc::new(next));
        Ok(Arc::ptr_eq(&old, &previous))
    }

    /// Publish a coherent catalog snapshot. Root-file changes win over an
    /// in-flight catalog read, so neither source can undo a permission removal.
    pub fn publish_catalog(&self, config: Config) -> Result<bool, ConfigError> {
        let old = self.snapshot();
        if config.source_hash() != old.file_hash {
            return Ok(false);
        }
        config.validate()?;
        let hash = catalog_hash(&config, &old.file_hash)?;
        self.catalog_managed.store(true, Ordering::Release);
        if hash == old.hash
            && serde_json::to_string(&config.workspaces).ok()
                == serde_json::to_string(&old.config.workspaces).ok()
        {
            return Ok(false);
        }
        let previous = self.current.compare_and_swap(
            &old,
            Arc::new(Snapshot {
                restart_key: old.restart_key.clone(),
                file_hash: old.file_hash.clone(),
                config,
                hash,
            }),
        );
        Ok(Arc::ptr_eq(&old, &previous))
    }

    pub async fn watch(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut watcher: Option<RecommendedWatcher> =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    let _ = sender.try_send(());
                }
            })
            .ok();
        if let Some(watcher) = watcher.as_mut()
            && watcher
                .watch(
                    self.path.parent().unwrap_or(&self.path),
                    RecursiveMode::NonRecursive,
                )
                .is_err()
        {
            tracing::warn!("config event watcher unavailable; periodic rescan remains active");
        }
        loop {
            let snapshot = self.snapshot();
            tokio::select! {
                _=stop.changed()=>break,
                _=receiver.recv(), if watcher.is_some()=>{tokio::time::sleep(Duration::from_millis(snapshot.config.config_reload.debounce_ms)).await;},
                _=tokio::time::sleep(Duration::from_secs(snapshot.config.config_reload.periodic_rescan_seconds))=>{},
            }
            if snapshot.config.config_reload.enabled {
                match self.reload() {
                    Ok(true) => tracing::info!("configuration reloaded"),
                    Ok(false) => {}
                    Err(_) => tracing::warn!(
                        "invalid configuration update rejected; retaining last valid configuration"
                    ),
                }
            }
        }
        drop(watcher);
    }
}

fn catalog_hash(config: &Config, file_hash: &str) -> Result<String, ConfigError> {
    let mut scopes = config.workspaces.clone();
    for workspace in &mut scopes {
        workspace.updated_at = None;
    }
    let catalog =
        serde_json::to_string(&(&config.providers, &scopes)).map_err(|_| ConfigError::Invalid)?;
    Ok(digest(&format!("{file_hash}\0{catalog}")))
}

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "reload_activity_tests.rs"]
mod activity_tests;
