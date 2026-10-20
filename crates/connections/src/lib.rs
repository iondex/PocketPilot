//! Execution transport contracts. Integrations never depend on these details.
use codex::{Client, Connection as CodexConnection, SSHOptions};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug)]
pub enum Endpoint {
    LocalProxy { binary: String, socket: PathBuf },
    SshProxy(SSHOptions),
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
    #[error(transparent)]
    Codex(#[from] codex::Error),
    #[error("connection endpoint is not configured")]
    Endpoint,
}

/// The only transport implementation currently shipped: Codex app-server proxy/WebSocket.
pub async fn connect(endpoint: Endpoint, timeouts: Timeouts) -> Result<Client, Error> {
    let connection = match endpoint {
        Endpoint::LocalProxy { binary, socket } => CodexConnection::LocalProxy { binary, socket },
        Endpoint::SshProxy(options) => CodexConnection::SSHProxy(options),
        Endpoint::Websocket { endpoint } => CodexConnection::Websocket { endpoint },
    };
    Client::connect(connection, timeouts.connect, timeouts.read)
        .await
        .map(|client| client.with_write_timeout(timeouts.write_receipt))
        .map_err(Error::Codex)
}
