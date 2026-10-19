pub use discord::config::Config as Discord;
pub use engine::config::{ConfigError, Host, Logging, TransportKind, Workspace};
use serde::Deserialize;
pub use slack::config::Config as Slack;
use std::path::{Path, PathBuf};
pub use telegram::config::Config as Telegram;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(flatten)]
    pub runtime: engine::config::Config,
    pub telegram: Option<Telegram>,
    pub discord: Option<Discord>,
    pub slack: Option<Slack>,
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
    if config.slack.as_ref().is_some_and(|s| !s.valid()) {
        return Err(ConfigError::Credentials);
    }
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
        "{:?}\0{:?}\0{:?}",
        root.telegram
            .as_ref()
            .map(|t| (&t.token, &t.default_locale, t.proxy.restart_key())),
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
        ))
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
                    default_locale: "zh-CN".into(),
                    proxy: Default::default()
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
    fn channel_proxies_default_to_direct_validate_and_require_restart() {
        let path = Path::new("/tmp/pocketpilot.toml");
        for text in [
            "[telegram]\ntoken='fixture'",
            "[discord]\ntoken='fixture'\nguild_id=1",
            "[slack]\napp_token='xapp-fixture'\nbot_token='xoxb-fixture'\napp_id='A1'\nteam_id='T1'\nowner_user_id='U1'",
        ] {
            let before = runtime_parser(path, text).unwrap();
            let empty = runtime_parser(path, &format!("{text}\nproxy='' ")).unwrap();
            assert_eq!(before.restart_key, empty.restart_key);
            let first =
                runtime_parser(path, &format!("{text}\nproxy='http://192.0.2.10:7890'")).unwrap();
            let second =
                runtime_parser(path, &format!("{text}\nproxy='http://192.0.2.11:7890'")).unwrap();
            assert_ne!(before.restart_key, first.restart_key);
            assert_ne!(first.restart_key, second.restart_key);
            assert!(parse(path, &format!("{text}\nproxy='invalid-secret'")).is_err());
        }
    }

    #[test]
    fn public_example_is_a_valid_configuration() {
        load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pocketpilot.example.toml"))
            .unwrap();
    }

    #[test]
    fn slack_account_scope_is_validated_redacted_and_requires_restart() {
        let path = Path::new("/tmp/pocketpilot.toml");
        let text = "[slack]\napp_token='xapp-fixture-secret'\nbot_token='xoxb-fixture-secret'\napp_id='A1'\nteam_id='T1'\nowner_user_id='U1'\n";
        let loaded = parse(path, text).unwrap();
        let debug = format!("{:?}", loaded.config);
        assert!(!debug.contains("fixture-secret"));
        assert!(loaded.config.slack.unwrap().credentials_valid());
        assert!(parse(path, &format!("{text}\nallowed_channel_ids=['C1']")).is_err());
        assert!(
            parse(
                path,
                &text.replace("owner_user_id='U1'", "owner_user_id='C1'")
            )
            .is_err()
        );
        let before = runtime_parser(path, text).unwrap();
        let after = runtime_parser(
            path,
            &text.replace("owner_user_id='U1'", "owner_user_id='U3'"),
        )
        .unwrap();
        assert_ne!(before.restart_key, after.restart_key);
    }
}
