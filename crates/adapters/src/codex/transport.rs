use crate::codex::rpc::Error;
use async_trait::async_trait;
use connections::{Connection as _, Endpoint};
use futures_util::{SinkExt, StreamExt};
use std::process::Stdio;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    process::{Child, Command},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

#[cfg(test)]
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub const MAX_FRAME: usize = 64 * 1024 * 1024;

#[async_trait]
pub(crate) trait Wire: Send {
    async fn send(&mut self, text: String) -> Result<(), Error>;

    async fn receive(&mut self) -> Result<Option<String>, Error>;
}

/// Host access is shared; the WebSocket handshake and frame limits are Codex-specific.
pub(crate) async fn connect(endpoint: Endpoint) -> Result<Box<dyn Wire>, Error> {
    match endpoint {
        Endpoint::Proxy { host, program } => {
            let command = host.command(&program).map_err(|_| Error::Endpoint)?;
            spawn_proxy(command).await
        }
        Endpoint::Websocket { endpoint } => {
            let parsed = url::Url::parse(&endpoint).map_err(|_| Error::Endpoint)?;
            let local = parsed.host_str().is_some_and(|h| {
                h == "localhost"
                    || h == "[::1]"
                    || h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
            if !parsed.username().is_empty()
                || parsed.password().is_some()
                || !(parsed.scheme() == "wss" || (parsed.scheme() == "ws" && local))
            {
                return Err(Error::Endpoint);
            }
            let config = WebSocketConfig::default()
                .max_message_size(Some(MAX_FRAME))
                .max_frame_size(Some(MAX_FRAME));
            let (socket, _) =
                tokio_tungstenite::connect_async_with_config(endpoint, Some(config), false)
                    .await
                    .map_err(|_| Error::Disconnected)?;
            Ok(Box::new(Websocket {
                socket,
                _proxy: None,
            }))
        }
    }
}

pub(super) async fn spawn_proxy(mut command: Command) -> Result<Box<dyn Wire>, Error> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // This child is exclusively a proxy/SSH client, never an execution daemon.
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error::Disconnected)?;
    let input = child.stdout.take().ok_or(Error::Disconnected)?;
    let output = child.stdin.take().ok_or(Error::Disconnected)?;
    if let Some(mut stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            while let Ok(size) = stderr.read(&mut buffer).await {
                if size == 0 {
                    break;
                }
                // Never log arbitrary remote stderr, which may contain credentials.
                tracing::debug!(bytes = size, "proxy diagnostic output discarded");
            }
        });
    }
    // The official proxy is a byte tunnel, not a JSONL adapter. Codex Unix
    // listeners require the same HTTP Upgrade and WS frames as TCP listeners.
    let stream = tokio::io::join(input, output);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (socket, _) =
        tokio_tungstenite::client_async_with_config("ws://localhost/", stream, Some(config))
            .await
            .map_err(|_| Error::Disconnected)?;
    Ok(Box::new(Websocket {
        socket,
        _proxy: Some(child),
    }))
}

#[cfg(test)]
pub(crate) struct JsonLines {
    input: FramedRead<Box<dyn AsyncRead + Unpin + Send>, LinesCodec>,
    output: FramedWrite<Box<dyn AsyncWrite + Unpin + Send>, LinesCodec>,
    _proxy: Option<Child>,
}

#[cfg(test)]
impl JsonLines {
    pub(crate) fn new(
        input: impl AsyncRead + Unpin + Send + 'static,
        output: impl AsyncWrite + Unpin + Send + 'static,
        proxy: Option<Child>,
    ) -> Self {
        Self {
            input: FramedRead::new(Box::new(input), LinesCodec::new_with_max_length(MAX_FRAME)),
            output: FramedWrite::new(Box::new(output), LinesCodec::new_with_max_length(MAX_FRAME)),
            _proxy: proxy,
        }
    }
}

#[async_trait]
#[cfg(test)]
impl Wire for JsonLines {
    async fn send(&mut self, text: String) -> Result<(), Error> {
        if text.len() > MAX_FRAME {
            return Err(Error::Protocol);
        }
        self.output
            .send(text)
            .await
            .map_err(|_| Error::Disconnected)
    }

    async fn receive(&mut self) -> Result<Option<String>, Error> {
        match self.input.next().await {
            None => Ok(None),
            Some(Err(_)) => Err(Error::Protocol),
            Some(Ok(line)) => Ok(Some(line)),
        }
    }
}

struct Websocket<S> {
    socket: WebSocketStream<S>,
    _proxy: Option<Child>,
}

#[async_trait]
impl<S: AsyncRead + AsyncWrite + Unpin + Send> Wire for Websocket<S> {
    async fn send(&mut self, text: String) -> Result<(), Error> {
        if text.len() > MAX_FRAME {
            return Err(Error::Protocol);
        }
        self.socket
            .send(Message::Text(text.into()))
            .await
            .map_err(|_| Error::Disconnected)
    }

    async fn receive(&mut self) -> Result<Option<String>, Error> {
        loop {
            match self.socket.next().await {
                None | Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Err(_)) => return Err(Error::Disconnected),
                Some(Ok(Message::Text(text))) => {
                    if text.len() > MAX_FRAME {
                        return Err(Error::Protocol);
                    }
                    return Ok(Some(text.to_string()));
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    self.socket.flush().await.map_err(|_| Error::Disconnected)?
                }
                _ => return Err(Error::Protocol),
            }
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
