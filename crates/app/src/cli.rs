use app::config::{self, Config, Host, TransportKind};
use clap::{Parser, Subcommand};
use engine::catalog::Catalog;
use secrecy::ExposeSecret;
use serde::Serialize;
use session::{AgentProvider, SessionQuery};
use std::{io::Write, path::PathBuf, time::Duration};
use telegram::credentials;

#[derive(Parser)]
#[command(
    name = "pocketpilot",
    version,
    about = "Chat bridge for shared Codex sessions"
)]
struct Arguments {
    #[arg(long, global = true, default_value = "pocketpilot.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "ssh-askpass", hide = true)]
    SSHAskpass {
        #[arg(long)]
        host: String,
        #[arg(long)]
        attempt: String,
        #[arg(long)]
        config_hash: String,
        #[arg(allow_hyphen_values = true)]
        prompt: String,
    },
    /// Run configured integrations until Ctrl-C or SIGTERM.
    Serve,
    /// Validate configuration and diagnose capabilities. Network probes require explicit flags.
    Doctor {
        /// Also check configured credentials offline (never prints their values).
        #[arg(long)]
        credentials: bool,
        #[arg(long)]
        connect: bool,
        #[arg(long)]
        telegram: bool,
    },
    /// List configured hosts or discover candidate SSH aliases without logging in.
    Hosts {
        #[command(subcommand)]
        command: Hosts,
    },
    /// Manage persisted project entries without restarting the bridge.
    Workspaces {
        #[command(subcommand)]
        command: Workspaces,
    },
    /// Inspect agent instances and explicitly attest their shared lifecycle.
    Providers {
        #[command(subcommand)]
        command: Providers,
    },
    /// Manage private-chat pairing. Approval is available only from this CLI.
    Pair {
        #[command(subcommand)]
        command: Pair,
    },
}

#[derive(Subcommand)]
enum Pair {
    Pending,
    Approve { code: String },
    List,
    Revoke { bot_id: i64, user_id: u64 },
}

#[derive(Subcommand)]
enum Hosts {
    List,
    Discover {
        #[arg(long)]
        ssh_config: Option<PathBuf>,
    },
    Resolve {
        id: String,
    },
}

