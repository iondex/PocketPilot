//! Client of the independent Claude server; only its proxy process belongs to this handle.
use crate::{Adapter, AdapterFactory, ConnectOptions, RuntimeKind};
use connections::{Connection, Endpoint};
use futures_util::{SinkExt, StreamExt};
use session::{
    AgentProvider, Capabilities, History, Page, ProviderError, Session, SessionQuery,
    provider::{Mutation, Receipt},
    server::{self, Call, Failure, Reply, Request, Response},
};
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    process::{Child, ChildStdin, ChildStdout},
    sync::Mutex,
};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub struct Factory;

#[async_trait::async_trait]
impl AdapterFactory for Factory {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    async fn connect(&self, options: ConnectOptions) -> Result<Arc<dyn Adapter>, ProviderError> {
        let Endpoint::Proxy { host, program } = options.endpoint else {
            return Err(ProviderError::Unsupported);
        };
        let mut command = host
            .command(&program)
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
        let client = Client {
            instance: options.instance,
            control: options.control_verified,
            shared: Arc::new(Shared {
                wire: Mutex::new(Some(Wire {
                    input: FramedWrite::new(
                        input,
                        LinesCodec::new_with_max_length(server::MAX_FRAME_BYTES),
                    ),
                    output: FramedRead::new(
                        output,
                        LinesCodec::new_with_max_length(server::MAX_FRAME_BYTES),
                    ),
                    child,
                    next: 0,
                })),
                connected: AtomicBool::new(true),
                read: options.timeouts.read,
                write: options.timeouts.write_receipt,
            }),
        };
        match client
            .call(
                Call::Initialize {
                    version: server::VERSION,
                    runtime: "claude_code".into(),
                },
                options.timeouts.connect,
            )
            .await?
        {
            Reply::Initialized {
                version, runtime, ..
            } if version == server::VERSION && runtime == "claude_code" => Ok(Arc::new(client)),
            _ => Err(ProviderError::Incompatible),
        }
    }
}

#[derive(Clone)]
struct Client {
    instance: String,
    control: bool,
    shared: Arc<Shared>,
}

struct Shared {
    wire: Mutex<Option<Wire>>,
    connected: AtomicBool,
    read: Duration,
    write: Duration,
}

struct Wire {
    input: FramedWrite<ChildStdin, LinesCodec>,
    output: FramedRead<ChildStdout, LinesCodec>,
    child: Child,
    next: u64,
}

impl Client {
    async fn call(&self, call: Call, limit: Duration) -> Result<Reply, ProviderError> {
        let mut guard = self.shared.wire.lock().await;
        let wire = guard.as_mut().ok_or(ProviderError::Disconnected)?;
        wire.next = wire
            .next
            .checked_add(1)
            .ok_or(ProviderError::Incompatible)?;
        let id = wire.next;
        let result = tokio::time::timeout(limit, async {
            let request = serde_json::to_string(&Request { id, call })
                .map_err(|_| ProviderError::Incompatible)?;
            if request.len() > server::MAX_FRAME_BYTES {
                return Err(ProviderError::Unsupported);
            }
            wire.input
                .send(request)
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            let line = wire
                .output
                .next()
                .await
                .ok_or(ProviderError::Disconnected)?
                .map_err(|_| ProviderError::Disconnected)?;
            let response: Response =
                serde_json::from_str(&line).map_err(|_| ProviderError::Incompatible)?;
            if response.id != id {
                return Err(ProviderError::Incompatible);
            }
            Ok(response.result)
        })
        .await;
        match result {
            Ok(Ok(reply)) => reply.map_err(failure),
            error => {
                // Invalidate on timeout/protocol failure. A late reply must never satisfy
                // a later request, and a mutation is never resent on a new connection.
                self.shared.connected.store(false, Ordering::Release);
                if let Some(mut wire) = guard.take() {
                    let _ = wire.child.start_kill();
                }
                match error {
                    Err(_) => Err(ProviderError::Timeout),
                    Ok(Err(error)) => Err(error),
                    _ => Err(ProviderError::Incompatible),
                }
            }
        }
    }

    fn session(&self, mut session: Session) -> Session {
        session.reference.instance = self.instance.clone();
        session
    }
}

fn failure(error: Failure) -> ProviderError {
    match error {
        Failure::Unsupported => ProviderError::Unsupported,
        Failure::Conflict | Failure::Busy => ProviderError::Conflict,
        Failure::Unknown => ProviderError::Timeout,
        Failure::Internal => ProviderError::Disconnected,
        Failure::Invalid => ProviderError::Rejected(-32602),
        Failure::NotFound => ProviderError::Rejected(-32004),
        Failure::Unauthorized => ProviderError::Rejected(-32003),
    }
}

impl Adapter for Client {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
    }

    fn with_control(&self, control: bool) -> Arc<dyn Adapter> {
        Arc::new(Self {
            control,
            ..self.clone()
        })
    }
}

#[async_trait::async_trait]
impl AgentProvider for Client {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            history: true,
            live_status: true,
            control_verified: self.control,
        }
    }

    async fn list(&self, query: SessionQuery) -> Result<Page<Session>, ProviderError> {
        match self.call(Call::List { query }, self.shared.read).await? {
            Reply::Sessions { page } => Ok(Page {
                data: page.data.into_iter().map(|s| self.session(s)).collect(),
                next_cursor: page.next_cursor,
            }),
            _ => Err(ProviderError::Incompatible),
        }
    }

    async fn read(&self, thread: &str) -> Result<Session, ProviderError> {
        match self
            .call(
                Call::Read {
                    thread: thread.into(),
                },
                self.shared.read,
            )
            .await?
        {
            Reply::Session { session } => Ok(self.session(session)),
            _ => Err(ProviderError::Incompatible),
        }
    }

    async fn history(&self, thread: &str) -> Result<History, ProviderError> {
        match self
            .call(
                Call::History {
                    thread: thread.into(),
                },
                self.shared.read,
            )
            .await?
        {
            Reply::History { history } => Ok(history),
            _ => Err(ProviderError::Incompatible),
        }
    }

    async fn execute(&self, mutation: Mutation) -> Result<Receipt, ProviderError> {
        if !self.control
            || matches!(
                mutation,
                Mutation::Steer { .. } | Mutation::SteerWithFiles { .. }
            )
        {
            return Err(ProviderError::Unsupported);
        }
        let operation = uuid::Uuid::new_v4().to_string();
        match self
            .call(
                Call::Mutate {
                    operation,
                    mutation,
                },
                self.shared.write,
            )
            .await?
        {
            Reply::Receipt { receipt } => Ok(receipt),
            _ => Err(ProviderError::Incompatible),
        }
    }
}
