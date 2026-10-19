//! A bounded loopback WebSocket adapter for SDKs without a connector hook.
use crate::{Proxy, failed};
use futures_util::{SinkExt, StreamExt};
use std::{io, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

pub struct Relay {
    pub endpoint: String,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Relay {
    pub async fn start(proxy: Proxy, upstream: String) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("ws://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        if tasks.len() >= 4 { continue; }
                        let proxy = proxy.clone();
                        let upstream = upstream.clone();
                        tasks.spawn(async move { let _ = forward(stream, proxy, upstream).await; });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Ok(Self { endpoint, task })
    }
}

// tungstenite fixes the callback error type to an HTTP response.
#[allow(clippy::result_large_err)]
async fn forward(stream: TcpStream, proxy: Proxy, upstream: String) -> io::Result<()> {
    let mut target = url::Url::parse(&upstream).map_err(|_| failed())?;
    let mut local = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::accept_hdr_async(stream, |req: &Request, response: Response| {
            if req.headers().contains_key("origin") {
                return Err(http::Response::builder()
                    .status(403)
                    .body(None)
                    .expect("fixed status"));
            }
            if let Some(query) = req.uri().query() {
                target.set_query(Some(query));
            }
            Ok(response)
        }),
    )
    .await
    .map_err(|_| failed())?
    .map_err(|_| failed())?;
    let mut remote = proxy.websocket(target.as_str()).await?;
    loop {
        tokio::select! {
            message = local.next() => {
                let Some(Ok(message)) = message else { break };
                tokio::time::timeout(Duration::from_secs(30), remote.send(message)).await.map_err(|_| failed())?.map_err(|_| failed())?;
            }
            message = remote.next() => {
                let Some(Ok(message)) = message else { break };
                tokio::time::timeout(Duration::from_secs(30), local.send(message)).await.map_err(|_| failed())?.map_err(|_| failed())?;
            }
        }
    }
    Ok(())
}