#[derive(Subcommand)]
enum Workspaces {
    List,
    Add {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        name: Option<String>,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
enum Providers {
    List,
    /// Declare that this exact shared execution instance has passed lifecycle verification.
    Trust {
        id: String,
        #[arg(long)]
        revoke: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    #[allow(clippy::upper_case_acronyms)] // Project convention keeps SSH uppercase.
    SSH(#[from] ssh::Error),
    #[error(transparent)]
    #[allow(clippy::upper_case_acronyms)] // Project convention keeps TOTP uppercase.
    TOTP(#[from] ssh::auth::Error),
    #[error(transparent)]
    Service(#[from] app::runtime::Error),
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Rpc(#[from] codex::Error),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error(transparent)]
    Telegram(#[from] telegram::Error),
    #[error(transparent)]
    Store(#[from] engine::storage::StoreError),
    #[error(transparent)]
    Pairing(#[from] telegram::pairing::Error),
    #[error("no existing Codex control endpoint was discovered for this host")]
    SharedEndpoint,
    #[error("unknown or unsupported configured host/workspace")]
    Selection,
    #[error("output could not be written (existing files are not overwritten)")]
    Output,
    #[error("logging configuration could not be initialized")]
    Logging,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::SharedEndpoint => ::i18n::Message::new("error.cli.shared_endpoint"),
            Self::Selection => ::i18n::Message::new("error.cli.selection"),
            Self::Output => ::i18n::Message::new("error.cli.output"),
            Self::Logging => ::i18n::Message::new("error.cli.logging"),
            Self::SSH(error) => ::i18n::LocalizedError::message(error),
            Self::TOTP(error) => ::i18n::LocalizedError::message(error),
            Self::Service(error) => ::i18n::LocalizedError::message(error),
            Self::Config(error) => ::i18n::LocalizedError::message(error),
            Self::Rpc(error) => ::i18n::LocalizedError::message(error),
            Self::Provider(error) => ::i18n::LocalizedError::message(error),
            Self::Telegram(error) => ::i18n::LocalizedError::message(error),
            Self::Store(error) => ::i18n::LocalizedError::message(error),
            Self::Pairing(error) => ::i18n::LocalizedError::message(error),
        }
    }
}

fn print(value: &impl Serialize) -> Result<(), Error> {
    let output = serde_json::to_string_pretty(value).map_err(|_| Error::Output)?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|_| Error::Output)
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

#[derive(Debug)]
pub struct Failure {
    error: Error,
    locale: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&i18n::LocalizedError::localized(&self.error, &self.locale))
    }
}

pub async fn run() -> Result<(), Failure> {
    let args = Arguments::parse();
    let loaded = config::load(&args.config).map_err(|error| Failure {
        error: Error::Config(error),
        locale: "zh-CN".into(),
    })?;
    let locale = loaded.config.language.clone();
    run_configured(args, loaded.config, &locale)
        .await
        .map_err(|error| Failure { error, locale })
}

async fn run_configured(args: Arguments, mut config: Config, locale: &str) -> Result<(), Error> {
    if let Command::SSHAskpass {
        host,
        attempt,
        config_hash,
        prompt,
    } = &args.command
    {
        let selected = config
            .hosts
            .iter()
            .find(|h| h.id == *host)
            .ok_or(Error::Selection)?;
        let context = ssh::auth::AuthContext {
            data_dir: config.service.data_dir.clone(),
            source: config.source().to_owned(),
            source_hash: config.source_hash().into(),
        };
        let answer = ssh::auth::answer(
            &selected.authentication,
            &context,
            attempt,
            config_hash,
            prompt,
        )
        .await?;
        writeln!(std::io::stdout().lock(), "{}", answer.expose_secret())
            .map_err(|_| Error::Output)?;
        return Ok(());
    }
    let catalog = Catalog::open(&config.service.data_dir).await?;
    catalog.load(&mut config.runtime).await?;
    let _logs = crate::telemetry::init(&config.logging).map_err(|_| Error::Logging)?;
    match args.command {
        Command::SSHAskpass { .. } => unreachable!(),
        Command::Serve => app::runtime::run(args.config).await?,
        Command::Doctor {
            credentials,
            connect,
            telegram,
        } => doctor(&config, credentials, connect, telegram).await?,
        Command::Pair { command } => {
            let store = telegram::pairing::Registry::open(&config.service.data_dir).await?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::Selection)?
                .as_secs() as i64;
            match command {
                Pair::Pending => print(&store.pending_pairings(now).await?)?,
                Pair::Approve { code } => print(&store.approve_pairing(&code, now).await?)?,
                Pair::List => print(&store.paired_users().await?)?,
                Pair::Revoke { bot_id, user_id } => {
                    store.revoke_pairing(bot_id, user_id).await?;
                    println!("{}", i18n::text(locale, "cli.pair_revoked"));
                }
            }
        }
        Command::Hosts { command } => match command {
            Hosts::List => {
                #[derive(Serialize)]
                struct Row<'a> {
                    id: &'a str,
                    name: &'a str,
                    transport: &'static str,
                    otp_configured: bool,
                }
                let rows: Vec<_> = config
                    .hosts
                    .iter()
                    .map(|h| Row {
                        id: &h.id,
                        name: &h.name,
                        transport: transport_name(h.transport),
                        otp_configured: h.authentication.totp.is_some(),
                    })
                    .collect();
                print(&rows)?;
            }
            Hosts::Discover { ssh_config } => {
                let path = match ssh_config {
                    Some(path) => path,
                    None => PathBuf::from(std::env::var_os("HOME").ok_or(Error::Selection)?)
                        .join(".ssh/config"),
                };
                print(&ssh::discover(&path)?)?;
            }
            Hosts::Resolve { id } => {
                let host = config
                    .hosts
                    .iter()
                    .find(|h| h.id == id)
                    .ok_or(Error::Selection)?;
                let alias = host.ssh_alias.as_deref().ok_or(Error::Selection)?;
                print(
                    &ssh::resolve(
                        alias,
                        host.ssh_config.as_ref(),
                        Duration::from_secs(host.timeouts.connect_seconds),
                    )
                    .await?,
                )?;
            }
        },
        Command::Workspaces { command } => match command {
            Workspaces::List => print(&config.workspaces)?,
            Workspaces::Add {
                provider,
                cwd,
                name,
            } => print(
                &catalog
                    .add_workspace(&config, &provider, &cwd, name.as_deref())
                    .await?,
            )?,
            Workspaces::Remove { id } => {
                let selected = config
                    .workspaces
                    .iter()
                    .find(|w| w.id == id || w.short_name == id)
                    .ok_or(Error::Selection)?;
                catalog.remove_workspace(&selected.id).await?;
                println!("{}", i18n::text(locale, "cli.workspace_removed"));
            }
        },
        Command::Providers { command } => match command {
            Providers::List => print(&config.providers)?,
            Providers::Trust { id, revoke } => {
                let mut provider = config
                    .providers
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or(Error::Selection)?;
                provider.control_verified = !revoke;
                catalog.save_provider(&provider).await?;
                println!("{}", i18n::text(locale, "cli.provider_updated"));
            }
        },
    }
    Ok(())
}

fn transport_name(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::LocalProxy => "local_proxy",
        TransportKind::SSHProxy => "ssh_proxy",
        TransportKind::Websocket => "websocket",
    }
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
    Ok(telegram::Inspector::new(token, Duration::from_secs(30))
        .check()
        .await?)
}

async fn doctor(config: &Config, check: bool, probe: bool, telegram: bool) -> Result<(), Error> {
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
    struct Report {
        configuration: ConfigurationSummary,
        hosts: Vec<HostCheck>,
        telegram: Option<telegram::Identity>,
        control_enabled: bool,
        note: String,
    }
    let configuration = configuration_summary(config, check)?;
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
    let telegram = if telegram {
        Some(check_telegram(config).await?)
    } else {
        None
    };
    print(&Report {
        configuration,
        hosts,
        telegram,
        control_enabled: config.providers.iter().any(|p| p.control_verified),
        note: i18n::text(&config.language, "cli.doctor_note"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_combines_offline_checks_with_explicit_network_flags() {
        let args = Arguments::try_parse_from([
            "pocketpilot",
            "doctor",
            "--credentials",
            "--connect",
            "--telegram",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            Command::Doctor {
                credentials: true,
                connect: true,
                telegram: true,
            }
        ));
        let args = Arguments::try_parse_from(["pocketpilot", "doctor"]).unwrap();
        assert!(matches!(
            args.command,
            Command::Doctor {
                credentials: false,
                connect: false,
                telegram: false,
            }
        ));
        assert!(Arguments::try_parse_from(["pocketpilot", "config-check"]).is_err());
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

    #[test]
    fn cli_errors_keep_their_type_and_render_the_selected_language() {
        let english = Failure {
            error: Error::Provider(session::ProviderError::Rejected(-32600)),
            locale: "en".into(),
        };
        let chinese = Failure {
            error: Error::Provider(session::ProviderError::Rejected(-32600)),
            locale: "zh-CN".into(),
        };
        assert!(english.to_string().contains("provider rejected"));
        assert!(chinese.to_string().contains("执行服务拒绝"));
        assert!(english.to_string().contains("-32600") && chinese.to_string().contains("-32600"));
    }
}
