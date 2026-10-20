//! Runtime-independent host access. Protocol framing and session semantics belong to adapters.
pub mod ssh;

#[cfg(test)]
mod tests;

use std::time::Duration;
use tokio::process::Command;

/// How a proxy process is reached, independent of the protocol spoken by that proxy.
#[derive(Clone, Debug)]
pub enum Host {
    Local,
    SSH(ssh::Options),
}

/// Literal arguments remain separate until the SSH shell boundary.
#[derive(Clone, Debug)]
pub struct Program {
    pub binary: String,
    pub arguments: Vec<String>,
}

/// Host selection and a proxy command, without knowledge of the protocol on its byte stream.
#[derive(Clone, Debug)]
pub enum Endpoint {
    Proxy { host: Host, program: Program },
    Websocket { endpoint: String },
}

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    pub read: Duration,
    pub write_receipt: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection program or endpoint is invalid")]
    Endpoint,
    #[error(transparent)]
    SSH(#[from] ssh::Error),
}

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        match self {
            Self::Endpoint => i18n::Message::new("error.ssh.invalid"),
            Self::SSH(error) => i18n::LocalizedError::message(error),
        }
    }
}

/// Build host commands without starting them; the caller owns the proxy's lifetime.
pub trait Connection: Send + Sync {
    fn command(&self, program: &Program) -> Result<Command, Error>;
}

impl Connection for Host {
    fn command(&self, program: &Program) -> Result<Command, Error> {
        if program.binary.is_empty()
            || program.binary.contains(['\0', '\r', '\n'])
            || program.arguments.iter().any(|part| part.contains('\0'))
        {
            return Err(Error::Endpoint);
        }
        match self {
            Self::Local => {
                let mut command = Command::new(&program.binary);
                command.args(&program.arguments);
                Ok(command)
            }
            Self::SSH(options) => {
                let remote = std::iter::once(&program.binary)
                    .chain(&program.arguments)
                    .map(|part| quote(part))
                    .collect::<Vec<_>>()
                    .join(" ");
                ssh::command(options.clone(), remote).map_err(Error::SSH)
            }
        }
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
