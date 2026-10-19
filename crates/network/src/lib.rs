//! Explicit per-integration HTTP CONNECT routing. Never reads proxy environment variables.
use std::{future::Future, io, pin::Pin, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::Url;

mod relay;

pub mod download;

pub use relay::Relay;

#[cfg(test)]
mod tests;

pub type WebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

#[derive(Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct Proxy(String);

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Direct"
        } else {
            "Proxy([REDACTED])"
        })
    }
}

impl TryFrom<String> for Proxy {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if !value.is_empty() {
            let url = Url::parse(value).map_err(|_| "invalid proxy URL (redacted)")?;
            if url.scheme() != "http"
                || url.host_str().is_none()
                || url.port_or_known_default().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("proxy requires an unauthenticated http://host:port URL (redacted)");
            }
        }
        Ok(Self(value.into()))
    }
}

fn failed() -> io::Error {
    io::Error::other("network connection failed (details redacted)")
}

impl Proxy {
    pub fn restart_key(&self) -> &str {
        &self.0
    }

    pub fn enabled(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn http_builder(&self) -> Result<reqwest::ClientBuilder, reqwest::Error> {
        let builder = reqwest::Client::builder().no_proxy();
        Ok(if self.enabled() {
            builder.proxy(reqwest::Proxy::all(&self.0)?)
        } else {
            builder
        })
    }

    pub async fn tcp(&self, host: &str, port: u16) -> io::Result<TcpStream> {
        tokio::time::timeout(Duration::from_secs(15), async {
            if !self.enabled() {
                return TcpStream::connect((host, port)).await;
            }
            let proxy = Url::parse(&self.0).map_err(|_| failed())?;
            let mut stream = TcpStream::connect((
                proxy.host_str().ok_or_else(failed)?,
                proxy.port_or_known_default().ok_or_else(failed)?,
            ))
            .await?;
            let authority = if host.contains(':') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            };
            stream
                .write_all(
                    format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes(),
                )
                .await?;
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                if header.len() >= 8192 {
                    return Err(failed());
                }
                header.push(stream.read_u8().await?);
            }
            let status = std::str::from_utf8(&header)
                .map_err(|_| failed())?
                .lines()
                .next()
                .ok_or_else(failed)?;
            let mut parts = status.split_whitespace();
            if !matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1"))
                || parts
                    .next()
                    .and_then(|p| p.parse::<u16>().ok())
                    .is_none_or(|p| !(200..300).contains(&p))
            {
                return Err(failed());
            }
            Ok(stream)
        })
        .await
        .map_err(|_| failed())?
        .map_err(|_| failed())
    }

    pub async fn websocket(&self, endpoint: &str) -> io::Result<WebSocket> {
        tokio::time::timeout(Duration::from_secs(30), async {
            let url = Url::parse(endpoint).map_err(|_| failed())?;
            if !matches!(url.scheme(), "ws" | "wss") {
                return Err(failed());
            }
            let stream = self
                .tcp(
                    url.host_str().ok_or_else(failed)?,
                    url.port_or_known_default().ok_or_else(failed)?,
                )
                .await?;
            let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(4 * 1024 * 1024))
                .max_frame_size(Some(4 * 1024 * 1024));
            tokio_tungstenite::client_async_tls_with_config(endpoint, stream, Some(config), None)
                .await
                .map(|(socket, _)| socket)
                .map_err(|_| failed())
        })
        .await
        .map_err(|_| failed())?
    }
}

/// Plain TCP connector for hyper-rustls; TLS and certificate checks remain in hyper-rustls.
#[derive(Clone, Debug, Default)]
pub struct Connector(pub Proxy);

impl tower_service::Service<http::Uri> for Connector {
    type Response = hyper_util::rt::TokioIo<TcpStream>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<Self::Response>> + Send>>;

    fn poll_ready(&mut self, _: &mut std::task::Context<'_>) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: http::Uri) -> Self::Future {
        let proxy = self.0.clone();
        Box::pin(async move {
            let host = uri.host().ok_or_else(failed)?.trim_matches(['[', ']']);
            let port = uri
                .port_u16()
                .unwrap_or(if uri.scheme_str() == Some("https") {
                    443
                } else {
                    80
                });
            proxy
                .tcp(host, port)
                .await
                .map(hyper_util::rt::TokioIo::new)
        })
    }
}
