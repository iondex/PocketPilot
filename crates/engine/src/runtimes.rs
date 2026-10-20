//! Shared runtime clients, observation budgets and per-session execution serialization.
use crate::config::{Config, Host, Provider, TransportKind, Workspace};
use adapters::{Adapter, CodexAdapter};
use connections::{Endpoint, Timeouts};
use session::AgentProvider;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};

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
    Rpc(#[from] codex::Error),
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
    client: Option<Arc<CodexAdapter>>,
    retry_at: Instant,
    failures: u32,
}

type SessionActors = HashMap<(String, String), Arc<Mutex<()>>>;

pub struct RuntimeRegistry {
    store: crate::storage::Store,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
    identities: Mutex<HashMap<String, String>>,
    actors: Mutex<SessionActors>,
    history_limits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl RuntimeRegistry {
    pub fn new(store: crate::storage::Store) -> Self {
        Self {
            store,
            entries: Mutex::new(HashMap::new()),
            identities: Mutex::new(HashMap::new()),
            actors: Mutex::new(HashMap::new()),
            history_limits: Mutex::new(HashMap::new()),
        }
    }

    pub async fn actor(&self, instance: &str, thread: &str) -> Arc<Mutex<()>> {
        self.actors
            .lock()
            .await
            .entry((instance.into(), thread.into()))
            .or_default()
            .clone()
    }

    pub async fn history_limit(&self, instance: &str) -> Arc<Semaphore> {
        self.history_limits
            .lock()
            .await
            .entry(instance.into())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    pub async fn get(
        &self,
        config: &Config,
        workspace: &Workspace,
    ) -> Result<Arc<dyn AgentProvider>, Error> {
        tracing::debug!(workspace_id=%workspace.id,provider_id=%workspace.provider_id,"requesting shared provider connection");
        self.get_provider(config, &workspace.provider_id).await
    }

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
        let mut entry = entry.lock().await;
        if let Some(client) = &entry.client {
            if client.is_connected() {
                if client.capabilities().control == provider.control_verified {
                    return Ok(client.provider());
                }
                let client = Arc::new((**client).clone().with_control(provider.control_verified));
                entry.client = Some(client.clone());
                return Ok(client.provider());
            }
            entry.client = None;
        }
        if entry.retry_at > Instant::now() {
            return Err(Error::Backoff);
        }
        let mut effective = identity.clone();
        if matches!(host.transport, TransportKind::SSHProxy) {
            let resolved = ssh::resolve(
                host.ssh_alias.as_deref().ok_or(Error::Selection)?,
                host.ssh_config.as_ref(),
                Duration::from_secs(host.timeouts.connect_seconds),
            )
            .await
            .map_err(|_| codex::Error::Endpoint)?;
            effective = crate::digest(&format!(
                "{identity}\0{}\0{}\0{}",
                resolved.hostname, resolved.user, resolved.port
            ));
        }
        self.store
            .check_instance(&provider.id, &effective, false)
            .await?;
        match connect(host, provider, config).await {
            Ok(client) => {
                tracing::info!(host_id=%host.id,provider_id=%provider.id,"connected to shared app-server");
                self.store
                    .check_instance(&provider.id, &effective, true)
                    .await?;
                self.identities
                    .lock()
                    .await
                    .insert(provider.id.clone(), identity);
                let client = Arc::new(CodexAdapter::new(
                    codex::Codex::new(provider.id.clone(), client)
                        .with_control(provider.control_verified),
                ));
                entry.client = Some(client.clone());
                entry.failures = 0;
                Ok(client.provider())
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

pub fn fingerprint(host: &Host, provider: &Provider) -> String {
    crate::digest(&format!(
        "{}\0{:?}\0{:?}\0{:?}\0{:?}\0{}",
        // Keep persisted identities compatible with the original enum Debug spelling.
        match host.transport {
            TransportKind::LocalProxy => "LocalProxy",
            TransportKind::SSHProxy => "SshProxy",
            TransportKind::Websocket => "Websocket",
        },
        host.ssh_alias,
        host.ssh_config,
        provider.endpoint.as_ref().map(|e| &e.socket),
        host.endpoint,
        provider
            .endpoint
            .as_ref()
            .map_or("codex", |e| e.binary.as_str())
    ))
}

pub async fn connect(
    host: &Host,
    provider: &Provider,
    config: &Config,
) -> Result<codex::Client, codex::Error> {
    let endpoint = match host.transport {
        TransportKind::LocalProxy => Endpoint::LocalProxy {
            binary: provider
                .endpoint
                .as_ref()
                .ok_or(codex::Error::Endpoint)?
                .binary
                .clone(),
            socket: provider
                .endpoint
                .as_ref()
                .ok_or(codex::Error::Endpoint)?
                .socket
                .clone()
                .into(),
        },
        TransportKind::Websocket => Endpoint::Websocket {
            endpoint: host.endpoint.clone().ok_or(codex::Error::Endpoint)?,
        },
        TransportKind::SSHProxy => Endpoint::SshProxy(codex::transport::SSHOptions {
            alias: host.ssh_alias.clone().ok_or(codex::Error::Endpoint)?,
            config: host.ssh_config.clone(),
            binary: provider
                .endpoint
                .as_ref()
                .ok_or(codex::Error::Endpoint)?
                .binary
                .clone(),
            socket: Some(
                provider
                    .endpoint
                    .as_ref()
                    .ok_or(codex::Error::Endpoint)?
                    .socket
                    .clone(),
            ),
            connect_timeout: Duration::from_secs(host.timeouts.connect_seconds),
            keepalive_seconds: host.timeouts.keepalive_seconds,
            keepalive_missed: host.timeouts.keepalive_missed,
            askpass: ssh_options(host, config)?.askpass,
            max_auth_attempts: host.authentication.max_attempts,
            authentication_methods: host.authentication.methods.clone(),
        }),
    };
    connections::connect(
        endpoint,
        Timeouts {
            connect: Duration::from_secs(
                host.timeouts
                    .connect_seconds
                    .saturating_add(host.timeouts.authenticate_seconds)
                    .saturating_add(host.timeouts.proxy_ready_seconds),
            ),
            read: Duration::from_secs(host.timeouts.rpc_read_seconds),
            write_receipt: Duration::from_secs(host.timeouts.rpc_write_receipt_seconds),
        },
    )
    .await
    .map_err(|error| match error {
        connections::Error::Codex(error) => error,
        connections::Error::Endpoint => codex::Error::Endpoint,
    })
}

pub fn require_workspace(cwd: &str, workspace: &Workspace) -> Result<(), Error> {
    if cwd == workspace.cwd || workspace.additional_cwds.iter().any(|p| p == cwd) {
        Ok(())
    } else {
        Err(Error::Selection)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkspaceCursor {
    workspace: String,
    paths: Vec<String>,
    index: usize,
    backend: Option<String>,
}

/// Page through the explicitly configured cwd and worktrees without broadening
/// the provider query to unrelated projects.
pub async fn list_workspace(
    provider: &dyn AgentProvider,
    workspace: &Workspace,
    cursor: Option<String>,
    limit: u32,
) -> Result<session::Page<session::Session>, session::ProviderError> {
    use session::ProviderError;
    let mut paths = vec![workspace.cwd.clone()];
    for cwd in &workspace.additional_cwds {
        if !paths.contains(cwd) {
            paths.push(cwd.clone());
        }
    }
    let mut cursor = match cursor {
        Some(value) => serde_json::from_str::<WorkspaceCursor>(
            value
                .strip_prefix("ws1:")
                .ok_or(ProviderError::Incompatible)?,
        )
        .map_err(|_| ProviderError::Incompatible)?,
        None => WorkspaceCursor {
            workspace: workspace.id.clone(),
            paths: paths.clone(),
            index: 0,
            backend: None,
        },
    };
    if cursor.workspace != workspace.id || cursor.paths != paths || cursor.index >= paths.len() {
        return Err(ProviderError::Incompatible);
    }
    let limit = limit.clamp(1, 100) as usize;
    let mut data = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while data.len() < limit && cursor.index < paths.len() {
        if !seen.insert((cursor.index, cursor.backend.clone())) {
            return Err(ProviderError::Incompatible);
        }
        let page = provider
            .list(session::SessionQuery {
                cwd: Some(paths[cursor.index].clone()),
                cursor: cursor.backend.clone(),
                limit: (limit - data.len()) as u32,
                archived: false,
            })
            .await?;
        data.extend(page.data);
        cursor.backend = page.next_cursor;
        if cursor.backend.is_none() {
            cursor.index += 1;
        }
    }
    let next_cursor = if cursor.index < paths.len() {
        Some(format!(
            "ws1:{}",
            serde_json::to_string(&cursor).map_err(|_| ProviderError::Incompatible)?
        ))
    } else {
        None
    };
    Ok(session::Page { data, next_cursor })
}

/// Hide only dedicated probes proven by this bridge's durable create receipts.
/// Never infer a test session from its title or mutate/archive it remotely.
pub async fn list_picker(
    store: &crate::storage::Store,
    provider: &dyn AgentProvider,
    workspace: &Workspace,
    cursor: Option<String>,
    limit: u32,
) -> Result<session::Page<session::Session>, session::ProviderError> {
    session_picker::list(store, provider, workspace, cursor, limit).await
}

#[path = "session_picker.rs"]
mod session_picker;

pub fn ssh_options(host: &Host, config: &Config) -> Result<ssh::Options, codex::Error> {
    Ok(ssh::Options {
        alias: host.ssh_alias.clone().ok_or(codex::Error::Endpoint)?,
        config: host.ssh_config.clone(),
        connect_timeout: Duration::from_secs(host.timeouts.connect_seconds),
        keepalive_seconds: host.timeouts.keepalive_seconds,
        keepalive_missed: host.timeouts.keepalive_missed,
        askpass: ssh::auth::helper(
            &host.authentication,
            &host.id,
            &ssh::auth::AuthContext {
                data_dir: config.service.data_dir.clone(),
                source: config.source().to_owned(),
                source_hash: config.source_hash().into(),
            },
        )
        .map_err(|_| codex::Error::Unsupported)?,
        max_auth_attempts: host.authentication.max_attempts,
        authentication_methods: host.authentication.methods.clone(),
    })
}

pub async fn discover(
    host: &Host,
    config: &Config,
) -> Result<Vec<codex::discovery::Endpoint>, codex::Error> {
    let options = match host.transport {
        TransportKind::SSHProxy => Some(ssh_options(host, config)?),
        TransportKind::LocalProxy => None,
        TransportKind::Websocket => return Ok(vec![]),
    };
    let mut endpoints = codex::discovery::discover(
        options,
        Duration::from_secs(
            host.timeouts
                .connect_seconds
                .saturating_add(host.timeouts.authenticate_seconds)
                .saturating_add(host.timeouts.proxy_ready_seconds),
        ),
    )
    .await?;
    for endpoint in &mut endpoints {
        if let Some(existing) = config
            .providers
            .iter()
            .filter(|p| p.host_id == host.id)
            .filter_map(|p| p.endpoint.as_ref())
            .find(|e| e.socket == endpoint.socket)
        {
            // Keep a verified instance on its recorded proxy binary. Installing
            // another CLI version does not silently replace its control profile.
            endpoint.binary = existing.binary.clone();
        }
    }
    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Paths;

    #[async_trait::async_trait]
    impl AgentProvider for Paths {
        fn capabilities(&self) -> session::Capabilities {
            session::Capabilities::default()
        }

        async fn list(
            &self,
            q: session::SessionQuery,
        ) -> Result<session::Page<session::Session>, session::ProviderError> {
            let cwd = q.cwd.unwrap();
            Ok(session::Page {
                data: vec![session::Session {
                    reference: session::SessionRef {
                        instance: "fixture".into(),
                        thread: cwd.clone(),
                    },
                    title: cwd.clone(),
                    native_title: Some(cwd.clone()),
                    cwd,
                    status: session::RuntimeStatus::Idle,
                    active_turn: None,
                    updated_at: None,
                }],
                next_cursor: None,
            })
        }

        async fn read(&self, _: &str) -> Result<session::Session, session::ProviderError> {
            Err(session::ProviderError::Unsupported)
        }

        async fn history(&self, _: &str) -> Result<session::History, session::ProviderError> {
            Err(session::ProviderError::Unsupported)
        }
    }

    #[tokio::test]
    async fn explicit_worktrees_are_paged_and_old_path_set_cursors_are_rejected() {
        let mut ws = Workspace {
            id: "ws".into(),
            provider_id: "fixture".into(),
            name: "workspace".into(),
            short_name: "ws".into(),
            cwd: "/base".into(),
            additional_cwds: vec!["/worktree".into(), "/worktree".into()],
            updated_at: None,
        };
        let first = list_workspace(&Paths, &ws, None, 1).await.unwrap();
        assert_eq!(first.data[0].cwd, "/base");
        let second = list_workspace(&Paths, &ws, first.next_cursor.clone(), 1)
            .await
            .unwrap();
        assert_eq!(second.data[0].cwd, "/worktree");
        assert!(second.next_cursor.is_none());
        ws.additional_cwds = vec!["/changed".into()];
        assert!(
            list_workspace(&Paths, &ws, first.next_cursor, 1)
                .await
                .is_err()
        );
    }
}
