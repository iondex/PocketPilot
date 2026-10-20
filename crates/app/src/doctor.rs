//! Configuration diagnostics and explicit read-only connection probes.
use crate::cli::Error;
use app::config::{self, Config, Host};
use engine::catalog::Catalog;
use serde::Serialize;
use session::{AgentProvider, SessionQuery};
use std::time::Duration;
use telegram::credentials;

/// Offline validation is the default; each network probe is explicit.
#[derive(clap::Args, Clone, Copy)]
pub(super) struct Options {
    /// Also check configured credentials offline (never prints their values).
    #[arg(long)]
    pub(super) credentials: bool,
    #[arg(long)]
    pub(super) connect: bool,
    #[arg(long)]
    pub(super) telegram: bool,
    /// Read Discord bot/application/guild identity without registering commands.
    #[arg(long)]
    pub(super) discord: bool,
}

#[derive(Serialize)]
struct ConfigurationSummary {
    hosts: usize,
    providers: usize,
    workspaces: usize,
    telegram_configured: bool,
    discord_configured: bool,
    discord_runtime_enabled: bool,
    slack_configured: bool,
    slack_runtime_enabled: bool,
    credentials_checked: bool,
    control_enabled: bool,
}

fn configuration_summary(config: &Config, check: bool) -> Result<ConfigurationSummary, Error> {
    if check && let Some(telegram) = &config.telegram {
        let _ = credentials::read(telegram)?;
    }
    if check
        && config
            .slack
            .as_ref()
            .is_some_and(|s| !s.credentials_valid())
    {
        return Err(config::ConfigError::Credentials.into());
    }
    // Discord checks non-empty credentials and guild ID during config loading;
    // token validity and permissions require a real Discord connection.
    Ok(ConfigurationSummary {
        hosts: config.hosts.len(),
        providers: config.providers.len(),
        workspaces: config.workspaces.len(),
        telegram_configured: config.telegram.is_some(),
        discord_configured: config.discord.is_some(),
        discord_runtime_enabled: cfg!(feature = "discord"),
        slack_configured: config.slack.is_some(),
        slack_runtime_enabled: cfg!(feature = "slack"),
        credentials_checked: check,
        control_enabled: config.providers.iter().any(|p| p.control_verified),
    })
}

#[derive(Serialize)]
struct HostCheck {
    id: String,
    name: String,
    ssh_alias: Option<String>,
    configured: bool,
    sample_sessions: Option<usize>,
    connected: Option<bool>,
    reason: Option<String>,
    control_verified: bool,
}

#[derive(Serialize)]
pub(super) struct Report {
    configuration: ConfigurationSummary,
    hosts: Vec<HostCheck>,
    telegram: Option<telegram::Identity>,
    discord: Option<discord::Identity>,
    control_enabled: bool,
    note: String,
}

async fn connect(host: &Host, config: &Config) -> Result<usize, Error> {
    for endpoint in engine::connections::discover(host, config).await? {
        let catalog = Catalog::open(&config.service.data_dir).await?;
        catalog.discover(&host.id, endpoint).await?;
    }
    let mut effective = config.runtime.clone();
    Catalog::open(&config.service.data_dir)
        .await?
        .load(&mut effective)
        .await?;
    let provider = effective
        .providers
        .iter()
        .find(|p| p.host_id == host.id)
        .ok_or(Error::SharedEndpoint)?;
    let client = engine::connections::connect(host, provider, &effective).await?;
    let agent = codex::Codex::new(provider.id.clone(), client);
    let page = agent
        .list(SessionQuery {
            cwd: None,
            cursor: None,
            limit: 10,
            archived: false,
        })
        .await?;
    Ok(page.data.len())
}

async fn check_telegram(config: &Config) -> Result<telegram::Identity, Error> {
    let tg = config.telegram.as_ref().ok_or(Error::Selection)?;
    let token = credentials::read(tg)?;
    Ok(
        telegram::Inspector::new(token, Duration::from_secs(30), &tg.proxy)?
            .check()
            .await?,
    )
}

