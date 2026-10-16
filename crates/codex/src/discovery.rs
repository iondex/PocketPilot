//! Discover existing Codex installations and control sockets without starting a server.
use serde::{Deserialize, Serialize};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub binary: String,
    pub socket: String,
}

// Fixed read-only shell program: no configured/user text is interpolated into it.
// Prefer a native executable: non-interactive shells may not load Node/NVM.
const SCAN: &str = r#"
root=${CODEX_HOME:-"$HOME/.codex"}
for binary in "$root"/packages/app-server-daemon/releases/*/bin/codex "$HOME"/.nvm/versions/node/*/lib/node_modules/@openai/codex/node_modules/@openai/codex-*/vendor/*/bin/codex "$HOME/.local/bin/codex" /opt/homebrew/bin/codex /usr/local/bin/codex; do
    if [ -x "$binary" ]; then printf 'B\t%s\n' "$binary"; fi
done
binary=$(command -v codex 2>/dev/null) || :
if [ -n "$binary" ]; then printf 'B\t%s\n' "$binary"; fi
socket="$root/app-server-control/app-server-control.sock"
if [ -S "$socket" ]; then printf 'S\t%s\n' "$socket"; fi
if [ -d "$root" ]; then find -H "$root" -maxdepth 4 -type s -name '*control*.sock' -print 2>/dev/null | while IFS= read -r socket; do printf 'S\t%s\n' "$socket"; done; fi
"#;

pub async fn discover(
    options: Option<ssh::Options>,
    timeout: Duration,
) -> Result<Vec<Endpoint>, crate::Error> {
    let mut command = match options {
        Some(options) => ssh::command(options, format!("sh -c '{}'", SCAN.replace('\'', "'\\''")))
            .map_err(|_| crate::Error::Endpoint)?,
        None => {
            let mut command = Command::new("sh");
            command.args(["-c", SCAN]);
            command
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    tracing::debug!("discovering existing Codex control endpoints");
    let mut child = command.spawn().map_err(|_| crate::Error::Disconnected)?;
    let stdout = child.stdout.take().ok_or(crate::Error::Disconnected)?;
    let scan = async {
        let mut bytes = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| crate::Error::Disconnected)?;
        if bytes.len() > 1024 * 1024 {
            return Err(crate::Error::Protocol);
        }
        if !child
            .wait()
            .await
            .map_err(|_| crate::Error::Disconnected)?
            .success()
        {
            return Err(crate::Error::Endpoint);
        }
        parse(std::str::from_utf8(&bytes).map_err(|_| crate::Error::Protocol)?)
    };
    tokio::time::timeout(timeout, scan)
        .await
        .map_err(|_| crate::Error::Timeout)?
}

fn parse(output: &str) -> Result<Vec<Endpoint>, crate::Error> {
    let mut binaries = Vec::new();
    let mut sockets = std::collections::BTreeSet::new();
    for line in output.lines() {
        let (kind, path) = line.split_once('\t').ok_or(crate::Error::Protocol)?;
        if !path.starts_with('/') || path.chars().any(char::is_control) {
            return Err(crate::Error::Protocol);
        }
        match kind {
            "B" => binaries.push(path.to_owned()),
            "S" => {
                sockets.insert(path.to_owned());
            }
            _ => return Err(crate::Error::Protocol),
        }
    }
    let Some(binary) = binaries.first() else {
        return Ok(vec![]);
    };
    Ok(sockets
        .into_iter()
        .map(|socket| Endpoint {
            binary: binary.clone(),
            socket,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_only_existing_sockets_and_rejects_shell_noise() {
        assert!(parse("B\t/usr/bin/codex\n").unwrap().is_empty());
        let found = parse("B\t/native/codex\nB\t/wrapper/codex\nS\t/home/.codex/control.sock\nS\t/home/.codex/control.sock\n").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].binary, "/native/codex");
        assert!(parse("welcome to server\n").is_err());
        assert!(parse("B\trelative\n").is_err());
    }
}
