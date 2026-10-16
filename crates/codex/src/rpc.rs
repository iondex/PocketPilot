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

pub(crate) trait WriteRequest: Serialize + sealed::Sealed {
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
macro_rules! write_request {
    ($params:ident, $response:ident, $method:literal) => {
        impl sealed::Sealed for p::$params {}

        impl WriteRequest for p::$params {
            type Response = p::$response;

            const METHOD: &'static str = $method;
        }
    };
}
write_request!(ThreadStartParams, ThreadStartResponse, "thread/start");
write_request!(ThreadResumeParams, ThreadResumeResponse, "thread/resume");
write_request!(TurnStartParams, TurnStartResponse, "turn/start");
write_request!(TurnSteerParams, TurnSteerResponse, "turn/steer");

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
    #[error("app-server history is not materialized yet; refresh shortly")]
    HistoryPending,
    #[error("the active turn changed before the input was accepted")]
    TurnChanged,
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
    #[serde(default)]
    message: String,
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
    write_timeout: Duration,
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
                // Needed for paginated history on the deployed 0.161 daemon.
                // Calls remain restricted to the generated, sealed method sets.
                capabilities: Some(p::InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
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
        client.write_timeout = request_timeout;
        Ok(client)
    }

    pub(crate) fn spawn(wire: Box<dyn Wire>, timeout: Duration) -> Self {
        let (commands, receiver) = mpsc::channel(128);
        let (events, _) = broadcast::channel(256);
        tokio::spawn(run(wire, receiver, events.clone()));
        Self {
            commands,
            events,
            timeout,
            write_timeout: timeout,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        !self.commands.is_closed()
    }

    pub fn with_write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }

    pub async fn call<R: ReadRequest>(&self, params: &R) -> Result<R::Response, Error> {
        let deadline = Instant::now() + self.timeout;
        let warming_deadline = Instant::now() + self.timeout.min(Duration::from_secs(5));
        let mut delay = Duration::from_millis(25);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            match self.request(R::METHOD, params, remaining).await {
                Err(Error::HistoryPending) if Instant::now() + delay < warming_deadline => {
                    // Native persistence can briefly expose an empty rollout after
                    // accepting a turn. Retry only this safe read; never its input.
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_millis(250));
                }
                result => return result,
            }
        }
    }

    pub(crate) async fn write<R: WriteRequest>(&self, params: &R) -> Result<R::Response, Error> {
        self.request(R::METHOD, params, self.write_timeout).await
    }

    async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &'static str,
        params: &P,
        timeout: Duration,
    ) -> Result<R, Error> {
        let deadline = Instant::now() + timeout;
        let (reply, receive) = oneshot::channel();
        let params = to_raw_value(params).map_err(|_| Error::Protocol)?;
        tokio::time::timeout_at(
            deadline,
            self.commands.send(Command::Call {
                method,
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
        let mut decoder = serde_json::Deserializer::from_str(body.get());
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            // Only schema field names/array indexes, never the response or error
            // text (which can include message content), reach diagnostics.
            let path=error.path().iter().take(12).map(|segment| match segment {
                serde_path_to_error::Segment::Seq{..}=>"[]",
                serde_path_to_error::Segment::Map{key} if matches!(key.as_str(),"data"|"thread"|"turns"|"items"|"content"|"status"|"phase"|"type"|"result"|"output"|"arguments"|"tool"|"name"|"text"|"error"|"itemsView")=>key.as_str(),
                _=>"?",
            }).collect::<Vec<_>>().join(".");
            let message=error.inner().to_string();
            let category=if message.contains("unknown variant") {"unknown_variant"}else if message.contains("missing field") {"missing_field"}else if message.contains("did not match any variant") {"union_mismatch"}else{"invalid_shape"};
            tracing::warn!(rpc_method=method,schema_path=%path,category,"typed response could not be decoded");
            Error::Protocol
        })
    }
}

type Pending = HashMap<i64, (Instant, oneshot::Sender<Reply>, &'static str)>;

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
                        pending.insert(id, (deadline, reply, method));
                        if !matches!(tokio::time::timeout_at(deadline, wire.send(message)).await, Ok(Ok(()))) { break; }
                        tracing::debug!(rpc_id=id, method, "typed request sent");
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
                if let Some((_, reply, method)) = pending.remove(&id) {
                    let result = match value.error {
                        Some(error) => {
                            // Only fixed diagnostic labels are logged, never the server's text.
                            let lower=error.message.to_lowercase();
                            let terms=["legacy","paginated","materialized","not found","active","summary","experimental","history","persisted","rollout","empty","unavailable","database","turn","read","before","partial"].into_iter().filter(|term|lower.contains(term)).collect::<Vec<_>>();
                            tracing::warn!(rpc_method=method,code=error.code,?terms,"app-server rejected typed request");
                            if history_pending(method, &error) { Err(Error::HistoryPending) }
                            else if turn_changed(method,&error) {Err(Error::TurnChanged)}
                            else { Err(Error::Rejected(error.code)) }
                        },
                        None => value.result.ok_or(Error::Protocol),
                    };
                    let _ = reply.send(result);
                }
            }
            _ = cleanup.tick() => {
                let now = Instant::now();
                let expired: Vec<_> = pending.iter().filter(|(_, (deadline, reply, _))| *deadline <= now || reply.is_closed()).map(|(id, _)| *id).collect();
                for id in expired {
                    if let Some((_, reply, _)) = pending.remove(&id) { let _ = reply.send(Err(Error::Timeout)); }
                }
            }
        }
    }
    for (_, (_, reply, _)) in pending {
        let _ = reply.send(Err(Error::Disconnected));
    }
}

fn turn_changed(method: &str, error: &Failure) -> bool {
    method == "turn/steer"
        && error.code == -32600
        && (error.message == "no active turn to steer"
            || error.message.starts_with("expected active turn id `"))
}

fn history_pending(method: &str, error: &Failure) -> bool {
    matches!(
        method,
        "thread/read" | "thread/turns/list" | "thread/items/list"
    ) && error.code == -32603
        && error.message.contains("rollout")
        && error.message.contains(" is empty")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::JsonLines;
    use futures_util::{SinkExt, StreamExt};
    use tokio_util::codec::{Framed, LinesCodec};

    #[tokio::test]
    async fn retries_only_the_known_empty_rollout_read_race() {
        let (local, remote) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(local);
        let client = Client::spawn(
            Box::new(JsonLines::new(read, write, None)),
            Duration::from_secs(1),
        );
        let task = tokio::spawn(async move {
            let mut server = Framed::new(remote, LinesCodec::new());
            for attempt in 0..2 {
                let request: Incoming =
                    serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
                assert_eq!(request.method.as_deref(), Some("thread/turns/list"));
                let body = if attempt == 0 {
                    r#""error":{"code":-32603,"message":"failed to read rollout: rollout at /private/fixture is empty"}"#
                } else {
                    r#""result":{"data":[]}"#
                };
                server
                    .send(format!("{{\"id\":{},{body}}}", request.id.unwrap()))
                    .await
                    .unwrap();
            }
        });
        let params: p::ThreadTurnsListParams = p::ThreadTurnsListParams::builder()
            .thread_id("fixture")
            .try_into()
            .unwrap();
        assert!(client.call(&params).await.unwrap().data.is_empty());
        task.await.unwrap();
        let error = Failure {
            code: -32603,
            message: "rollout at /fixture is empty".into(),
        };
        assert!(!history_pending("turn/start", &error));
        assert!(!history_pending("turn/steer", &error));
        assert!(!history_pending("thread/resume", &error));
    }

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
            assert!(params.capabilities.unwrap().experimental_api);
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
