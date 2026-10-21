//! Explicit installation using the same authenticated local/SSH command path as runtime clients.
use crate::config::{Config, Host, RuntimeServer, TransportKind};
use connections::{Connection, Program};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// All configured values cross this shell as positional arguments, never interpolation.
// An existing socket is preserved along with its recorded binary. An installation
// does not replace a live server or restart any native agent process.
const INSTALL: &str = r#"
set -eu
umask 077
pp_directory=$1
pp_digest=$2
pp_agent=$3
pp_arch=$4
shift 4
case "$pp_directory" in '~/.pocketpilot') pp_directory="$HOME/.pocketpilot";; /*) ;; *) exit 2;; esac
[ "$(uname -s)" = Linux ]
[ "$(uname -m)" = "$pp_arch" ]
[ ! -L "$pp_directory" ]
mkdir -p "$pp_directory"
chmod 700 "$pp_directory"
exec 9>"$pp_directory/install.lock"
flock -n 9
pp_state="$pp_directory/claude-code"
[ ! -L "$pp_state" ]
mkdir -p "$pp_state"
chmod 700 "$pp_state"
if [ -S "$pp_state/server.sock" ]; then
    cat > /dev/null
    cat "$pp_state/endpoint.json"
    exit 0
fi
[ ! -L "$pp_directory/bin" ]
mkdir -p "$pp_directory/bin"
pp_binary="$pp_directory/bin/agent-server-$pp_digest"
pp_temporary=$(mktemp "$pp_directory/bin/install.XXXXXX")
trap 'rm -f "$pp_temporary"' EXIT HUP INT TERM
cat > "$pp_temporary"
printf '%s  %s\n' "$pp_digest" "$pp_temporary" | sha256sum -c - > /dev/null
chmod 700 "$pp_temporary"
"$pp_temporary" --version > /dev/null
mv -f "$pp_temporary" "$pp_binary"
command -v "$pp_agent" > /dev/null
nohup "$pp_binary" serve --socket "$pp_state/server.sock" --state-dir "$pp_state" --agent-bin "$pp_agent" "$@" < /dev/null > /dev/null 2>&1 9>&- &
pp_attempt=0
while [ ! -S "$pp_state/server.sock" ]; do
    pp_attempt=$((pp_attempt+1))
    [ "$pp_attempt" -lt 50 ]
    sleep 0.1
done
# The paths are emitted as separate lines; Rust serializes the endpoint as JSON.
printf '%s\n%s\n' "$pp_binary" "$pp_state/server.sock"
"#;

pub async fn install(
    config: &Config,
    host: &Host,
    runtime: &RuntimeServer,
) -> Result<adapters::ServerEndpoint, session::ProviderError> {
    use session::ProviderError;
    if runtime.kind != adapters::RuntimeKind::ClaudeCode || host.allowed_workspace_roots.is_empty()
    {
        return Err(ProviderError::Unsupported);
    }
    let file = tokio::fs::File::open(&runtime.artifact)
        .await
        .map_err(|_| ProviderError::Unsupported)?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| ProviderError::Unsupported)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(ProviderError::Incompatible);
    }
    let mut artifact = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut artifact)
        .await
        .map_err(|_| ProviderError::Unsupported)?;
    let arch = artifact_arch(&artifact)?;
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&artifact));
    let transport = match host.transport {
        TransportKind::LocalProxy => connections::Host::Local,
        TransportKind::SSHProxy => connections::Host::SSH(
            crate::runtimes::ssh_options(host, config).map_err(|_| ProviderError::Disconnected)?,
        ),
        TransportKind::Websocket => return Err(ProviderError::Unsupported),
    };
    let mut arguments = vec![
        "-c".into(),
        INSTALL.into(),
        "pocketpilot-install".into(),
        runtime.directory.clone(),
        digest,
        runtime.agent_binary.clone(),
        arch.into(),
    ];
    for root in &host.allowed_workspace_roots {
        arguments.extend(["--allow-root".into(), root.clone()]);
    }
    let mut command = transport
        .command(&Program {
            binary: "sh".into(),
            arguments,
        })
        .map_err(|_| ProviderError::Disconnected)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ProviderError::Disconnected)?;
    let input = child.stdin.take().ok_or(ProviderError::Disconnected)?;
    let output = child.stdout.take().ok_or(ProviderError::Disconnected)?;
    let transfer = async {
        let write = send_artifact(input, &artifact);
        let read = async {
            let mut bytes = Vec::new();
            output
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            if bytes.len() > 64 * 1024 {
                return Err(ProviderError::Incompatible);
            }
            Ok(bytes)
        };
        let (_, bytes) = tokio::try_join!(write, read)?;
        if !child
            .wait()
            .await
            .map_err(|_| ProviderError::Disconnected)?
            .success()
        {
            return Err(ProviderError::Disconnected);
        }
        let body = String::from_utf8(bytes).map_err(|_| ProviderError::Incompatible)?;
        // An existing manifest is returned without touching its live process.
        let endpoint = if body.starts_with('{') {
            serde_json::from_str(&body).map_err(|_| ProviderError::Incompatible)?
        } else {
            let parts = body.lines().collect::<Vec<_>>();
            if parts.len() != 2 {
                return Err(ProviderError::Incompatible);
            }
            adapters::ServerEndpoint {
                binary: parts[0].into(),
                socket: parts[1].into(),
            }
        };
        if !std::path::Path::new(&endpoint.binary).is_absolute()
            || !std::path::Path::new(&endpoint.socket).is_absolute()
            || endpoint.binary.contains(['\r', '\n', '\0'])
            || endpoint.socket.contains(['\r', '\n', '\0'])
        {
            return Err(ProviderError::Incompatible);
        }
        Ok(endpoint)
    };
    tokio::time::timeout(
        Duration::from_secs(
            host.timeouts
                .connect_seconds
                .saturating_add(host.timeouts.authenticate_seconds)
                .saturating_add(120),
        ),
        transfer,
    )
    .await
    .map_err(|_| ProviderError::Timeout)?
}

/// Own stdin until transfer completes; returning closes it and delivers remote EOF.
async fn send_artifact(
    mut input: tokio::process::ChildStdin,
    artifact: &[u8],
) -> Result<(), session::ProviderError> {
    input
        .write_all(artifact)
        .await
        .map_err(|_| session::ProviderError::Disconnected)?;
    // ChildStdin::shutdown does not close the descriptor on every platform.
    drop(input);
    Ok(())
}

/// Reject malformed files and dynamic interpreters before transferring an artifact.
fn artifact_arch(bytes: &[u8]) -> Result<&'static str, session::ProviderError> {
    use session::ProviderError;
    if bytes.len() < 64
        || bytes.len() > 64 * 1024 * 1024
        || &bytes[..4] != b"\x7fELF"
        || bytes[4] != 2
        || bytes[5] != 1
    {
        return Err(ProviderError::Incompatible);
    }
    let arch = match u16::from_le_bytes([bytes[18], bytes[19]]) {
        62 => "x86_64",
        183 => "aarch64",
        _ => return Err(ProviderError::Incompatible),
    };
    let offset = usize::try_from(u64::from_le_bytes(
        bytes[32..40]
            .try_into()
            .map_err(|_| ProviderError::Incompatible)?,
    ))
    .map_err(|_| ProviderError::Incompatible)?;
    let size = usize::from(u16::from_le_bytes([bytes[54], bytes[55]]));
    let count = usize::from(u16::from_le_bytes([bytes[56], bytes[57]]));
    if size < 56
        || count == 0
        || count > 128
        || offset
            .checked_add(size * count)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(ProviderError::Incompatible);
    }
    if (0..count)
        .any(|index| bytes[offset + index * size..offset + index * size + 4] == 3u32.to_le_bytes())
    {
        return Err(ProviderError::Incompatible);
    }
    Ok(arch)
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
