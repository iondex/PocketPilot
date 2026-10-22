//! Resolve configured runtime transports without owning the shared client registry.
//!
//! Configuration fingerprints keep existing persisted spellings. Changing their
//! encoding would invalidate bindings even when the execution account is unchanged.
use super::Error;
use crate::config::{Config, Host, Provider, TransportKind};
use adapters::{Adapter, AdapterFactory};
use connections::{Endpoint, Timeouts};
use std::{sync::Arc, time::Duration};

/// Hash the configured execution identity using the persisted, version-compatible encoding.
///
/// A control-grant change is deliberately excluded: it changes permissions, not
/// the runtime endpoint. Registry reconnects additionally verify the resolved SSH account.
pub fn fingerprint(host: &Host, provider: &Provider) -> String {
    let identity = format!(
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
    );
    crate::digest(&if provider.kind == "codex" {
        identity
    } else {
        format!("{identity}\0{}", provider.kind)
    })
}

/// Bind an SSH alias to the account it currently resolves to before consulting
/// the durable identity ledger. Editing ssh_config cannot silently retarget a provider.
pub(super) async fn effective_identity(host: &Host, configured: &str) -> Result<String, Error> {
    if host.transport != TransportKind::SSHProxy {
        return Ok(configured.into());
    }
    let resolved = connections::ssh::resolve(
        host.ssh_alias.as_deref().ok_or(Error::Selection)?,
        host.ssh_config.as_ref(),
        Duration::from_secs(host.timeouts.connect_seconds),
    )
    .await
    .map_err(|_| adapters::codex::Error::Endpoint)?;
    Ok(crate::digest(&format!(
        "{configured}\0{}\0{}\0{}",
        resolved.hostname, resolved.user, resolved.port
    )))
}

fn factory(kind: &str) -> Result<&'static dyn AdapterFactory, session::ProviderError> {
    match kind {
        "codex" => Ok(&adapters::codex::Factory),
        "claude_code" => Ok(&adapters::claude::Factory),
        _ => Err(session::ProviderError::Unsupported),
    }
}

/// Validated connection settings; constructing a plan performs no network I/O.
/// Keeping preparation separate prevents configuration errors from entering the
/// registry's transport reconnect backoff.
pub(super) struct ConnectionPlan {
    factory: &'static dyn AdapterFactory,
    options: adapters::ConnectOptions,
}

impl ConnectionPlan {
    pub(super) async fn connect(self) -> Result<Arc<dyn Adapter>, session::ProviderError> {
        self.factory.connect(self.options).await
    }
}

pub(super) fn prepare(
    host: &Host,
    provider: &Provider,
    config: &Config,
) -> Result<ConnectionPlan, Error> {
    Ok(ConnectionPlan {
        factory: factory(&provider.kind)?,
        options: adapters::ConnectOptions {
            instance: provider.id.clone(),
            endpoint: endpoint(host, provider, config)?,
            timeouts: timeouts(host),
            control_verified: provider.control_verified,
        },
    })
}

/// One read-only diagnostic connection; it never installs or starts a runtime server.
pub async fn connect_adapter(
    host: &Host,
    provider: &Provider,
    config: &Config,
) -> Result<Arc<dyn Adapter>, Error> {
    Ok(prepare(host, provider, config)?.connect().await?)
}

/// Native RPC access for deployment diagnostics and explicitly authorized control probes.
pub async fn connect(
    host: &Host,
    provider: &Provider,
    config: &Config,
) -> Result<adapters::codex::Client, adapters::codex::Error> {
    if provider.kind != "codex" {
        return Err(adapters::codex::Error::Unsupported);
    }
    adapters::codex::connect(endpoint(host, provider, config)?, timeouts(host)).await
}

/// Resolve configuration into an existing transport; never discover or create a daemon here.
fn endpoint(
    host: &Host,
    provider: &Provider,
    config: &Config,
) -> Result<Endpoint, adapters::codex::Error> {
    Ok(match host.transport {
        TransportKind::Websocket if provider.kind != "codex" => {
            return Err(adapters::codex::Error::Unsupported);
        }
        TransportKind::Websocket => Endpoint::Websocket {
            endpoint: host
                .endpoint
                .clone()
                .ok_or(adapters::codex::Error::Endpoint)?,
        },
        TransportKind::LocalProxy | TransportKind::SSHProxy => {
            let endpoint = provider
                .endpoint
                .as_ref()
                .ok_or(adapters::codex::Error::Endpoint)?;
            let transport = match host.transport {
                TransportKind::SSHProxy => connections::Host::SSH(ssh_options(host, config)?),
                _ => connections::Host::Local,
            };
            match provider.kind.as_str() {
                "codex" => adapters::codex::proxy_endpoint(
                    transport,
                    endpoint.binary.clone(),
                    endpoint.socket.clone(),
                )?,
                "claude_code" => Endpoint::Proxy {
                    host: transport,
                    program: connections::Program {
                        binary: endpoint.binary.clone(),
                        arguments: vec!["proxy".into(), "--socket".into(), endpoint.socket.clone()],
                    },
                },
                _ => return Err(adapters::codex::Error::Unsupported),
            }
        }
    })
}

fn timeouts(host: &Host) -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(
            host.timeouts
                .connect_seconds
                .saturating_add(host.timeouts.authenticate_seconds)
                .saturating_add(host.timeouts.proxy_ready_seconds),
        ),
        read: Duration::from_secs(host.timeouts.rpc_read_seconds),
        write_receipt: Duration::from_secs(host.timeouts.rpc_write_receipt_seconds),
    }
}

/// Build OpenSSH options and the bounded authentication helper without logging in.
///
/// # Errors
///
/// Returns an endpoint error for a missing alias, or unsupported when the configured
/// authentication helper cannot be prepared. No runtime is started by this function.
pub fn ssh_options(
    host: &Host,
    config: &Config,
) -> Result<connections::ssh::Options, adapters::codex::Error> {
    Ok(connections::ssh::Options {
        alias: host
            .ssh_alias
            .clone()
            .ok_or(adapters::codex::Error::Endpoint)?,
        config: host.ssh_config.clone(),
        connect_timeout: Duration::from_secs(host.timeouts.connect_seconds),
        keepalive_seconds: host.timeouts.keepalive_seconds,
        keepalive_missed: host.timeouts.keepalive_missed,
        askpass: connections::ssh::auth::helper(
            &host.authentication,
            &host.id,
            &connections::ssh::auth::AuthContext {
                data_dir: config.service.data_dir.clone(),
                source: config.source().to_owned(),
                source_hash: config.source_hash().into(),
            },
        )
        .map_err(|_| adapters::codex::Error::Unsupported)?,
        max_auth_attempts: host.authentication.max_attempts,
        authentication_methods: host.authentication.methods.clone(),
    })
}

/// Discover existing Codex endpoints without resuming sessions or replacing proxies.
///
/// A recorded proxy binary is retained for an existing socket, so an unrelated CLI
/// upgrade cannot replace the protocol profile of a previously verified instance.
pub async fn discover(
    host: &Host,
    config: &Config,
) -> Result<Vec<adapters::ServerEndpoint>, adapters::codex::Error> {
    let options = match host.transport {
        TransportKind::SSHProxy => Some(ssh_options(host, config)?),
        TransportKind::LocalProxy => None,
        TransportKind::Websocket => return Ok(vec![]),
    };
    let mut endpoints = adapters::codex::discovery::discover(
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
