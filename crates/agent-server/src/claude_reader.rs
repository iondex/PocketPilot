//! Native output ingestion and durable receipts for the server-owned Claude child.
use super::{
    Pending, Round,
    protocol::{Block, MAX_NATIVE_FRAME, NativeState, Output, decode},
};
use crate::{questions::Requests, storage::Store};
use futures_util::StreamExt;
use session::{Role, RuntimeStatus, TextItem, server::Failure};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    process::{Child, ChildStdout},
    sync::{Mutex, oneshot},
};
use tokio_util::codec::{FramedRead, LinesCodec};

pub(super) struct Reader {
    pub(super) store: Store,
    pub(super) thread: String,
    pub(super) pending: Arc<Mutex<HashMap<String, Pending>>>,
    pub(super) round: Arc<Mutex<Option<Round>>>,
    pub(super) init_id: String,
    pub(super) ready: Option<oneshot::Sender<Result<(), Failure>>>,
    pub(super) requests: Arc<Mutex<Requests>>,
}

impl Reader {
    /// Outlive RPC clients and own stdout until native EOF or a protocol failure.
    /// The detached task reaps its own child and never sends kill/interrupt.
    pub(super) async fn run(
        mut self,
        mut child: Child,
        stdout: ChildStdout,
        alive: Arc<AtomicBool>,
    ) {
        let mut lines = FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_NATIVE_FRAME));
        while let Some(Ok(line)) = lines.next().await {
            let Ok(frame) = decode(&line) else {
                break;
            };
            if self.frame(frame).await.is_err() {
                break;
            }
        }
        alive.store(false, Ordering::Release);
        let _ = self.requests.lock().await.finish();
        let _ = self
            .store
            .status(&self.thread, RuntimeStatus::Unknown, None)
            .await;
        for (_, pending) in self.pending.lock().await.drain() {
            let _ = pending.done.send(Err(Failure::Unknown));
        }
        // Reap only this server-owned child; no kill or interrupt is sent on disconnect.
        let _ = child.wait().await;
    }

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
