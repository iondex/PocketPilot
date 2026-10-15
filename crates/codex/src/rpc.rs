use crate::{
    protocol as p,
    transport::{self, Connection, Wire},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::{RawValue, to_raw_value};
use std::{collections::HashMap, time::Duration};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

mod sealed {
    pub trait Sealed {}
}

/// A schema-generated, read-only request. Downstream callers cannot add methods.
pub trait ReadRequest: Serialize + sealed::Sealed {
    type Response: DeserializeOwned;
    const METHOD: &'static str;
}
macro_rules! request {
    ($params:ident, $response:ident, $method:literal) => {
        impl sealed::Sealed for p::$params {}
        impl ReadRequest for p::$params {
            type Response = p::$response;
            const METHOD: &'static str = $method;
        }
    };
}
request!(InitializeParams, InitializeResponse, "initialize");
request!(ThreadListParams, ThreadListResponse, "thread/list");
request!(ThreadReadParams, ThreadReadResponse, "thread/read");
request!(
    ThreadLoadedListParams,
    ThreadLoadedListResponse,
    "thread/loaded/list"
);
request!(
    ThreadTurnsListParams,
    ThreadTurnsListResponse,
    "thread/turns/list"
);
request!(
    ThreadItemsListParams,
    ThreadItemsListResponse,
    "thread/items/list"
);

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("app-server disconnected or could not be reached")]
    Disconnected,
    #[error("app-server request timed out")]
    Timeout,
    #[error("app-server returned an incompatible protocol message")]
    Protocol,
    #[error("app-server rejected request (code {0})")]
    Rejected(i64),
    #[error("endpoint is missing or does not satisfy connection policy")]
    Endpoint,
    #[error("connection profile is not supported yet")]
    Unsupported,
}

#[derive(Clone)]
pub struct Event {
    pub method: String,
    pub request_id: Option<p::RequestId>,
    /// Opaque extension payload. Consumers may deserialize a known generated DTO.
    pub params: Option<Box<RawValue>>,
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("method", &self.method)
            .field("is_request", &self.request_id.is_some())
            .finish()
    }
}

#[derive(Serialize)]
struct RequestEnvelope<'a> {
    id: p::RequestId,
    method: &'static str,
    params: &'a RawValue,
}
#[derive(Deserialize)]
struct Incoming {
    id: Option<p::RequestId>,
    method: Option<String>,
    params: Option<Box<RawValue>>,
    result: Option<Box<RawValue>>,
    error: Option<Failure>,
}
#[derive(Deserialize)]
struct Failure {
    code: i64,
}

// RawValue exists only at this RPC framing boundary. It is never indexed or modified.
type Reply = Result<Box<RawValue>, Error>;
enum Command {
    Call {
        method: &'static str,
        params: Box<RawValue>,
        deadline: Instant,
        reply: oneshot::Sender<Reply>,
    },
    Initialized {
        reply: oneshot::Sender<Result<(), Error>>,
    },
}

#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    timeout: Duration,
}

impl Client {
    pub async fn connect(
        connection: Connection,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, Error> {
        let ready_deadline = Instant::now() + connect_timeout;
        let wire = tokio::time::timeout_at(ready_deadline, transport::connect(connection))
            .await
            .map_err(|_| Error::Timeout)??;
        let mut client = Self::spawn(
            wire,
            ready_deadline.saturating_duration_since(Instant::now()),
        );
        client
            .call(&p::InitializeParams {
                client_info: p::ClientInfo {
                    name: "pocketpilot".into(),
                    title: None,
                    version: env!("CARGO_PKG_VERSION").into(),
                },
                capabilities: Some(p::InitializeCapabilities::default()),
            })
            .await?;
        let (reply, receive) = oneshot::channel();
        client
            .commands
            .send(Command::Initialized { reply })
            .await
            .map_err(|_| Error::Disconnected)?;
        tokio::time::timeout_at(ready_deadline, receive)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Disconnected)??;
        client.timeout = request_timeout;
        Ok(client)
    }

