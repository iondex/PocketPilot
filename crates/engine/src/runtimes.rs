//! Shared runtime clients, observation budgets and per-session execution serialization.
use crate::config::{Config, Workspace};
use adapters::Adapter;
use session::AgentProvider;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};

#[path = "runtime_connection.rs"]
mod connection;

#[path = "workspace_sessions.rs"]
mod workspace_sessions;

#[path = "session_picker.rs"]
mod session_picker;

pub use connection::{connect, connect_adapter, discover, fingerprint, ssh_options};
pub use workspace_sessions::{PickerPage, list_picker, list_workspace, require_workspace};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] crate::storage::StoreError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error("工作区已移除或未配置")]
    Selection,
    #[error("执行实例地址发生变化；请分配新的 provider ID 后重新绑定")]
    Identity,
    #[error("连接正在退避；稍后刷新即可，未自动发送任务")]
    Backoff,
    #[error(transparent)]
    Rpc(#[from] adapters::codex::Error),
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Selection => ::i18n::Message::new("error.connection.selection"),
            Self::Identity => ::i18n::Message::new("error.connection.identity"),
            Self::Backoff => ::i18n::Message::new("error.connection.backoff"),
            Self::Config(error) => ::i18n::LocalizedError::message(error),
            Self::Provider(error) => ::i18n::LocalizedError::message(error),
            Self::Store(error) => ::i18n::LocalizedError::message(error),
            Self::Rpc(error) => ::i18n::LocalizedError::message(error),
        }
    }
}

struct Entry {
    client: Option<Arc<dyn Adapter>>,
    retry_at: Instant,
    failures: u32,
}

type SessionActors = HashMap<(String, String), Arc<Mutex<()>>>;

/// Shared runtime connections and execution ordering for one bridge process.
///
/// Global maps protect metadata only. Connecting may await while holding an
/// individual provider entry; executing may await while holding a session actor.
/// Neither lock is a claim about other clients connected to the native runtime.
pub struct RuntimeRegistry {
    store: crate::storage::Store,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
    identities: Mutex<HashMap<String, String>>,
    actors: Mutex<SessionActors>,
    history_limits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl RuntimeRegistry {
    /// Create an empty registry sharing the service's durable identity ledger.
    pub fn new(store: crate::storage::Store) -> Self {
        Self {
            store,
            entries: Mutex::new(HashMap::new()),
            identities: Mutex::new(HashMap::new()),
            actors: Mutex::new(HashMap::new()),
            history_limits: Mutex::new(HashMap::new()),
        }
    }

    /// Return the shared execution mutex for this exact provider/session pair.
    ///
    /// Hold its guard across revalidation and dispatch, including network awaits.
    /// Dropping the guard releases serialization; it cannot undo a remote write.
    pub async fn actor(&self, instance: &str, thread: &str) -> Arc<Mutex<()>> {
        self.actors
            .lock()
            .await
            .entry((instance.into(), thread.into()))
            .or_default()
            .clone()
    }

    /// Return the per-provider permit used to serialize expensive history reads.
    /// Navigation metadata reads do not consume this permit.
    pub async fn history_limit(&self, instance: &str) -> Arc<Semaphore> {
        self.history_limits
            .lock()
            .await
            .entry(instance.into())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    /// Connect to the selected project's provider without starting a session.
    pub async fn get(
        &self,
        config: &Config,
        workspace: &Workspace,
    ) -> Result<Arc<dyn AgentProvider>, Error> {
        tracing::debug!(workspace_id=%workspace.id,provider_id=%workspace.provider_id,"requesting shared provider connection");
        self.get_provider(config, &workspace.provider_id).await
    }

    /// Reuse or connect one configured provider, checking its persisted identity.
    ///
    /// # Errors
    ///
    /// Returns selection/identity errors for changed configuration, [`Error::Backoff`]
    /// during a reconnect delay, or the original connection/storage error.
    ///
    /// # Cancellation
    ///
    /// Cancelling releases the provider lock; it does not terminate the native
    /// runtime or make a previously dispatched operation safe to resend.
    pub async fn get_provider(
        &self,
        config: &Config,
        id: &str,
    ) -> Result<Arc<dyn AgentProvider>, Error> {
        let provider = config
            .providers
            .iter()
            .find(|p| p.id == id)
            .ok_or(Error::Selection)?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or(Error::Selection)?;
        let identity = fingerprint(host, provider);
        {
            let identities = self.identities.lock().await;
            if identities
                .get(&provider.id)
                .is_some_and(|original| *original != identity)
            {
                return Err(Error::Identity);
            }
        }
        let entry = self
            .entries
            .lock()
            .await
            .entry(provider.id.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(Entry {
                    client: None,
                    retry_at: Instant::now(),
                    failures: 0,
                }))
            })
            .clone();
        // The map guard above has been dropped. Only this provider is blocked
        // during SSH resolution/handshake; unrelated instances remain usable.
        let mut entry = entry.lock().await;
        if let Some(client) = &entry.client {
            if client.is_connected() {
                if client.capabilities().control_verified == provider.control_verified {
                    return Ok(client.clone());
                }
                // Swap the immutable capability view while retaining the shared
                // transport. A grant change is not a reason to start another client.
                let client = client.with_control(provider.control_verified);
                entry.client = Some(client.clone());
                return Ok(client.clone());
            }
            entry.client = None;
        }
        if entry.retry_at > Instant::now() {
            return Err(Error::Backoff);
        }
        let effective = connection::effective_identity(host, &identity).await?;
        self.store
            .check_instance(&provider.id, &effective, false)
            .await?;
        let connection = connection::prepare(host, provider, config)?;
        match connection.connect().await {
            Ok(client) => {
                tracing::info!(host_id=%host.id,provider_id=%provider.id,"connected to shared app-server");
                self.store
                    .check_instance(&provider.id, &effective, true)
                    .await?;
                self.identities
                    .lock()
                    .await
                    .insert(provider.id.clone(), identity);
                entry.client = Some(client.clone());
                entry.failures = 0;
                Ok(client)
            }
            Err(error) => {
                entry.failures = entry.failures.saturating_add(1);
                let backoff = host
                    .timeouts
                    .reconnect_initial_seconds
                    .saturating_mul(1_u64 << entry.failures.min(10))
                    .min(host.timeouts.reconnect_max_seconds);
                // Small stable jitter avoids a synchronized reconnect wave across hosts.
                entry.retry_at = Instant::now()
                    + Duration::from_millis(
                        backoff.saturating_mul(1000)
                            + u64::from(
                                provider
                                    .id
                                    .bytes()
                                    .fold(0_u16, |a, b| a.wrapping_add(u16::from(b))),
                            ) % 1000,
                    );
                tracing::warn!(host_id=%host.id,provider_id=%provider.id,retry_seconds=backoff,%error,"provider connection failed; reads will back off");
                Err(error.into())
            }
        }
    }
}

#[cfg(test)]
#[path = "runtimes_tests.rs"]
mod tests;