pub(super) async fn inspect(config: &Config, options: &Options) -> Result<Report, Error> {
    let configuration = configuration_summary(config, options.credentials)?;
    let probe = options.connect;
    let mut tasks = tokio::task::JoinSet::new();
    for host in config.hosts.clone() {
        let config = config.clone();
        tasks.spawn(async move {
            let (connected, reason, sample_sessions) = if probe {
                match connect(&host, &config).await {
                    Ok(count) => (Some(true), None, Some(count)),
                    Err(error) => (
                        Some(false),
                        Some(i18n::LocalizedError::localized(&error, &config.language)),
                        None,
                    ),
                }
            } else {
                (None, None, None)
            };
            HostCheck {
                id: host.id.clone(),
                name: host.name.clone(),
                ssh_alias: host.ssh_alias.clone(),
                configured: true,
                sample_sessions,
                connected,
                reason,
                control_verified: config
                    .providers
                    .iter()
                    .any(|p| p.host_id == host.id && p.control_verified),
            }
        });
    }
    let mut hosts = vec![];
    while let Some(result) = tasks.join_next().await {
        hosts.push(result.map_err(|_| Error::Selection)?);
    }
    hosts.sort_by(|a, b| a.id.cmp(&b.id));
    let telegram = if options.telegram {
        Some(check_telegram(config).await?)
    } else {
        None
    };
    let discord = if options.discord {
        #[cfg(feature = "discord")]
        {
            let settings = config.discord.as_ref().ok_or(Error::Selection)?;
            Some(
                tokio::time::timeout(Duration::from_secs(30), discord::Api::inspect(settings))
                    .await
                    .map_err(|_| discord::Error::Request)??,
            )
        }
        #[cfg(not(feature = "discord"))]
        {
            return Err(app::runtime::Error::Feature.into());
        }
    } else {
        None
    };
    Ok(Report {
        configuration,
        hosts,
        telegram,
        discord,
        control_enabled: config.providers.iter().any(|p| p.control_verified),
        note: i18n::text(&config.language, "cli.doctor_note"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn offline_report_sorts_hosts_and_skips_network_and_credential_checks() {
        let config = config::parse(
            std::path::Path::new("/tmp/pocketpilot.toml"),
            "[[hosts]]\nid='z'\nname='Zulu'\ntransport='local_proxy'\n\
             [[hosts]]\nid='a'\nname='Alpha'\ntransport='local_proxy'\n\
             [telegram]\ntoken='invalid-fixture-secret'",
        )
        .unwrap()
        .config;
        let options = Options {
            credentials: false,
            connect: false,
            telegram: false,
            discord: false,
        };

        let report = inspect(&config, &options).await.unwrap();

        assert_eq!(
            report
                .hosts
                .iter()
                .map(|host| host.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        assert!(report.hosts.iter().all(|host| {
            host.connected.is_none() && host.sample_sessions.is_none() && host.reason.is_none()
        }));
        assert!(report.telegram.is_none() && report.discord.is_none());
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("invalid-fixture-secret"));
        assert!(!report.configuration.credentials_checked);
    }

    #[test]
    fn offline_credentials_check_supports_discord_only_and_redacts_tokens() {
        let path = std::path::Path::new("/tmp/pocketpilot.toml");
        let discord = config::parse(
            path,
            "[discord]\ntoken='fixture-discord-secret'\nguild_id=2",
        )
        .unwrap()
        .config;
        let summary = configuration_summary(&discord, true).unwrap();
        let json = serde_json::to_value(summary).unwrap();
        assert_eq!(json["discord_configured"], true);
        assert_eq!(json["telegram_configured"], false);
        assert_eq!(json["credentials_checked"], true);
        assert!(!json.to_string().contains("fixture-discord-secret"));
        let telegram = config::parse(path, "[telegram]\ntoken='invalid-fixture-secret'")
            .unwrap()
            .config;
        assert!(configuration_summary(&telegram, false).is_ok());
        assert!(matches!(
            configuration_summary(&telegram, true),
            Err(Error::Config(config::ConfigError::Credentials))
        ));
    }
}
