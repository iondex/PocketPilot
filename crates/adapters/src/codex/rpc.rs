//! Typed RPC frontend and the single owner of a Codex transport connection.
//!
//! The actor owns request IDs, pending replies and live question tokens. Clients
//! exchange bounded commands; event broadcasts are lossy observation hints, not
//! the source of truth for questions or write acceptance. Dropping a response
//! future does not retract a command already queued or sent to the runtime.
use crate::codex::{
    protocol as p,
    transport::{self, Wire},
};
use connections::Endpoint;
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
write_request!(FsWriteFileParams, FsWriteFileResponse, "fs/writeFile");
write_request!(
    FsCreateDirectoryParams,
    FsCreateDirectoryResponse,
    "fs/createDirectory"
);
write_request!(
    ThreadSetNameParams,
    ThreadSetNameResponse,
    "thread/name/set"
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
    #[error("app-server history is not materialized yet; refresh shortly")]
    HistoryPending,
    #[error("the active turn changed before the input was accepted")]
    TurnChanged,
    #[error("endpoint is missing or does not satisfy connection policy")]
    Endpoint,
    #[error("connection profile is not supported yet")]
    Unsupported,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Disconnected => ::i18n::Message::new("error.codex.disconnected"),
            Self::Timeout => ::i18n::Message::new("error.codex.timeout"),
            Self::Protocol => ::i18n::Message::new("error.codex.protocol"),
            Self::Rejected(value) => {
                ::i18n::Message::new("error.codex.rejected").arg("code", value)
            }
            Self::HistoryPending => ::i18n::Message::new("error.codex.history_pending"),
            Self::TurnChanged => ::i18n::Message::new("error.codex.turn_changed"),
            Self::Endpoint => ::i18n::Message::new("error.codex.endpoint"),
            Self::Unsupported => ::i18n::Message::new("error.codex.unsupported"),
        }
    }
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
    Questions {
        thread: String,
        reply: oneshot::Sender<Vec<session::questions::Dialog>>,
    },
    Answer {
        thread: String,
        submission: session::questions::Submission,
        deadline: Instant,
        reply: oneshot::Sender<Result<Option<String>, Error>>,
    },
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

/// Cloneable handle to one transport actor, shared by observations and control.
#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    timeout: Duration,
    write_timeout: Duration,
}

