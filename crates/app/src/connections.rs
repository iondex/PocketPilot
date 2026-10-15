use crate::config::{Config, Host, TransportKind, Workspace};
use codex::{Client, Codex, Connection, SshOptions};
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
    #[error("工作区已移除或未配置")]
    Selection,
    #[error("执行实例地址发生变化；请分配新的 provider ID 后重新绑定")]
    Identity,
    #[error("连接正在退避；稍后刷新即可，未自动发送任务")]
    Backoff,
    #[error(transparent)]
    Rpc(#[from] codex::Error),
}
struct Entry {
    client: Option<Arc<Codex>>,
    retry_at: Instant,
    failures: u32,
}
type SessionActors = HashMap<(String, String), Arc<Mutex<()>>>;
pub struct Connections {
    store: crate::storage::Store,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
    identities: Mutex<HashMap<String, String>>,
    actors: Mutex<SessionActors>,
    history_limits: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl Connections {
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
    pub async fn get(&self, config: &Config, workspace: &Workspace) -> Result<Arc<Codex>, Error> {
        let provider = config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .ok_or(Error::Selection)?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or(Error::Selection)?;
        let identity = fingerprint(host);
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
            if client.client().is_connected() {
                if client.capabilities().control_verified == provider.control_verified {
                    return Ok(client.clone());
                }
                let client = Arc::new((**client).clone().with_control(provider.control_verified));
                entry.client = Some(client.clone());
                return Ok(client);
            }
            entry.client = None;
        }
        if entry.retry_at > Instant::now() {
            return Err(Error::Backoff);
        }
        let mut effective = identity.clone();
        if matches!(host.transport, TransportKind::SshProxy) {
            let resolved = codex::ssh::resolve(
                host.ssh_alias.as_deref().ok_or(Error::Selection)?,
                host.ssh_config.as_ref(),
                Duration::from_secs(host.timeouts.connect_seconds),
            )
            .await?;
            effective = crate::ledger::digest(&format!(
                "{identity}\0{}\0{}\0{}",
                resolved.hostname, resolved.user, resolved.port
            ));
        }
        self.store
            .check_instance(&provider.id, &effective, false)
            .await?;
        match connect(host, config).await {
            Ok(client) => {
                self.store
                    .check_instance(&provider.id, &effective, true)
                    .await?;
                self.identities
                    .lock()
                    .await
                    .insert(provider.id.clone(), identity);
                let client = Arc::new(
                    Codex::new(provider.id.clone(), client).with_control(provider.control_verified),
                );
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
                Err(error.into())
            }
        }
    }
}

pub fn fingerprint(host: &Host) -> String {
    crate::ledger::digest(&format!(
        "{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{}",
        host.transport,
        host.ssh_alias,
        host.ssh_config,
        host.codex_control_socket,
        host.endpoint,
        host.codex_bin
    ))
}

pub async fn connect(host: &Host, config: &Config) -> Result<Client, codex::Error> {
    let connection = match host.transport {
        TransportKind::LocalProxy => Connection::LocalProxy {
            binary: host.codex_bin.clone(),
            socket: host
                .codex_control_socket
                .as_ref()
                .ok_or(codex::Error::Endpoint)?
                .into(),
        },
        TransportKind::Websocket => Connection::Websocket {
            endpoint: host.endpoint.clone().ok_or(codex::Error::Endpoint)?,
        },
        TransportKind::SshProxy => Connection::SshProxy(SshOptions {
            alias: host.ssh_alias.clone().ok_or(codex::Error::Endpoint)?,
            config: host.ssh_config.clone(),
            binary: host.codex_bin.clone(),
            socket: host.codex_control_socket.clone(),
            connect_timeout: Duration::from_secs(host.timeouts.connect_seconds),
            keepalive_seconds: host.timeouts.keepalive_seconds,
            keepalive_missed: host.timeouts.keepalive_missed,
            askpass: crate::otp::helper(host, config).map_err(|_| codex::Error::Unsupported)?,
            max_auth_attempts: host.authentication.max_attempts,
            authentication_methods: host.authentication.methods.clone(),
        }),
    };
    Client::connect(
        connection,
        Duration::from_secs(
            host.timeouts
                .connect_seconds
                .saturating_add(host.timeouts.authenticate_seconds)
                .saturating_add(host.timeouts.proxy_ready_seconds),
        ),
        Duration::from_secs(host.timeouts.rpc_read_seconds),
    )
    .await
    .map(|client| {
        client.with_write_timeout(Duration::from_secs(host.timeouts.rpc_write_receipt_seconds))
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
    mut cursor: Option<String>,
    limit: u32,
) -> Result<session::Page<session::Session>, session::ProviderError> {
    let hidden = store
        .probe_threads(&workspace.provider_id)
        .await
        .map_err(|_| session::ProviderError::Disconnected)?;
    let mut data = Vec::new();
    let limit = limit.clamp(1, 100) as usize;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(cursor.clone()) {
            return Err(session::ProviderError::Incompatible);
        }
        let page = list_workspace(provider, workspace, cursor, (limit - data.len()) as u32).await?;
        data.extend(
            page.data
                .into_iter()
                .filter(|s| !hidden.contains(&s.reference.thread)),
        );
        cursor = page.next_cursor;
        if data.len() >= limit || cursor.is_none() {
            break;
        }
    }
    Ok(session::Page {
        data,
        next_cursor: cursor,
    })
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
