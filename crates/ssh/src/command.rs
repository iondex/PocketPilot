use crate::Error;
use std::{path::PathBuf, time::Duration};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub struct Options {
    pub alias: String,
    pub config: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub keepalive_seconds: u64,
    pub keepalive_missed: u32,
    pub askpass: Option<PathBuf>,
    pub max_auth_attempts: u32,
    pub authentication_methods: Vec<String>,
}

pub fn command(options: Options, remote: String) -> Result<Command, Error> {
    if !crate::resolve::valid_alias(&options.alias) {
        return Err(Error::Endpoint);
    }
    let mut command = Command::new("ssh");
    command
        .args([
            "-T",
            "-o",
            if options.askpass.is_some() {
                "BatchMode=no"
            } else {
                "BatchMode=yes"
            },
            "-o",
            "StrictHostKeyChecking=yes",
        ])
        .arg("-o")
        .arg(format!(
            "ConnectTimeout={}",
            options.connect_timeout.as_secs().max(1)
        ))
        .arg("-o")
        .arg(format!("ServerAliveInterval={}", options.keepalive_seconds))
        .arg("-o")
        .arg(format!("ServerAliveCountMax={}", options.keepalive_missed));
    if let Some(helper) = options.askpass {
        command
            .env("SSH_ASKPASS", helper)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .arg("-o")
            .arg(format!(
                "NumberOfPasswordPrompts={}",
                options.max_auth_attempts
            ));
    }
    if !options.authentication_methods.is_empty() {
        command.arg("-o").arg(format!(
            "PreferredAuthentications={}",
            options.authentication_methods.join(",")
        ));
    }
    if let Some(config) = options.config {
        command.arg("-F").arg(config);
    }
    command.arg(options.alias).arg(remote);
    Ok(command)
}
