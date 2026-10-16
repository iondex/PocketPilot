pub use discord::config::Config as Discord;
pub use engine::config::{ConfigError, Host, Logging, TransportKind, Workspace};
use serde::Deserialize;
use std::path::{Path, PathBuf};
pub use telegram::config::Config as Telegram;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(flatten)]
    pub runtime: engine::config::Config,
    pub telegram: Option<Telegram>,
    pub discord: Option<Discord>,
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
    if config
        .telegram
        .as_ref()
        .is_some_and(|t| t.token.trim().is_empty())
        || config
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

pub fn runtime_parser(
    path: &Path,
    text: &str,
) -> Result<engine::reload::ParsedConfig, ConfigError> {
    let loaded = parse(path, text)?;
    let root = loaded.config;
    let restart_key = engine::digest(&format!(
        "{:?}\0{:?}",
        root.telegram
            .as_ref()
            .map(|t| (&t.token, &t.default_locale)),
        root.discord.as_ref().map(|d| (&d.token, d.guild_id))
    ));
    Ok(engine::reload::ParsedConfig {
        config: root.runtime,
        restart_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn language_is_an_explicit_root_setting_with_a_chinese_default() {
        let path = std::path::Path::new("/tmp/pocketpilot.toml");
        assert_eq!(parse(path, "").unwrap().config.language, "zh-CN");
        assert_eq!(parse(path, "language='en'").unwrap().config.language, "en");
        assert!(parse(path, "language='fr'").is_err());
    }

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
                    token: "private-value".into(),
                    default_locale: "zh-CN".into()
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
            "[[providers]]\nid='obsolete'",
            "[[workspaces]]\nid='obsolete'",
            "[[hosts]]\nid='h'\nname='h'\ntransport='ssh_proxy'\nssh_alias='h'\ncodex_bin='obsolete'",
            "[[hosts]]\nid='h'\nname='h'\ntransport='ssh_proxy'\nssh_alias='h'\ncodex_control_socket='/obsolete.sock'",
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
