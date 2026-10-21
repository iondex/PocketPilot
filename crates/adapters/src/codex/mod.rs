//! Typed observation and four explicit mutation methods on existing Codex app servers.
mod adapter;

pub use adapter::Factory;

pub mod discovery;

#[cfg(test)]
mod live_tests;

pub mod protocol;

pub mod provider;

pub mod rpc;

pub mod transport;

pub use provider::Codex;
pub use rpc::{Client, Error, Event, ReadRequest};

mod errors;

/// Build native proxy arguments without leaking Codex CLI syntax to the engine.
pub fn proxy_endpoint(
    host: connections::Host,
    binary: String,
    socket: String,
) -> Result<connections::Endpoint, Error> {
    if matches!(host, connections::Host::Local) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if !std::fs::metadata(&socket).is_ok_and(|metadata| metadata.file_type().is_socket()) {
                return Err(Error::Endpoint);
            }
        }
    }
    Ok(connections::Endpoint::Proxy {
        host,
        program: connections::Program {
            binary,
            arguments: vec!["app-server".into(), "proxy".into(), "--sock".into(), socket],
        },
    })
}

/// Establish the Codex handshake over a selected host transport; never start a daemon.
pub async fn connect(
    endpoint: connections::Endpoint,
    timeouts: connections::Timeouts,
) -> Result<Client, Error> {
    Client::connect(endpoint, timeouts.connect, timeouts.read)
        .await
        .map(|client| client.with_write_timeout(timeouts.write_receipt))
}

mod questions;
