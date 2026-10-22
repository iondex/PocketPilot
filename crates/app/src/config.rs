//! Combine platform settings with host policy and identify changes that require restart.

pub use engine::config::{ConfigError, Host, Logging, TransportKind, Workspace};
pub use integrations::discord::config::Config as Discord;
pub use integrations::feishu::config::Config as Feishu;
pub use integrations::slack::config::Config as Slack;
pub use integrations::telegram::config::Config as Telegram;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(flatten)]
    pub runtime: engine::config::Config,
    pub telegram: Option<Telegram>,
    pub discord: Option<Discord>,
    pub slack: Option<Slack>,
    pub feishu: Option<Feishu>,
    /// Lark shares the credential schema; its table selects a separate client and scope.
    pub lark: Option<Feishu>,
}

impl std::ops::Deref for Config {
    type Target = engine::config::Config;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub sources: Vec<PathBuf>,
}

pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let path = path.canonicalize().map_err(|_| ConfigError::Io)?;
    let text = std::fs::read_to_string(&path).map_err(|_| ConfigError::Io)?;
    parse(&path, &text)
}

pub fn parse(path: &Path, text: &str) -> Result<LoadedConfig, ConfigError> {
    let mut config: Config = toml::from_str(text).map_err(|_| ConfigError::Parse)?;
    config.runtime.prepare(path, text)?;
    if [config.feishu.as_ref(), config.lark.as_ref()]
        .into_iter()
        .flatten()
        .any(|s| !s.valid())
    {
        return Err(ConfigError::Credentials);
    }
    if config.slack.as_ref().is_some_and(|s| !s.valid()) {
        return Err(ConfigError::Credentials);
    }
    if config.telegram.as_ref().is_some_and(|t| {
        t.token.trim().is_empty() || integrations::telegram::Owner::new(t.owner_user_id).is_none()
    }) || config
        .discord
        .as_ref()
        .is_some_and(|d| d.token.trim().is_empty() || d.guild_id == 0)
    {
        return Err(ConfigError::Credentials);
    }
    if config
        .telegram
        .as_ref()
        .is_some_and(|t| !matches!(t.default_locale.as_str(), "zh-CN" | "zh" | "en" | "auto"))
    {
        return Err(ConfigError::Invalid);
    }
    Ok(LoadedConfig {
        config,
        sources: vec![path.to_owned()],
    })
}

/// Fingerprint platform credentials and allowlists separately from reloadable engine state.
pub fn runtime_parser(
    path: &Path,
    text: &str,
) -> Result<engine::reload::ParsedConfig, ConfigError> {
    let loaded = parse(path, text)?;
    let root = loaded.config;
    let restart_key = engine::digest(&format!(
        "{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
        root.telegram.as_ref().map(|t| (
            &t.token,
            t.owner_user_id,
            &t.default_locale,
            t.proxy.restart_key()
        )),
        root.discord
            .as_ref()
            .map(|d| (&d.token, d.guild_id, d.proxy.restart_key())),
        root.slack.as_ref().map(|s| (
            &s.app_token,
            &s.bot_token,
            &s.app_id,
            &s.team_id,
            &s.owner_user_id,
            s.proxy.restart_key()
        )),
        root.feishu.as_ref().map(|s| (
            &s.app_id,
            &s.app_secret,
            &s.tenant_key,
            &s.owner_open_id,
            s.groups
                .iter()
                .map(|g| (&g.chat_id, &g.name))
                .collect::<Vec<_>>()
        )),
        root.lark.as_ref().map(|s| (
            &s.app_id,
            &s.app_secret,
            &s.tenant_key,
            &s.owner_open_id,
            s.groups
                .iter()
                .map(|g| (&g.chat_id, &g.name))
                .collect::<Vec<_>>()
        ))
    ));
    Ok(engine::reload::ParsedConfig {
        config: root.runtime,
        restart_key,
    })
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
