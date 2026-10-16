use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration file cannot be read")]
    Io,
    #[error("configuration has invalid TOML or unknown fields")]
    Parse,
    #[error("configuration contains duplicate identifiers")]
    Duplicate,
    #[error("configuration references an unknown host or provider")]
    Reference,
    #[error("configuration has an invalid path, identifier, timeout or limit")]
    Invalid,
    #[error("integration credentials are missing or invalid (values are redacted)")]
    Credentials,
}

impl ::i18n::LocalizedError for ConfigError {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Io => ::i18n::Message::new("error.config.io"),
            Self::Parse => ::i18n::Message::new("error.config.parse"),
            Self::Duplicate => ::i18n::Message::new("error.config.duplicate"),
            Self::Reference => ::i18n::Message::new("error.config.reference"),
            Self::Invalid => ::i18n::Message::new("error.config.invalid"),
            Self::Credentials => ::i18n::Message::new("error.config.credentials"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(skip)]
    source: PathBuf,
    #[serde(skip)]
    source_hash: String,
    #[serde(default)]
    pub service: Service,
    #[serde(default)]
    pub logging: Logging,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(skip)]
    pub providers: Vec<Provider>,
    #[serde(skip)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub config_reload: Reload,
    #[serde(default)]
    pub delivery: Delivery,
}

fn default_language() -> String {
    "zh-CN".into()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Service {
    pub data_dir: PathBuf,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            data_dir: "var/data".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    pub format: String,
    pub filter: String,
    pub directory: Option<PathBuf>,
    pub rotation: String,
    pub retained_files: usize,
    pub console: bool,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            format: "json".into(),
            filter: "info".into(),
            directory: None,
            rotation: "daily".into(),
            retained_files: 14,
            console: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    LocalProxy,
    #[serde(rename = "ssh_proxy")]
    SSHProxy,
    Websocket,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SSHBackend {
    #[default]
    Auto,
    #[serde(rename = "openssh")]
    OpenSSH,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub transport: TransportKind,
    #[serde(default)]
    pub backend: SSHBackend,
    pub ssh_alias: Option<String>,
    pub ssh_config: Option<PathBuf>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub allowed_workspace_roots: Vec<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub authentication: Authentication,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timeouts {
    pub connect_seconds: u64,
    pub authenticate_seconds: u64,
    pub proxy_ready_seconds: u64,
    pub rpc_read_seconds: u64,
    pub rpc_write_receipt_seconds: u64,
    pub keepalive_seconds: u64,
    pub keepalive_missed: u32,
    pub reconnect_initial_seconds: u64,
    pub reconnect_max_seconds: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect_seconds: 90,
            authenticate_seconds: 120,
            proxy_ready_seconds: 120,
            rpc_read_seconds: 60,
            rpc_write_receipt_seconds: 60,
            keepalive_seconds: 30,
            keepalive_missed: 6,
            reconnect_initial_seconds: 2,
            reconnect_max_seconds: 120,
        }
    }
}

pub use ssh::config::{Authentication, TOTP};

pub use crate::model::{Provider, Workspace};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reload {
    pub enabled: bool,
    pub debounce_ms: u64,
    pub periodic_rescan_seconds: u64,
}

impl Default for Reload {
    fn default() -> Self {
        Self {
            enabled: true,
            debounce_ms: 500,
            periodic_rescan_seconds: 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Delivery {
    pub burst_window_ms: u64,
    pub burst_threshold: usize,
    pub max_flush_delay_ms: u64,
    pub history_barrier_timeout_seconds: u64,
    #[serde(alias = "telegram_messages_per_minute")]
    pub messages_per_minute: u32,
    pub history_active: String,
    pub history_idle: String,
    pub overflow: String,
}

impl Default for Delivery {
    fn default() -> Self {
        Self {
            burst_window_ms: 2000,
            burst_threshold: 3,
            max_flush_delay_ms: 5000,
            history_barrier_timeout_seconds: 60,
            messages_per_minute: 18,
            history_active: "full_bundle".into(),
            history_idle: "last_user_and_final".into(),
            overflow: "single_document".into(),
        }
    }
}

fn absolute(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.into()
    } else {
        base.join(path)
    }
}

fn valid_absolute(path: &str) -> bool {
    !path.contains(['\0', '\n', '\r'])
        && Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir))
}

fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), ConfigError> {
    let mut seen = HashSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(ConfigError::Invalid);
        }
        if !seen.insert(id) {
            return Err(ConfigError::Duplicate);
        }
    }
    Ok(())
}

