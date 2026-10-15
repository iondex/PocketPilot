use app::{
    config::{self, Config, Host, TransportKind},
    credentials,
    storage::Store,
};
use clap::{Parser, Subcommand};
use codex::{Client, Codex};
use secrecy::ExposeSecret;
use serde::Serialize;
use session::{AgentProvider, SessionQuery};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
#[command(
    name = "pocketpilot",
    version,
    about = "Telegram bridge for shared Codex sessions"
)]
struct Arguments {
    #[arg(long, global = true, default_value = "pocketpilot.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a deployment; --write uses a dedicated NEW probe session with the configured model.
    Probe {
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        write: bool,
        #[arg(long, conflicts_with = "write")]
        thread: Option<String>,
    },
    #[command(hide = true)]
    SshAskpass {
        #[arg(long)]
        host: String,
        #[arg(long)]
        attempt: String,
        #[arg(long)]
        config_hash: String,
        #[arg(allow_hyphen_values = true)]
        prompt: String,
    },
    /// Run the private-chat Telegram bridge until Ctrl-C or SIGTERM.
    Serve,
    /// Validate configuration without making network connections.
    ConfigCheck {
        /// Also validate the inline token format (never prints its value).
        #[arg(long)]
        credentials: bool,
    },
    /// Diagnose capabilities. Network probes require explicit flags.
    Doctor {
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
    /// Read a configured workspace's sessions. Never resumes a thread.
    Sessions {
        #[arg(long)]
        workspace: String,
        #[command(subcommand)]
        command: Sessions,
    },
    /// Read-only Telegram account/webhook verification. Sends no messages.
    TelegramCheck,
    /// Initialize PocketPilot's own SQLite ledger. Does not access Codex storage.
    StateInit,
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
enum Sessions {
    List {
        #[arg(long, default_value_t = 25)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        archived: bool,
    },
    Show {
        thread: String,
    },
    History {
        thread: String,
        #[arg(long)]
        latest: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Probe(#[from] app::probe::Error),
    #[error(transparent)]
    Otp(#[from] app::otp::Error),
    #[error(transparent)]
    Service(#[from] app::service::Error),
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Rpc(#[from] codex::Error),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error(transparent)]
    Telegram(#[from] telegram::Error),
    #[error(transparent)]
    Store(#[from] app::storage::StoreError),
    #[error(transparent)]
    Pairing(#[from] app::pairing::Error),
    #[error("unknown or unsupported configured host/workspace")]
    Selection,
    #[error("output could not be written (existing files are not overwritten)")]
    Output,
    #[error("logging configuration could not be initialized")]
    Logging,
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
    credentials_checked: bool,
    control_enabled: bool,
}

pub async fn run() -> Result<(), Error> {
    let args = Arguments::parse();
    let loaded = config::load(&args.config)?;
    let config = loaded.config;
    if let Command::SshAskpass {
        host,
        attempt,
        config_hash,
        prompt,
    } = &args.command
    {
        let answer = app::otp::answer(&config, host, attempt, config_hash, prompt).await?;
        writeln!(std::io::stdout().lock(), "{}", answer.expose_secret())
            .map_err(|_| Error::Output)?;
        return Ok(());
    }
    if let Command::ConfigCheck { credentials: check } = args.command {
        if check {
            let _ = credentials::read(config.telegram.as_ref().ok_or(Error::Selection)?)?;
        }
        return print(&ConfigurationSummary {
            hosts: config.hosts.len(),
            providers: config.providers.len(),
            workspaces: config.workspaces.len(),
            telegram_configured: config.telegram.is_some(),
            credentials_checked: check,
            control_enabled: config.providers.iter().any(|p| p.control_verified),
        });
    }
    let _logs = crate::telemetry::init(&config.logging).map_err(|_| Error::Logging)?;
    match args.command {
        Command::Probe {
            workspace,
            write,
            thread,
        } => print(&app::probe::run(&config, &workspace, write, thread.as_deref()).await?)?,
        Command::SshAskpass { .. } => unreachable!(),
        Command::Serve => app::service::run(args.config).await?,
        Command::ConfigCheck { .. } => unreachable!(),
        Command::Doctor { connect, telegram } => doctor(&config, connect, telegram).await?,
        Command::TelegramCheck => print(&check_telegram(&config).await?)?,
        Command::Pair { command } => {
            let store = app::pairing::Registry::open(&config.service.data_dir).await?;
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
                    println!("Pairing revoked; remote sessions remain unchanged.");
                }
            }
        }
        Command::StateInit => {
            let _ = Store::open(&config.service.data_dir).await?;
            println!("Local state initialized; no agent operation was sent.");
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
                print(&codex::ssh::discover(&path)?)?;
            }
            Hosts::Resolve { id } => {
                let host = config
                    .hosts
                    .iter()
                    .find(|h| h.id == id)
                    .ok_or(Error::Selection)?;
                let alias = host.ssh_alias.as_deref().ok_or(Error::Selection)?;
                print(
                    &codex::ssh::resolve(
                        alias,
                        host.ssh_config.as_ref(),
                        Duration::from_secs(host.timeouts.connect_seconds),
                    )
                    .await?,
                )?;
            }
        },
        Command::Sessions { workspace, command } => {
            let workspace = config
                .workspaces
                .iter()
                .find(|w| w.id == workspace)
                .ok_or(Error::Selection)?;
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
            let client = connect(host, &config).await?;
            let provider = Codex::new(provider.id.clone(), client);
            match command {
                Sessions::List {
                    limit,
                    cursor,
                    archived,
                } => {
                    print(
                        &provider
                            .list(SessionQuery {
                                cwd: Some(workspace.cwd.clone()),
                                cursor,
                                limit,
                                archived,
                            })
                            .await?,
                    )?;
                }
                Sessions::Show { thread } => {
                    let session = provider.read(&thread).await?;
                    require_workspace(&session.cwd, workspace)?;
                    print(&session)?;
                }
                Sessions::History {
                    thread,
                    latest,
                    output,
                } => {
                    let summary = provider.read(&thread).await?;
                    require_workspace(&summary.cwd, workspace)?;
                    let history = provider.history(&thread).await?;
                    let text = if latest {
                        session::delivery::latest_interaction(&history)
                    } else {
                        session::delivery::full_history(&history)
                    };
                    write_output(output.as_deref(), &text)?;
                }
            }
        }
    }
    Ok(())
}

fn require_workspace(cwd: &str, workspace: &config::Workspace) -> Result<(), Error> {
    if cwd != workspace.cwd && !workspace.additional_cwds.iter().any(|p| p == cwd) {
        return Err(Error::Selection);
    }
    Ok(())
}

fn transport_name(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::LocalProxy => "local_proxy",
        TransportKind::SshProxy => "ssh_proxy",
        TransportKind::Websocket => "websocket",
    }
}

async fn connect(host: &Host, config: &Config) -> Result<Client, Error> {
    Ok(app::connections::connect(host, config).await?)
}

async fn check_telegram(config: &Config) -> Result<telegram::Identity, Error> {
    let tg = config.telegram.as_ref().ok_or(Error::Selection)?;
    let token = credentials::read(tg)?;
    Ok(telegram::Inspector::new(token, Duration::from_secs(30))
        .check()
        .await?)
}

async fn doctor(config: &Config, probe: bool, telegram: bool) -> Result<(), Error> {
    #[derive(Serialize)]
    struct HostCheck {
        id: String,
        configured: bool,
        connected: Option<bool>,
        reason: Option<String>,
        control_verified: bool,
    }
    #[derive(Serialize)]
    struct Report {
        hosts: Vec<HostCheck>,
        telegram: Option<telegram::Identity>,
        control_enabled: bool,
        note: &'static str,
    }
    let mut tasks = tokio::task::JoinSet::new();
    for host in config.hosts.clone() {
        let config = config.clone();
        tasks.spawn(async move {
            let (connected, reason) = if probe {
                match connect(&host, &config).await {
                    Ok(_) => (Some(true), None),
                    Err(error) => (Some(false), Some(error.to_string())),
                }
            } else {
                (None, None)
            };
            HostCheck {
                id: host.id.clone(),
                configured: true,
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
        hosts,
        telegram,
        control_enabled: config.providers.iter().any(|p| p.control_verified),
        note: "Read-only probe. Control flags are operator attestations for each configured shared runtime.",
    })
}

fn write_output(path: Option<&Path>, text: &str) -> Result<(), Error> {
    if let Some(path) = path {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| Error::Output)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| Error::Output)?;
        }
        file.write_all(text.as_bytes()).map_err(|_| Error::Output)?;
    } else {
        writeln!(std::io::stdout().lock(), "{text}").map_err(|_| Error::Output)?;
    }
    Ok(())
}
