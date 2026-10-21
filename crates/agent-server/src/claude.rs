//! Claude's stream-json boundary. The server owns pipes; RPC clients never own the child.
use crate::storage::Store;
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
}

#[derive(Deserialize)]
struct ControlRequest {
    subtype: String,
    #[serde(default)]
    tool_name: Option<String>,
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
        let driver = Self {
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
            requests: HashMap::new(),
        };
        // Intentionally detached from the RPC task. Dropping a proxy cannot end the native turn.
        tokio::spawn(async move {
            let mut owner = owner;
            let mut lines =
                FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_NATIVE_FRAME));
            while let Some(Ok(line)) = lines.next().await {
                let Ok(frame) = serde_json::from_str::<Output>(&line) else {
                    break;
                };
                if owner.frame(frame).await.is_err() {
                    break;
                }
            }
            alive.store(false, Ordering::Release);
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
    requests: HashMap<String, bool>,
}

impl Reader {
    async fn frame(&mut self, frame: Output) -> Result<(), Failure> {
        match frame {
            Output::ControlResponse { response } if response.request_id == self.init_id => {
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
                self.requests.clear();
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
                        self.store
                            .status(
                                &self.thread,
                                RuntimeStatus::Active {
                                    waiting_on_approval: waiting
                                        && self.requests.values().any(|q| !*q),
                                    waiting_on_user_input: waiting
                                        && self.requests.values().any(|q| *q),
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
                if self.requests.len() >= 64 {
                    return Err(Failure::Busy);
                }
                // AskUserQuestion is not a permission approval. Response support is planned separately.
                let question = request.subtype == "can_use_tool"
                    && request.tool_name.as_deref() == Some("AskUserQuestion");
                self.requests.insert(request_id, question);
                self.waiting().await?;
            }
            Output::ControlCancelRequest { request_id } => {
                self.requests.remove(&request_id);
                self.waiting().await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn waiting(&self) -> Result<(), Failure> {
        let turn = self.round.lock().await;
        self.store
            .status(
                &self.thread,
                RuntimeStatus::Active {
                    waiting_on_approval: self.requests.values().any(|q| !*q),
                    waiting_on_user_input: self.requests.values().any(|q| *q),
                },
                turn.as_ref().map(|t| t.id.as_str()),
            )
            .await
    }
}
