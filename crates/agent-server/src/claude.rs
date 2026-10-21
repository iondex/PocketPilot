//! Claude's stream-json boundary. The server owns pipes; RPC clients never own the child.
use crate::{
    questions::{Request as ControlRequest, Requests, Snapshot},
    storage::Store,
};
use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use session::{Role, RuntimeStatus, TextItem, files::Uploaded, provider::Receipt, server::Failure};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{ChildStdin, Command},
    sync::{Mutex, oneshot},
};
use tokio_util::codec::{FramedRead, LinesCodec};

const MAX_NATIVE_FRAME: usize = 32 * 1024 * 1024;

pub struct Driver {
    writer: Mutex<ChildStdin>,
    requests: Arc<Mutex<Requests>>,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    round: Arc<Mutex<Option<Round>>>,
    alive: Arc<AtomicBool>,
    store: Store,
    thread: String,
}

struct Pending {
    operation: String,
    text: String,
    files: Vec<Uploaded>,
    receipt: Receipt,
    done: oneshot::Sender<Result<Receipt, Failure>>,
}

struct Round {
    id: String,
    client: String,
    has_text: bool,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Input<'a> {
    ControlResponse {
        response: crate::questions::Response,
    },
    ControlRequest {
        request_id: &'a str,
        request: Initialize,
    },
    User {
        uuid: &'a str,
        session_id: &'a str,
        parent_tool_use_id: Option<&'a str>,
        message: Message<'a>,
    },
}

