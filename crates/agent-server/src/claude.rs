//! Claude's stream-json boundary. The server owns pipes; RPC clients never own the child.
use crate::{questions::Requests, storage::Store};
use session::{RuntimeStatus, files::Uploaded, provider::Receipt, server::Failure};
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

#[path = "claude_protocol.rs"]
mod protocol;

#[path = "claude_reader.rs"]
mod reader;

use protocol::{Initialize, Input, MAX_NATIVE_FRAME, Message, content};
use reader::Reader;

// Initialization proves protocol readiness; write/accept budgets bound uncertain I/O.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);

const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(45);

/// Own the native stdin and shared lifecycle state of one server-managed session.
///
/// RPC callers borrow this driver through the server's session actor. The output
/// reader is detached from those callers so losing a proxy cannot cancel a turn.
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

impl Driver {
    /// Start or resume the server-owned child and wait for its initialization reply.
    ///
    /// # Errors
    ///
    /// Returns an internal error on spawn failure, unsupported on a rejected
    /// handshake, or unknown when readiness cannot be confirmed within the budget.
    /// The durable started marker is written first; failure never permits a fresh
    /// `--session-id` launch to overwrite a potentially created native session.
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
        tokio::spawn(owner.run(child, stdout, alive));
        driver
            .write(&Input::ControlRequest {
                request_id: &init_id,
                request: Initialize {
                    subtype: "initialize",
                },
            })
            .await?;
        tokio::time::timeout(INITIALIZE_TIMEOUT, ready_rx)
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)??;
        Ok(driver)
    }

    /// Observe reader liveness; this is not proof that the latest write was accepted.
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    async fn write(&self, input: &Input<'_>) -> Result<(), Failure> {
        let mut bytes = serde_json::to_vec(input).map_err(|_| Failure::Internal)?;
        if bytes.len() > MAX_NATIVE_FRAME {
            return Err(Failure::Invalid);
        }
        bytes.push(b'\n');
        tokio::time::timeout(WRITE_TIMEOUT, self.writer.lock().await.write_all(&bytes))
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)
    }

    /// List only questions still owned by this live child process.
    pub async fn questions(&self) -> Result<Vec<session::questions::Dialog>, Failure> {
        if !self.alive() {
            return Err(Failure::Conflict);
        }
        Ok(self.requests.lock().await.list())
    }

    /// Consume a current answer token before writing its response to native stdin.
    ///
    /// # Errors
    ///
    /// Invalid/stale answers fail before dispatch. I/O uncertainty returns unknown;
    /// the consumed token must not be reused even if no reply reaches the caller.
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

    /// Submit one input, waiting for its matching native replay as acceptance proof.
    ///
    /// The caller must hold the server's session actor and record the operation
    /// intent first. A successful stdin write alone is not an accepted receipt.
    ///
    /// # Errors
    ///
    /// Busy rejects overlapping rounds; invalid files fail before input is sent.
    /// A timeout, dropped channel or partial write has an unknown outcome and must
    /// not trigger automatic resubmission.
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
        tokio::time::timeout(ACCEPT_TIMEOUT, wait)
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)?
    }
}