impl Config {
    pub fn prepare(&mut self, path: &Path, source: &str) -> Result<(), ConfigError> {
        self.source = path.to_owned();
        self.source_hash = crate::digest(source);
        let base = path.parent().ok_or(ConfigError::Invalid)?;
        self.service.data_dir = absolute(base, &self.service.data_dir);
        self.logging.directory = self.logging.directory.as_ref().map(|p| absolute(base, p));
        for host in &mut self.hosts {
            host.ssh_config = host.ssh_config.as_ref().map(|p| absolute(base, p));
        }
        self.validate()
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn source_hash(&self) -> &str {
        &self.source_hash
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if !matches!(self.language.as_str(), "zh-CN" | "zh" | "en") {
            return Err(ConfigError::Invalid);
        }
        unique(self.hosts.iter().map(|h| h.id.as_str()))?;
        unique(self.providers.iter().map(|p| p.id.as_str()))?;
        unique(self.workspaces.iter().map(|w| w.id.as_str()))?;
        unique(self.workspaces.iter().map(|w| w.short_name.as_str()))?;
        if self.workspaces.iter().any(|w| {
            self.workspaces
                .iter()
                .any(|other| w.id != other.id && w.short_name == other.id)
        }) {
            return Err(ConfigError::Duplicate);
        }
        for host in &self.hosts {
            ssh::auth::validate(&host.authentication).map_err(|_| ConfigError::Invalid)?;
            let t = &host.timeouts;
            if [
                t.connect_seconds,
                t.authenticate_seconds,
                t.proxy_ready_seconds,
                t.rpc_read_seconds,
                t.rpc_write_receipt_seconds,
                t.keepalive_seconds,
                t.reconnect_initial_seconds,
                t.reconnect_max_seconds,
            ]
            .contains(&0)
                || t.keepalive_missed == 0
                || t.reconnect_max_seconds < t.reconnect_initial_seconds
                || host.authentication.max_attempts == 0
            {
                return Err(ConfigError::Invalid);
            }
            if host
                .allowed_workspace_roots
                .iter()
                .any(|p| !valid_absolute(p))
            {
                return Err(ConfigError::Invalid);
            }
            match host.transport {
                TransportKind::LocalProxy => {}
                TransportKind::SSHProxy => {
                    if !host
                        .ssh_alias
                        .as_deref()
                        .is_some_and(ssh::resolve::valid_alias)
                    {
                        return Err(ConfigError::Invalid);
                    }
                }
                TransportKind::Websocket => {
                    if host.endpoint.as_deref().is_none_or(str::is_empty) {
                        return Err(ConfigError::Invalid);
                    }
                }
            }
            if let Some(totp) = &host.authentication.totp
                && (totp.secret_base32.is_empty()
                    || !matches!(totp.algorithm.as_str(), "SHA1" | "SHA256" | "SHA512")
                    || !matches!(totp.digits, 6 | 8)
                    || totp.period_seconds == 0
                    || totp.minimum_remaining_seconds >= totp.period_seconds
                    || totp.prompt_patterns.is_empty())
            {
                return Err(ConfigError::Invalid);
            }
        }
        for provider in &self.providers {
            if self.hosts.iter().all(|h| h.id != provider.host_id) {
                return Err(ConfigError::Reference);
            }
            if provider.kind != "codex"
                || provider.connection_mode != "existing_proxy"
                || provider.compatibility_profile.is_empty()
            {
                return Err(ConfigError::Invalid);
            }
        }
        for workspace in &self.workspaces {
            let provider = self
                .providers
                .iter()
                .find(|p| p.id == workspace.provider_id)
                .ok_or(ConfigError::Reference)?;
            let host = self
                .hosts
                .iter()
                .find(|h| h.id == provider.host_id)
                .ok_or(ConfigError::Reference)?;
            for cwd in std::iter::once(&workspace.cwd).chain(workspace.additional_cwds.iter()) {
                if !valid_absolute(cwd)
                    || !host
                        .allowed_workspace_roots
                        .iter()
                        .any(|root| Path::new(cwd).starts_with(root))
                {
                    return Err(ConfigError::Invalid);
                }
            }
        }
        let d = &self.delivery;
        if d.burst_window_ms == 0
            || d.burst_threshold == 0
            || d.max_flush_delay_ms < d.burst_window_ms
            || d.history_barrier_timeout_seconds == 0
            || d.messages_per_minute == 0
            || d.history_active != "full_bundle"
            || d.history_idle != "last_user_and_final"
            || d.overflow != "single_document"
            || self.config_reload.periodic_rescan_seconds == 0
            || !matches!(self.logging.format.as_str(), "json" | "text")
            || !matches!(self.logging.rotation.as_str(), "daily" | "hourly")
            || self.logging.retained_files == 0
        {
            return Err(ConfigError::Invalid);
        }
        Ok(())
    }
}
