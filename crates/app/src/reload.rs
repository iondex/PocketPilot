use crate::{
    config::{self, Config, ConfigError},
    ledger::digest,
};
use arc_swap::ArcSwap;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

pub struct Snapshot {
    pub config: Config,
    pub hash: String,
}
pub struct ConfigFile {
    pub path: PathBuf,
    current: ArcSwap<Snapshot>,
}

impl ConfigFile {
    pub fn open(path: PathBuf) -> Result<Self, ConfigError> {
        let path = path.canonicalize().map_err(|_| ConfigError::Io)?;
        let current = Self::read(&path)?;
        Ok(Self {
            path,
            current: ArcSwap::from_pointee(current),
        })
    }
    fn read(path: &std::path::Path) -> Result<Snapshot, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|_| ConfigError::Io)?;
        let loaded = config::parse(path, &text)?;
        Ok(Snapshot {
            config: loaded.config,
            hash: digest(&text),
        })
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }
    /// Single-file atomic reload. Runtime-owned data directory and bot-account changes
    /// require a service restart, which only disconnects PocketPilot's transport.
    pub fn reload(&self) -> Result<bool, ConfigError> {
        let next = Self::read(&self.path)?;
        let old = self.snapshot();
        if next.hash == old.hash {
            return Ok(false);
        }
        if next.config.service.data_dir != old.config.service.data_dir {
            return Err(ConfigError::Invalid);
        }
        if next.config.telegram.as_ref().map(|t| &t.token)
            != old.config.telegram.as_ref().map(|t| &t.token)
        {
            return Err(ConfigError::Invalid);
        }
        if next.config.discord.as_ref().map(|d| (&d.token, d.guild_id))
            != old.config.discord.as_ref().map(|d| (&d.token, d.guild_id))
        {
            return Err(ConfigError::Invalid);
        }
        self.current.store(Arc::new(next));
        Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bad_reload_keeps_the_authoritative_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocketpilot.toml");
        std::fs::write(&path, "[delivery]\nburst_threshold=3").unwrap();
        let config = ConfigFile::open(path.clone()).unwrap();
        let original = config.snapshot();
        std::fs::write(&path, "include=['other.toml']").unwrap();
        assert!(config.reload().is_err());
        assert_eq!(config.snapshot().hash, original.hash);
        std::fs::write(&path, "[delivery]\nburst_threshold=5").unwrap();
        assert!(config.reload().unwrap());
        assert_eq!(config.snapshot().config.delivery.burst_threshold, 5);
    }
}
