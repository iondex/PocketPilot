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
    #[error("Telegram credentials are missing or invalid (values are redacted)")]
    Credentials,
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub sources: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub service: Service,
    #[serde(default)]
    pub logging: Logging,
    pub telegram: Option<Telegram>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub config_reload: Reload,
    #[serde(default)]
    pub delivery: Delivery,
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
}
impl Default for Logging {
    fn default() -> Self {
        Self {
            format: "json".into(),
            filter: "info".into(),
            directory: None,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Telegram {
    pub token: String,
}
impl std::fmt::Debug for Telegram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Telegram([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    LocalProxy,
    SshProxy,
    Websocket,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshBackend {
    #[default]
    Auto,
    Openssh,
    Russh,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub transport: TransportKind,
    #[serde(default)]
    pub backend: SshBackend,
    pub ssh_alias: Option<String>,
    pub ssh_config: Option<PathBuf>,
    #[serde(default = "default_codex")]
    pub codex_bin: String,
    pub codex_control_socket: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub allowed_workspace_roots: Vec<String>,
    #[serde(default)]
    pub timeouts: Timeouts,
    #[serde(default)]
    pub authentication: Authentication,
}
fn default_codex() -> String {
    "codex".into()
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

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Authentication {
    pub methods: Vec<String>,
    pub private_key_passphrase: Option<String>,
    pub password: Option<String>,
    pub max_attempts: u32,
    pub totp: Option<Totp>,
}
impl Default for Authentication {
    fn default() -> Self {
        Self {
            methods: vec![],
            private_key_passphrase: None,
            password: None,
            max_attempts: 2,
            totp: None,
        }
    }
}
impl std::fmt::Debug for Authentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authentication([REDACTED])")
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Totp {
    pub secret_base32: String,
    pub algorithm: String,
    pub digits: usize,
    pub period_seconds: u64,
    pub minimum_remaining_seconds: u64,
    pub prompt_patterns: Vec<String>,
    #[serde(default)]
    pub password_prompt_patterns: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub host_id: String,
    pub kind: String,
    pub connection_mode: String,
    pub compatibility_profile: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub short_name: String,
    pub cwd: String,
    #[serde(default)]
    pub additional_cwds: Vec<String>,
}

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
    pub telegram_messages_per_minute: u32,
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
            telegram_messages_per_minute: 18,
            history_active: "full_bundle".into(),
            history_idle: "last_user_and_final".into(),
            overflow: "single_document".into(),
        }
    }
}

pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let path = path.canonicalize().map_err(|_| ConfigError::Io)?;
    let source = std::fs::read_to_string(&path).map_err(|_| ConfigError::Io)?;
    let mut config: Config = toml::from_str(&source).map_err(|_| ConfigError::Parse)?;
    let base = path.parent().ok_or(ConfigError::Invalid)?;
    config.service.data_dir = absolute(base, &config.service.data_dir);
    config.logging.directory = config.logging.directory.as_ref().map(|p| absolute(base, p));
    for host in &mut config.hosts {
        host.ssh_config = host.ssh_config.as_ref().map(|p| absolute(base, p));
    }
    config.validate()?;
    Ok(LoadedConfig {
        config,
        sources: vec![path],
    })
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
    pub fn validate(&self) -> Result<(), ConfigError> {
        unique(self.hosts.iter().map(|h| h.id.as_str()))?;
        unique(self.providers.iter().map(|p| p.id.as_str()))?;
        unique(self.workspaces.iter().map(|w| w.id.as_str()))?;
        for host in &self.hosts {
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
                || host.codex_bin.trim().is_empty()
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
                TransportKind::LocalProxy => {
                    if !host
                        .codex_control_socket
                        .as_deref()
                        .is_some_and(valid_absolute)
                    {
                        return Err(ConfigError::Invalid);
                    }
                }
                TransportKind::SshProxy => {
                    if !host.ssh_alias.as_deref().is_some_and(|s| {
                        !s.starts_with('-')
                            && !s.is_empty()
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    }) {
                        return Err(ConfigError::Invalid);
                    }
                    if host
                        .codex_control_socket
                        .as_deref()
                        .is_some_and(|s| !valid_absolute(s))
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
        if self
            .telegram
            .as_ref()
            .is_some_and(|tg| tg.token.trim().is_empty())
        {
            return Err(ConfigError::Credentials);
        }
        let d = &self.delivery;
        if d.burst_window_ms == 0
            || d.burst_threshold == 0
            || d.max_flush_delay_ms < d.burst_window_ms
            || d.history_barrier_timeout_seconds == 0
            || d.telegram_messages_per_minute == 0
            || d.history_active != "full_bundle"
            || d.history_idle != "last_user_and_final"
            || d.overflow != "single_document"
            || self.config_reload.periodic_rescan_seconds == 0
            || !matches!(self.logging.format.as_str(), "json" | "text")
        {
            return Err(ConfigError::Invalid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn paths_are_relative_to_the_single_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocketpilot.toml");
        fs::write(&path, "[service]\ndata_dir='state'\n").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded.config.service.data_dir,
            dir.path().canonicalize().unwrap().join("state")
        );
        assert_eq!(loaded.sources.len(), 1);
    }
    #[test]
    fn parse_errors_and_debug_output_do_not_echo_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocketpilot.toml");
        fs::write(
            &path,
            "[telegram]\ntoken='private-value'\nunknown='private-value'",
        )
        .unwrap();
        assert!(
            !load(&path)
                .unwrap_err()
                .to_string()
                .contains("private-value")
        );
        assert!(
            !format!(
                "{:?}",
                Telegram {
                    token: "private-value".into()
                }
            )
            .contains("private-value")
        );
    }
    #[test]
    fn includes_and_configured_user_chat_allowlists_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocketpilot.toml");
        for input in [
            "include=[]",
            "[telegram]\ntoken='t'\nallowed_user_ids=[]",
            "[telegram]\ntoken='t'\nallowed_chat_ids=[]",
        ] {
            fs::write(&path, input).unwrap();
            assert!(matches!(load(&path), Err(ConfigError::Parse)));
        }
    }
    #[test]
    fn public_example_is_a_valid_configuration() {
        load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pocketpilot.example.toml"))
            .unwrap();
    }
}