#[derive(Serialize)]
struct Initialize {
    subtype: &'static str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: Content<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Content<'a> {
    Text(&'a str),
    Blocks(Vec<InputBlock<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputBlock<'a> {
    Text { text: &'a str },
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: String,
}

async fn content<'a>(body: &'a str, files: &[Uploaded]) -> Result<Content<'a>, Failure> {
    use tokio::io::AsyncReadExt;
    if !files.iter().any(|f| f.image) {
        return Ok(Content::Text(body));
    }
    let mut blocks = vec![InputBlock::Text { text: body }];
    for file in files.iter().filter(|f| f.image) {
        let mut bytes = Vec::new();
        tokio::fs::File::open(&file.path)
            .await
            .map_err(|_| Failure::Unauthorized)?
            .take(5 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Failure::Internal)?;
        if bytes.len() > 5 * 1024 * 1024 {
            return Err(Failure::Unsupported);
        }
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
            "image/webp"
        } else {
            return Err(Failure::Unsupported);
        };
        blocks.push(InputBlock::Image {
            source: ImageSource {
                kind: "base64",
                media_type: mime,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        });
    }
    Ok(Content::Blocks(blocks))
}

// Unknown extensions are ignored at this boundary, never inspected as business JSON.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Output {
    ControlResponse {
        response: ControlResponse,
    },
    ControlRequest {
        request_id: String,
        request: ControlRequest,
    },
    ControlCancelRequest {
        request_id: String,
    },
    User {
        #[serde(default)]
        uuid: Option<String>,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default, rename = "isReplay")]
        replay: bool,
    },
    Assistant {
        session_id: String,
        uuid: String,
        message: Assistant,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
    },
    Result {
        session_id: String,
        subtype: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        result: Option<String>,
    },
    System {
        subtype: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        state: Option<NativeState>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativeState {
    Idle,
    Running,
    RequiresAction,
}

#[derive(Deserialize)]
struct ControlResponse {
    subtype: String,
    request_id: String,
    #[serde(default)]
    pending_permission_requests: Option<Vec<Snapshot>>,
    #[serde(default)]
    pending_user_dialog_requests: Option<Vec<Snapshot>>,
}

#[derive(Deserialize)]
struct Assistant {
    content: Vec<Block>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

/// Decode control payloads directly from JSON. Serde's internally tagged enum
/// buffering cannot preserve RawValue extensions in nested tool input/snapshots.
fn decode(line: &str) -> Result<Output, serde_json::Error> {
    #[derive(Deserialize)]
    struct Kind {
        #[serde(rename = "type")]
        kind: String,
    }

    #[derive(Deserialize)]
    struct RequestFrame {
        request_id: String,
        request: ControlRequest,
    }

    #[derive(Deserialize)]
    struct ResponseFrame {
        response: ControlResponse,
    }

    match serde_json::from_str::<Kind>(line)?.kind.as_str() {
        "control_request" => {
            let frame: RequestFrame = serde_json::from_str(line)?;
            Ok(Output::ControlRequest {
                request_id: frame.request_id,
                request: frame.request,
            })
        }
        "control_response" => {
            let frame: ResponseFrame = serde_json::from_str(line)?;
            Ok(Output::ControlResponse {
                response: frame.response,
            })
        }
        _ => serde_json::from_str(line),
    }
}

impl Driver {
    pub async fn open(
        store: Store,
        thread: &str,
        binary: &Path,
        cwd: &Path,
    ) -> Result<Self, Failure> {
        let started = store.started(thread).await?;
        let mut command = Command::new(binary);
        command
            .args([
                "--print",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "manual",
                "--permission-prompt-tool",
                "stdio",
                "--replay-user-messages",
            ])
            .arg(format!(
                "{}={thread}",
                if started { "--resume" } else { "--session-id" }
            ))
            .env("CLAUDE_CODE_SDK_READS_SESSION_STATE", "1")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Mark before spawn: even an ambiguous crash must not reuse --session-id as a new session.
        store.mark_started(thread).await?;
        let mut child = command.spawn().map_err(|_| Failure::Internal)?;
        let writer = child.stdin.take().ok_or(Failure::Internal)?;
        let stdout = child.stdout.take().ok_or(Failure::Internal)?;
        let alive = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let round = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = oneshot::channel();
        let init_id = uuid::Uuid::new_v4().to_string();
        let requests = Arc::new(Mutex::new(Requests::default()));
        let driver = Self {
            requests: requests.clone(),
            writer: Mutex::new(writer),
            pending: pending.clone(),
            round: round.clone(),
            alive: alive.clone(),
            store: store.clone(),
            thread: thread.into(),
        };
        let owner = Reader {
            store: store.clone(),
            thread: thread.into(),
            pending,
            round,
            init_id: init_id.clone(),
            ready: Some(ready_tx),
            requests,
        };
        // Intentionally detached from the RPC task. Dropping a proxy cannot end the native turn.
        tokio::spawn(async move {
            let mut owner = owner;
            let mut lines =
                FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_NATIVE_FRAME));
            while let Some(Ok(line)) = lines.next().await {
                let Ok(frame) = decode(&line) else {
                    break;
                };
                if owner.frame(frame).await.is_err() {
                    break;
                }
            }
            alive.store(false, Ordering::Release);
            let _ = owner.requests.lock().await.finish();
            let _ = owner
                .store
                .status(&owner.thread, RuntimeStatus::Unknown, None)
                .await;
            for (_, pending) in owner.pending.lock().await.drain() {
                let _ = pending.done.send(Err(Failure::Unknown));
            }
            // Reap only this server-owned child; no kill or interrupt is sent on disconnect.
            let _ = child.wait().await;
        });
        driver
            .write(&Input::ControlRequest {
                request_id: &init_id,
                request: Initialize {
                    subtype: "initialize",
                },
            })
            .await?;
        tokio::time::timeout(Duration::from_secs(30), ready_rx)
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)??;
        Ok(driver)
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    async fn write(&self, input: &Input<'_>) -> Result<(), Failure> {
        let mut bytes = serde_json::to_vec(input).map_err(|_| Failure::Internal)?;
        if bytes.len() > MAX_NATIVE_FRAME {
            return Err(Failure::Invalid);
        }
        bytes.push(b'\n');
        tokio::time::timeout(
            Duration::from_secs(15),
            self.writer.lock().await.write_all(&bytes),
        )
        .await
        .map_err(|_| Failure::Unknown)?
        .map_err(|_| Failure::Unknown)
    }

    pub async fn questions(&self) -> Result<Vec<session::questions::Dialog>, Failure> {
        if !self.alive() {
            return Err(Failure::Conflict);
        }
        Ok(self.requests.lock().await.list())
    }

    pub async fn answer(
        &self,
        submission: session::questions::Submission,
    ) -> Result<Receipt, Failure> {
        let mut requests = self.requests.lock().await;
        if !self.alive() {
            return Err(Failure::Conflict);
        }
        let (response, turn) = requests.answer(submission)?;
        // Consume the token before touching stdin. Failed/uncertain writes are never repeated.
        let (approval, question) = requests.waiting();
        self.store
            .status(
                &self.thread,
                RuntimeStatus::Active {
                    waiting_on_approval: approval,
                    waiting_on_user_input: question,
                },
                turn.as_deref(),
            )
            .await?;
        self.write(&Input::ControlResponse { response }).await?;
        Ok(Receipt {
            thread: self.thread.clone(),
            turn,
        })
    }

    pub async fn start(
        &self,
        operation: &str,
        text: String,
        client: String,
        files: Vec<Uploaded>,
    ) -> Result<Receipt, Failure> {
        if !self.alive() {
            return Err(Failure::Unknown);
        }
        if self.round.lock().await.is_some() {
            return Err(Failure::Busy);
        }
        let body = session::files::envelope(&text, &files);
        let content = content(&body, &files).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let receipt = Receipt {
            thread: self.thread.clone(),
            turn: Some(id.clone()),
        };
        let (done, wait) = oneshot::channel();
        let input_id = uuid::Uuid::new_v4().to_string();
        *self.round.lock().await = Some(Round {
            id: id.clone(),
            client,
            has_text: false,
        });
        self.store
            .status(
                &self.thread,
                RuntimeStatus::Active {
                    waiting_on_approval: false,
                    waiting_on_user_input: false,
                },
                Some(&id),
            )
            .await?;
        self.pending.lock().await.insert(
            input_id.clone(),
            Pending {
                operation: operation.into(),
                text,
                files,
                receipt,
                done,
            },
        );
        self.write(&Input::User {
            uuid: &input_id,
            session_id: "default",
            parent_tool_use_id: None,
            message: Message {
                role: "user",
                content,
            },
        })
        .await?;
        tokio::time::timeout(Duration::from_secs(45), wait)
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)?
    }
}

