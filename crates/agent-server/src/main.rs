//! A durable agent owner. Proxy/SSH clients may disconnect without owning its child processes.
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod server;

mod storage;

mod claude;

#[derive(Parser)]
#[command(version, about = "PocketPilot independent agent runtime server")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long, default_value = "claude")]
        agent_bin: PathBuf,
        #[arg(long, required = true)]
        allow_root: Vec<PathBuf>,
    },
    Proxy {
        #[arg(long)]
        socket: PathBuf,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let result = match Arguments::parse().command {
        Command::Serve {
            socket,
            state_dir,
            agent_bin,
            allow_root,
        } => server::serve(socket, state_dir, agent_bin, allow_root).await,
        Command::Proxy { socket } => server::proxy(&socket).await,
    };
    if result.is_err() {
        // A fixed diagnostic only: agent frames, prompts and file contents are private.
        eprintln!("PocketPilot runtime server unavailable");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
