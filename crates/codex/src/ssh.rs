//! Resolve configured aliases with OpenSSH rather than reimplementing Match semantics.
use crate::Error;
use serde::Serialize;
use ssh2_config::{ParseRule, SshConfig};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

pub fn discover(path: &Path) -> Result<Vec<String>, Error> {
    let file = std::fs::File::open(path).map_err(|_| Error::Endpoint)?;
    let config = SshConfig::default()
        .parse(&mut BufReader::new(file), ParseRule::ALLOW_UNKNOWN_FIELDS)
        .map_err(|_| Error::Protocol)?;
    let mut aliases = std::collections::BTreeSet::new();
    for host in config.get_hosts() {
        for clause in &host.pattern {
            if !clause.negated && valid_alias(&clause.pattern) {
                aliases.insert(clause.pattern.clone());
            }
        }
    }
    Ok(aliases.into_iter().collect())
}

pub fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias.starts_with('-')
        && alias
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolvedHost {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity_files: Vec<String>,
    pub keyboard_interactive_allowed: bool,
    pub has_proxy: bool,
}

pub async fn resolve(
    alias: &str,
    config: Option<&PathBuf>,
    timeout: Duration,
) -> Result<ResolvedHost, Error> {
    if !valid_alias(alias) {
        return Err(Error::Endpoint);
    }
    let mut command = Command::new("ssh");
    command.args(["-G", "-T"]);
    if let Some(path) = config {
        command.arg("-F").arg(path);
    }
    let mut child = command
        .arg(alias)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error::Disconnected)?;
    let mut stdout = child.stdout.take().ok_or(Error::Disconnected)?;
    let work = async {
        let mut bytes = vec![];
        (&mut stdout)
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error::Disconnected)?;
        if bytes.len() > 1024 * 1024 {
            return Err(Error::Protocol);
        }
        let status = child.wait().await.map_err(|_| Error::Disconnected)?;
        if !status.success() {
            return Err(Error::Endpoint);
        }
        let output = String::from_utf8(bytes).map_err(|_| Error::Protocol)?;
        parse_resolved(alias, &output)
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| Error::Timeout)?
}

fn parse_resolved(alias: &str, input: &str) -> Result<ResolvedHost, Error> {
    let mut result = ResolvedHost {
        alias: alias.into(),
        hostname: String::new(),
        user: String::new(),
        port: 22,
        identity_files: vec![],
        keyboard_interactive_allowed: false,
        has_proxy: false,
    };
    for line in input.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "hostname" => result.hostname = value.into(),
            "user" => result.user = value.into(),
            "port" => result.port = value.parse().map_err(|_| Error::Protocol)?,
            "identityfile" => result.identity_files.push(value.into()),
            "kbdinteractiveauthentication" => result.keyboard_interactive_allowed = value == "yes",
            "proxyjump" | "proxycommand" => result.has_proxy |= value != "none",
            _ => {}
        }
    }
    if result.hostname.is_empty() || result.user.is_empty() || result.port == 0 {
        return Err(Error::Protocol);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_does_not_expand_wildcards_or_negated_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(
            &path,
            "Host alpha beta !blocked *.example\n HostName example.invalid\nHost *\n User user\n",
        )
        .unwrap();
        assert_eq!(discover(&path).unwrap(), vec!["alpha", "beta"]);
    }
    #[test]
    fn keyboard_interactive_is_permission_not_an_otp_requirement() {
        let host = parse_resolved(
            "host",
            "hostname example.invalid\nuser user\nport 22\nkbdinteractiveauthentication yes\n",
        )
        .unwrap();
        assert!(host.keyboard_interactive_allowed);
        assert!(!host.has_proxy);
    }
}