impl Client {
    pub async fn connect(
        connection: Endpoint,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, Error> {
        let ready_deadline = Instant::now() + connect_timeout;
        let wire = tokio::time::timeout_at(ready_deadline, transport::connect(connection))
            .await
            .map_err(|_| Error::Timeout)??;
        Self::initialize(wire, ready_deadline, request_timeout).await
    }

    /// Handshake and initialized notification share the original connection deadline.
    async fn initialize(
        wire: Box<dyn Wire>,
        ready_deadline: Instant,
        request_timeout: Duration,
    ) -> Result<Self, Error> {
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
        // Bound queued work separately from in-flight calls. Event subscribers
        // may lag without blocking replies or losing the actor's question state.
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

    pub(crate) async fn questions(
        &self,
        thread: &str,
    ) -> Result<Vec<session::questions::Dialog>, Error> {
        let (reply, receive) = oneshot::channel();
        tokio::time::timeout(self.timeout, async {
            self.commands
                .send(Command::Questions {
                    thread: thread.into(),
                    reply,
                })
                .await
                .map_err(|_| Error::Disconnected)?;
            receive.await.map_err(|_| Error::Disconnected)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub(crate) async fn answer(
        &self,
        thread: String,
        submission: session::questions::Submission,
    ) -> Result<Option<String>, Error> {
        let deadline = Instant::now() + self.write_timeout;
        let (reply, receive) = oneshot::channel();
        tokio::time::timeout_at(deadline, async {
            self.commands
                .send(Command::Answer {
                    thread,
                    submission,
                    deadline,
                    reply,
                })
                .await
                .map_err(|_| Error::Disconnected)?;
            receive.await.map_err(|_| Error::Disconnected)?
        })
        .await
        .map_err(|_| Error::Timeout)?
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

    /// Perform a sealed read request, retrying only transient history materialization.
    ///
    /// # Errors
    ///
    /// Returns connection, protocol, rejection or deadline errors. The original
    /// request deadline bounds all history retries.
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

    /// Send one mutation without retries; a timeout may follow remote acceptance.
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
        decode_reply(method, &body)
    }
}

// Keep protocol diagnostics at the boundary and log only fixed labels. Native
// error strings and decoder paths can contain user content or extension keys.
fn decode_reply<R: DeserializeOwned>(method: &str, body: &RawValue) -> Result<R, Error> {
    let mut decoder = serde_json::Deserializer::from_str(body.get());
    serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
        let path = error.path().iter().take(12).map(|segment| match segment {
            serde_path_to_error::Segment::Seq { .. } => "[]",
            serde_path_to_error::Segment::Map { key }
                if matches!(key.as_str(), "data" | "thread" | "turns" | "items" | "content"
                    | "status" | "phase" | "type" | "result" | "output" | "arguments"
                    | "tool" | "name" | "text" | "error" | "itemsView") => key.as_str(),
            _ => "?",
        }).collect::<Vec<_>>().join(".");
        let message = error.inner().to_string();
        let category = if message.contains("unknown variant") {
            "unknown_variant"
        } else if message.contains("missing field") {
            "missing_field"
        } else if message.contains("did not match any variant") {
            "union_mismatch"
        } else {
            "invalid_shape"
        };
        tracing::warn!(rpc_method = method, schema_path = %path, category, "typed response could not be decoded");
        Error::Protocol
    })
}

fn reject(method: &str, error: Failure) -> Error {
    let lower = error.message.to_lowercase();
    let terms = [
        "legacy",
        "paginated",
        "materialized",
        "not found",
        "active",
        "summary",
        "experimental",
        "history",
        "persisted",
        "rollout",
        "empty",
        "unavailable",
        "database",
        "turn",
        "read",
        "before",
        "partial",
    ]
    .into_iter()
    .filter(|term| lower.contains(term))
    .collect::<Vec<_>>();
    tracing::warn!(
        rpc_method = method,
        code = error.code,
        ?terms,
        "app-server rejected typed request"
    );
    if history_pending(method, &error) {
        Error::HistoryPending
    } else if turn_changed(method, &error) {
        Error::TurnChanged
    } else {
        Error::Rejected(error.code)
    }
}

struct PendingRequest {
    deadline: Instant,
    reply: oneshot::Sender<Reply>,
    method: &'static str,
}

async fn run(
    mut wire: Box<dyn Wire>,
    mut commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<Event>,
) {
    let mut pending = HashMap::<i64, PendingRequest>::new();
    let mut questions = super::questions::Questions::default();
    let mut next_id = 1_i64;
    let mut cleanup = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    None => break,
                    Some(Command::Questions { thread, reply }) => { let _ = reply.send(questions.list(&thread)); }
                    Some(Command::Answer { thread, submission, deadline, reply }) => {
                        if reply.is_closed() || deadline <= Instant::now() {
                            let _ = reply.send(Err(Error::Timeout));
                            continue;
                        }
                        let (message, turn) = match questions.answer(&thread, submission) {
                            Ok(value) => value,
                            Err(error) => { let _ = reply.send(Err(error)); continue; }
                        };
                        let result = tokio::time::timeout_at(deadline, wire.send(message)).await
                            .map_err(|_| Error::Timeout).and_then(|r| r).map(|()| turn);
                        let failed = result.is_err();
                        let _ = reply.send(result);
                        if failed { break; }
                    }
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
                        // Register before sending: a partial/failed send may have
                        // reached the runtime, so disconnect must notify this caller too.
                        pending.insert(id, PendingRequest { deadline, reply, method });
                        if !matches!(tokio::time::timeout_at(deadline, wire.send(message)).await, Ok(Ok(()))) { break; }
                        tracing::debug!(rpc_id=id, method, "typed request sent");
                    }
                }
            }
            received = wire.receive() => {
                let text = match received { Ok(Some(value)) => value, _ => break };
                let value: Incoming = match serde_json::from_str(&text) { Ok(value) => value, Err(_) => break };
                if let Some(method) = value.method {
                    if let Some(params) = value.params.as_ref() {
                        let tracked = match (method.as_str(), value.id.as_ref()) {
                            ("item/tool/requestUserInput", Some(id)) => serde_json::from_str::<p::ToolRequestUserInputParams>(params.get())
                                .map_err(|_| Error::Protocol).and_then(|params| questions.receive(id.clone(), params)),
                            ("serverRequest/resolved", _) => serde_json::from_str::<p::ServerRequestResolvedNotification>(params.get())
                                .map(|n| questions.resolved(n)).map_err(|_| Error::Protocol),
                            ("turn/completed", _) => serde_json::from_str::<p::TurnCompletedNotification>(params.get())
                                .map(|n| questions.completed(&n.thread_id, &n.turn.id)).map_err(|_| Error::Protocol),
                            _ => Ok(()),
                        };
                        if tracked.is_err() { break; }
                    }
                    // Track questions above before this lossy broadcast. Missing
                    // a UI wakeup must not make a live native question unanswerable.
                    let _ = events.send(Event { method, request_id:value.id, params:value.params });
                    // Question responses require a separate explicit command; approvals remain untouched.
                    continue;
                }
                let Some(p::RequestId::Int64(id)) = value.id else { continue; };
                if let Some(PendingRequest { reply, method, .. }) = pending.remove(&id) {
                    let result = match value.error {
                        Some(error) => Err(reject(method, error)),
                        None => value.result.ok_or(Error::Protocol),
                    };
                    let _ = reply.send(result);
                }
            }
            _ = cleanup.tick() => {
                let now = Instant::now();
                let expired: Vec<_> = pending.iter()
                    .filter(|(_, request)| request.deadline <= now || request.reply.is_closed())
                    .map(|(id, _)| *id)
                    .collect();
                for id in expired {
                    if let Some(request) = pending.remove(&id) {
                        let _ = request.reply.send(Err(Error::Timeout));
                    }
                }
            }
        }
    }
    for (_, request) in pending {
        let _ = request.reply.send(Err(Error::Disconnected));
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
#[path = "rpc_tests.rs"]
mod tests;