    fn spawn(wire: Box<dyn Wire>, timeout: Duration) -> Self {
        let (commands, receiver) = mpsc::channel(128);
        let (events, _) = broadcast::channel(256);
        tokio::spawn(run(wire, receiver, events.clone()));
        Self {
            commands,
            events,
            timeout,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn call<R: ReadRequest>(&self, params: &R) -> Result<R::Response, Error> {
        let deadline = Instant::now() + self.timeout;
        let (reply, receive) = oneshot::channel();
        let params = to_raw_value(params).map_err(|_| Error::Protocol)?;
        tokio::time::timeout_at(
            deadline,
            self.commands.send(Command::Call {
                method: R::METHOD,
                params,
                deadline,
                reply,
            }),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::Disconnected)?;
        let body = tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Disconnected)??;
        serde_json::from_str(body.get()).map_err(|_| Error::Protocol)
    }
}

type Pending = HashMap<i64, (Instant, oneshot::Sender<Reply>)>;

async fn run(
    mut wire: Box<dyn Wire>,
    mut commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<Event>,
) {
    let mut pending = Pending::new();
    let mut next_id = 1_i64;
    let mut cleanup = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    None => break,
                    Some(Command::Initialized { reply }) => {
                        let result = tokio::time::timeout(Duration::from_secs(10), wire.send("{\"method\":\"initialized\"}".into())).await
                            .map_err(|_| Error::Timeout).and_then(|r| r);
                        let failed = result.is_err();
                        let _ = reply.send(result);
                        if failed { break; }
                    }
                    Some(Command::Call { method, params, deadline, reply }) => {
                        if reply.is_closed() || deadline <= Instant::now() {
                            let _ = reply.send(Err(Error::Timeout));
                            continue;
                        }
                        if pending.len() >= 128 {
                            let _ = reply.send(Err(Error::Rejected(-32001)));
                            continue;
                        }
                        let id = next_id;
                        let Some(next) = next_id.checked_add(1) else {
                            let _ = reply.send(Err(Error::Protocol));
                            break;
                        };
                        next_id = next;
                        let envelope = RequestEnvelope { id:p::RequestId::Int64(id), method, params:&params };
                        let message = match serde_json::to_string(&envelope) {
                            Ok(value) => value,
                            Err(_) => { let _ = reply.send(Err(Error::Protocol)); continue; }
                        };
                        pending.insert(id, (deadline, reply));
                        if !matches!(tokio::time::timeout_at(deadline, wire.send(message)).await, Ok(Ok(()))) { break; }
                        tracing::debug!(rpc_id=id, method, "read-only request sent");
                    }
                }
            }
            received = wire.receive() => {
                let text = match received { Ok(Some(value)) => value, _ => break };
                let value: Incoming = match serde_json::from_str(&text) { Ok(value) => value, Err(_) => break };
                if let Some(method) = value.method {
                    let _ = events.send(Event { method, request_id:value.id, params:value.params });
                    // Observe server requests without answering, cancelling or changing policies.
                    continue;
                }
                let Some(p::RequestId::Int64(id)) = value.id else { continue; };
                if let Some((_, reply)) = pending.remove(&id) {
                    let result = match value.error {
                        Some(error) => Err(Error::Rejected(error.code)),
                        None => value.result.ok_or(Error::Protocol),
                    };
                    let _ = reply.send(result);
                }
            }
            _ = cleanup.tick() => {
                let now = Instant::now();
                let expired: Vec<_> = pending.iter().filter(|(_, (deadline, reply))| *deadline <= now || reply.is_closed()).map(|(id, _)| *id).collect();
                for id in expired {
                    if let Some((_, reply)) = pending.remove(&id) { let _ = reply.send(Err(Error::Timeout)); }
                }
            }
        }
    }
    for (_, (_, reply)) in pending {
        let _ = reply.send(Err(Error::Disconnected));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::JsonLines;
    use futures_util::{SinkExt, StreamExt};
    use tokio_util::codec::{Framed, LinesCodec};

    #[tokio::test]
    async fn websocket_handshake_uses_schema_types_and_initialized_notification() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let request: Incoming = serde_json::from_str(&frame).unwrap();
            assert_eq!(request.method.as_deref(), Some("initialize"));
            let params: p::InitializeParams =
                serde_json::from_str(request.params.unwrap().get()).unwrap();
            assert!(!params.capabilities.unwrap().experimental_api);
            socket.send(tokio_tungstenite::tungstenite::Message::Text(format!(
                "{{\"id\":{},\"result\":{{\"codexHome\":\"/tmp/test-codex\",\"userAgent\":\"test\",\"platformFamily\":\"unix\",\"platformOs\":\"test\"}}}}", request.id.unwrap()
            ).into())).await.unwrap();
            let initialized: Incoming =
                serde_json::from_str(&socket.next().await.unwrap().unwrap().into_text().unwrap())
                    .unwrap();
            assert_eq!(initialized.method.as_deref(), Some("initialized"));
            assert!(initialized.id.is_none());
        });
        let client = Client::connect(
            Connection::Websocket { endpoint },
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        server.await.unwrap();
        drop(client);
    }

    #[tokio::test]
    async fn multiplexes_typed_responses_and_never_answers_server_requests() {
        let (local, remote) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(local);
        let client = Client::spawn(
            Box::new(JsonLines::new(read, write, None)),
            Duration::from_secs(2),
        );
        let mut server = Framed::new(remote, LinesCodec::new());
        let task = tokio::spawn(async move {
            let a: Incoming = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            let b: Incoming = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(a.method.as_deref(), Some("thread/list"));
            assert_eq!(b.method.as_deref(), Some("thread/loaded/list"));
            server.send("{\"id\":\"approval-1\",\"method\":\"item/commandExecution/requestApproval\",\"params\":{\"threadId\":\"t\"}}".to_owned()).await.unwrap();
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"data\":[\"t\"]}}}}",
                    b.id.unwrap()
                ))
                .await
                .unwrap();
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"data\":[]}}}}",
                    a.id.unwrap()
                ))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), server.next())
                    .await
                    .is_err()
            );
        });
        let mut events = client.subscribe();
        let list = p::ThreadListParams::default();
        let loaded = p::ThreadLoadedListParams::default();
        let (a, b) = tokio::join!(client.call(&list), client.call(&loaded));
        assert!(a.unwrap().data.is_empty());
        assert_eq!(b.unwrap().data, vec!["t"]);
        assert!(
            matches!(events.recv().await.unwrap().request_id,Some(p::RequestId::String(id)) if id == "approval-1")
        );
        task.await.unwrap();
    }
    #[tokio::test]
    async fn timeout_does_not_resend_or_cancel() {
        let (local, remote) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(local);
        let client = Client::spawn(
            Box::new(JsonLines::new(read, write, None)),
            Duration::from_millis(30),
        );
        let mut server = Framed::new(remote, LinesCodec::new());
        let request =
            tokio::spawn(async move { client.call(&p::ThreadListParams::default()).await });
        let incoming: Incoming =
            serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(incoming.method.as_deref(), Some("thread/list"));
        assert!(matches!(request.await.unwrap(), Err(Error::Timeout)));
        assert!(server.next().await.is_none());
    }
}