struct Reader {
    store: Store,
    thread: String,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    round: Arc<Mutex<Option<Round>>>,
    init_id: String,
    ready: Option<oneshot::Sender<Result<(), Failure>>>,
    requests: Arc<Mutex<Requests>>,
}

impl Reader {
    async fn frame(&mut self, frame: Output) -> Result<(), Failure> {
        match frame {
            Output::ControlResponse { response } if response.request_id == self.init_id => {
                let snapshots = response
                    .pending_permission_requests
                    .into_iter()
                    .flatten()
                    .chain(response.pending_user_dialog_requests.into_iter().flatten());
                let turn = self.round.lock().await.as_ref().map(|r| r.id.clone());
                for snapshot in snapshots {
                    self.requests.lock().await.receive(
                        snapshot.request_id,
                        snapshot.request,
                        turn.clone(),
                    )?;
                }
                if self.requests.lock().await.waiting() != (false, false) {
                    self.waiting().await?;
                }
                if response.subtype == "success"
                    && !matches!(
                        self.store.read(&self.thread).await?.status,
                        RuntimeStatus::Active { .. }
                    )
                {
                    self.store
                        .status(&self.thread, RuntimeStatus::Idle, None)
                        .await?;
                }
                if let Some(ready) = self.ready.take() {
                    let _ = ready.send(if response.subtype == "success" {
                        Ok(())
                    } else {
                        Err(Failure::Unsupported)
                    });
                }
            }
            Output::User {
                uuid: Some(id),
                parent_tool_use_id: None,
                session_id: Some(session_id),
                replay: true,
            } if session_id == self.thread => {
                if let Some(pending) = self.pending.lock().await.remove(&id) {
                    let round = self.round.lock().await;
                    let turn = round.as_ref().ok_or(Failure::Conflict)?;
                    self.store
                        .item(
                            &self.thread,
                            TextItem {
                                turn_id: turn.id.clone(),
                                item_id: id,
                                role: Role::User,
                                phase: None,
                                text: pending.text,
                                complete: true,
                                client_id: Some(turn.client.clone()),
                                attachments: pending
                                    .files
                                    .into_iter()
                                    .map(|f| session::Attachment { name: f.name })
                                    .collect(),
                            },
                        )
                        .await?;
                    self.store
                        .accept(&pending.operation, &pending.receipt)
                        .await?;
                    let _ = pending.done.send(Ok(pending.receipt));
                }
            }
            Output::Assistant {
                session_id,
                uuid,
                message,
                parent_tool_use_id: None,
            } if session_id == self.thread => {
                let text = message
                    .content
                    .into_iter()
                    .filter_map(|b| match b {
                        Block::Text { text } => Some(text),
                        Block::Other => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if !text.is_empty() {
                    let mut round = self.round.lock().await;
                    if let Some(turn) = round.as_mut() {
                        turn.has_text = true;
                        self.store
                            .item(
                                &self.thread,
                                TextItem {
                                    turn_id: turn.id.clone(),
                                    item_id: uuid,
                                    role: Role::Agent,
                                    phase: Some("commentary".into()),
                                    text,
                                    complete: true,
                                    client_id: None,
                                    attachments: vec![],
                                },
                            )
                            .await?;
                    }
                }
            }
            Output::Result {
                session_id,
                subtype,
                is_error,
                result,
            } if session_id == self.thread => {
                if let Some(turn) = self.round.lock().await.take() {
                    let failed = is_error || subtype != "success";
                    if turn.has_text && !failed {
                        self.store
                            .finish(&self.thread, &turn.id, "final_answer")
                            .await?;
                    } else {
                        self.store
                            .item(
                                &self.thread,
                                TextItem {
                                    turn_id: turn.id,
                                    item_id: "result".into(),
                                    role: Role::Agent,
                                    phase: Some(
                                        if failed {
                                            "turn_failed"
                                        } else {
                                            "final_answer"
                                        }
                                        .into(),
                                    ),
                                    text: if turn.has_text {
                                        String::new()
                                    } else {
                                        result.unwrap_or_default()
                                    },
                                    complete: true,
                                    client_id: None,
                                    attachments: vec![],
                                },
                            )
                            .await?;
                    }
                    self.store
                        .status(
                            &self.thread,
                            if failed {
                                RuntimeStatus::SystemError
                            } else {
                                RuntimeStatus::Idle
                            },
                            None,
                        )
                        .await?;
                }
                self.requests.lock().await.finish()?;
            }
            Output::System {
                subtype,
                session_id: Some(id),
                ..
            } if subtype == "init" && id != self.thread => return Err(Failure::Conflict),
            Output::System {
                subtype,
                session_id: Some(id),
                state: Some(state),
            } if subtype == "session_state_changed" && id == self.thread => {
                let turn = self.round.lock().await;
                match state {
                    NativeState::Idle if turn.is_none() => {
                        self.store
                            .status(&self.thread, RuntimeStatus::Idle, None)
                            .await?
                    }
                    NativeState::Running | NativeState::RequiresAction => {
                        let waiting = matches!(state, NativeState::RequiresAction);
                        let (approval, question) = self.requests.lock().await.waiting();
                        self.store
                            .status(
                                &self.thread,
                                RuntimeStatus::Active {
                                    waiting_on_approval: waiting && approval,
                                    waiting_on_user_input: waiting && question,
                                },
                                turn.as_ref().map(|t| t.id.as_str()),
                            )
                            .await?;
                    }
                    _ => {}
                }
            }
            Output::ControlRequest {
                request_id,
                request,
            } => {
                let turn = self.round.lock().await.as_ref().map(|r| r.id.clone());
                let inserted = self
                    .requests
                    .lock()
                    .await
                    .receive(request_id, request, turn)?;
                if inserted {
                    self.waiting().await?;
                }
            }
            Output::ControlCancelRequest { request_id } => {
                // A late cancellation of an already-finished request must not
                // turn an idle session back into an active waiting state.
                let removed = self.requests.lock().await.cancel(&request_id)?;
                if removed {
                    self.waiting().await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn waiting(&self) -> Result<(), Failure> {
        let turn = self.round.lock().await;
        let (approval, question) = self.requests.lock().await.waiting();
        self.store
            .status(
                &self.thread,
                RuntimeStatus::Active {
                    waiting_on_approval: approval,
                    waiting_on_user_input: question,
                },
                turn.as_ref().map(|t| t.id.as_str()),
            )
            .await
    }
}
