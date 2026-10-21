//! Configuration diagnostics and explicit read-only connection probes.
use crate::cli::Error;
use app::config::{self, Config, Host};
use engine::catalog::Catalog;
use integrations::telegram::credentials;
use serde::Serialize;
use session::SessionQuery;
use std::time::Duration;

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
    slack_configured: bool,
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
        slack_configured: config.slack.is_some(),
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
    telegram: Option<integrations::telegram::Identity>,
    discord: Option<integrations::discord::Identity>,
    control_enabled: bool,
    note: String,
}

async fn connect(host: &Host, config: &Config) -> Result<usize, Error> {
    for endpoint in engine::runtimes::discover(host, config).await? {
        let catalog = Catalog::open(&config.service.data_dir).await?;
        catalog
            .discover(&host.id, adapters::RuntimeKind::Codex, endpoint)
            .await?;
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
    let agent = engine::runtimes::connect_adapter(host, provider, &effective)
        .await
        .map_err(|_| Error::SharedEndpoint)?;
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

async fn check_telegram(config: &Config) -> Result<integrations::telegram::Identity, Error> {
    let tg = config.telegram.as_ref().ok_or(Error::Selection)?;
    let token = credentials::read(tg)?;
    Ok(
        integrations::telegram::Inspector::new(token, Duration::from_secs(30), &tg.proxy)?
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
        {
            let settings = config.discord.as_ref().ok_or(Error::Selection)?;
            Some(
                tokio::time::timeout(
                    Duration::from_secs(30),
                    integrations::discord::Api::inspect(settings),
                )
                .await
                .map_err(|_| integrations::discord::Error::Request)??,
            )
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
#[path = "doctor_tests.rs"]
mod tests;
