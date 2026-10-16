use crate::{Client, Error, protocol as p};
use session::provider::{Mutation, Receipt};
use session::{
    AgentProvider, Capabilities, History, Page, ProviderError, Role, RuntimeStatus, Session,
    SessionQuery, SessionRef, TextItem,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct ListCursor {
    instance: String,
    cwd: Option<String>,
    archived: bool,
    loaded: bool,
    backend: Option<String>,
    seen_loaded: Vec<String>,
}

#[derive(Clone)]
pub struct Codex {
    instance: String,
    client: Client,
    control: bool,
    errors: crate::errors::Errors,
}

impl Codex {
    pub fn new(instance: String, client: Client) -> Self {
        let errors = crate::errors::listen(&client);
        Self {
            instance,
            client,
            control: false,
            errors,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Enable only after the configured shared runtime passes the lifecycle probe.
    pub fn with_control(mut self, verified: bool) -> Self {
        self.control = verified;
        self
    }
}

impl From<Error> for ProviderError {
    fn from(error: Error) -> Self {
        match error {
            Error::Timeout => Self::Timeout,
            Error::Rejected(code) => Self::Rejected(code),
            Error::TurnChanged => Self::Conflict,
            Error::Protocol => Self::Incompatible,
            Error::Unsupported => Self::Unsupported,
            _ => Self::Disconnected,
        }
    }
}

pub fn runtime(status: &p::ThreadStatus) -> RuntimeStatus {
    match status {
        p::ThreadStatus::NotLoaded => RuntimeStatus::NotLoaded,
        p::ThreadStatus::Idle => RuntimeStatus::Idle,
        p::ThreadStatus::SystemError => RuntimeStatus::SystemError,
        p::ThreadStatus::Active(flags) => RuntimeStatus::Active {
            waiting_on_approval: flags
                .iter()
                .any(|f| matches!(f, p::ThreadActiveFlag::WaitingOnApproval)),
            waiting_on_user_input: flags
                .iter()
                .any(|f| matches!(f, p::ThreadActiveFlag::WaitingOnUserInput)),
        },
    }
}

fn summary(instance: &str, thread: p::Thread) -> Session {
    let native_title = thread.name.clone().filter(|name| !name.trim().is_empty());
    let active_turn = thread
        .turns
        .iter()
        .find(|turn| matches!(turn.status, p::TurnStatus::InProgress))
        .map(|t| t.id.clone());
    Session {
        reference: SessionRef {
            instance: instance.into(),
            thread: thread.id,
        },
        title: thread
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if thread.preview.is_empty() {
                    "未命名会话".into()
                } else {
                    thread.preview
                }
            }),
        cwd: thread.cwd.to_string(),
        native_title,
        status: runtime(&thread.status),
        active_turn,
        updated_at: Some(thread.updated_at),
    }
}

fn text_item(turn_id: &str, turn_complete: bool, item: p::ThreadItem) -> Option<TextItem> {
    let item = match item {
        p::ThreadItem::UserMessage {
            client_id,
            content,
            id,
        } => {
            let mut text = Vec::new();
            let mut attachments = Vec::new();
            for input in content {
                let name = match input {
                    p::UserInput::TextUserInput { text: body, .. } => {
                        text.push(body);
                        continue;
                    }
                    p::UserInput::LocalImageUserInput { path, .. }
                    | p::UserInput::LocalAudioUserInput { path, .. } => std::path::Path::new(&path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("attachment")
                        .to_owned(),
                    p::UserInput::ImageUserInput(_) => "image".into(),
                    p::UserInput::AudioUserInput { .. } => "audio".into(),
                    _ => continue,
                };
                attachments.push(session::Attachment { name });
            }
            TextItem {
                turn_id: turn_id.into(),
                item_id: id,
                role: Role::User,
                phase: None,
                text: text.join("\n"),
                complete: true,
                client_id,
                attachments,
            }
        }
        p::ThreadItem::AgentMessage {
            id, phase, text, ..
        } => TextItem {
            turn_id: turn_id.into(),
            item_id: id,
            role: Role::Agent,
            complete: turn_complete || matches!(&phase, Some(p::MessagePhase::FinalAnswer)),
            phase: phase.map(|phase| {
                match phase {
                    p::MessagePhase::Commentary => "commentary",
                    p::MessagePhase::PartialAnswer => "partial_answer",
                    p::MessagePhase::FinalAnswer => "final_answer",
                }
                .into()
            }),
            text,
            client_id: None,
            attachments: Vec::new(),
        },
        p::ThreadItem::Plan { id, text } => TextItem {
            turn_id: turn_id.into(),
            item_id: id,
            role: Role::Agent,
            phase: Some("plan".into()),
            text,
            complete: turn_complete,
            client_id: None,
            attachments: Vec::new(),
        },
        _ => return None,
    };
    (!item.text.is_empty() || !item.attachments.is_empty()).then_some(item)
}

fn turn_items(turn: p::Turn) -> Vec<TextItem> {
    let complete = matches!(turn.status, p::TurnStatus::Completed);
    let error = if let Some(error) = turn.error {
        Some(("error", error.message))
    } else {
        match turn.status {
            p::TurnStatus::Failed => Some(("turn_failed", String::new())),
            p::TurnStatus::Interrupted => Some(("turn_interrupted", String::new())),
            _ => None,
        }
    };
    let mut items = turn
        .items
        .into_iter()
        .filter_map(|item| text_item(&turn.id, complete, item))
        .collect::<Vec<_>>();
    if let Some((phase, text)) = error {
        items.push(TextItem {
            turn_id: turn.id,
            item_id: "pp:turn-error".into(),
            role: Role::Agent,
            phase: Some(phase.into()),
            text,
            complete: true,
            client_id: None,
            attachments: vec![],
        });
    }
    items
}

#[async_trait::async_trait]
impl AgentProvider for Codex {
    fn capabilities(&self) -> Capabilities {
        // A schema match alone does not certify live observation or lifecycle safety.
        Capabilities {
            list: true,
            history: true,
            live_status: true,
            control_verified: self.control,
        }
    }

    async fn list(&self, query: SessionQuery) -> Result<Page<Session>, ProviderError> {
        let mut cursor = if let Some(encoded) = &query.cursor {
            let cursor: ListCursor = serde_json::from_str(
                encoded
                    .strip_prefix("pp1:")
                    .ok_or(ProviderError::Incompatible)?,
            )
            .map_err(|_| ProviderError::Incompatible)?;
            if cursor.instance != self.instance
                || cursor.cwd != query.cwd
                || cursor.archived != query.archived
            {
                return Err(ProviderError::Incompatible);
            }
            cursor
        } else {
            ListCursor {
                instance: self.instance.clone(),
                cwd: query.cwd.clone(),
                archived: query.archived,
                loaded: !query.archived,
                backend: None,
                seen_loaded: vec![],
            }
        };
        let limit = query.limit.clamp(1, 100) as usize;
        let mut data = vec![];
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            if !seen.insert((cursor.loaded, cursor.backend.clone())) {
                return Err(ProviderError::Incompatible);
            }
            if cursor.loaded {
                // Newly active threads can precede rollout/state-DB indexing. Read the
                // already-loaded objects first, without subscribing or resuming them.
                let page = self
                    .client
                    .call(&p::ThreadLoadedListParams {
                        cursor: cursor.backend.clone(),
                        limit: Some((limit - data.len()) as u32),
                    })
                    .await?;
                for id in page.data {
                    let result = match self
                        .client
                        .call(&p::ThreadReadParams {
                            thread_id: id,
                            include_turns: Some(false),
                        })
                        .await
                    {
                        Ok(thread) => thread,
                        Err(Error::Rejected(_)) => continue, // closed during pagination
                        Err(error) => return Err(error.into()),
                    };
                    if !matches!(
                        result.thread.source,
                        p::SessionSource::Cli
                            | p::SessionSource::Vscode
                            | p::SessionSource::Exec
                            | p::SessionSource::AppServer
                            | p::SessionSource::Unknown
                    ) {
                        continue;
                    }
                    let item = summary(&self.instance, result.thread);
                    if query.cwd.as_deref().is_none_or(|cwd| cwd == item.cwd) {
                        cursor.seen_loaded.push(item.reference.thread.clone());
                        data.push(item);
                    }
                }
                cursor.backend = page.next_cursor;
                if cursor.backend.is_none() {
                    cursor.loaded = false;
                }
            } else {
                let page = self
                    .client
                    .call(&p::ThreadListParams {
                        cwd: query.cwd.clone().map(p::ThreadListCwdFilter::String),
                        cursor: cursor.backend.clone(),
                        limit: Some((limit - data.len()) as u32),
                        archived: Some(query.archived),
                        use_state_db_only: Some(true),
                        sort_key: Some(p::ThreadSortKey::UpdatedAt),
                        sort_direction: Some(p::SortDirection::Desc),
                        source_kinds: Some(vec![
                            p::ThreadSourceKind::Cli,
                            p::ThreadSourceKind::Vscode,
                            p::ThreadSourceKind::AppServer,
                            p::ThreadSourceKind::Exec,
                            p::ThreadSourceKind::Unknown,
                        ]),
                        ..Default::default()
                    })
                    .await?;
                for thread in page.data {
                    if !cursor.seen_loaded.contains(&thread.id) {
                        data.push(summary(&self.instance, thread));
                    }
                }
                cursor.backend = page.next_cursor;
                if cursor.backend.is_none() {
                    return Ok(Page {
                        data,
                        next_cursor: None,
                    });
                }
            }
            if data.len() >= limit {
                let encoded =
                    serde_json::to_string(&cursor).map_err(|_| ProviderError::Incompatible)?;
                return Ok(Page {
                    data,
                    next_cursor: Some(format!("pp1:{encoded}")),
                });
            }
        }
        Err(ProviderError::Incompatible)
    }

    async fn read(&self, thread: &str) -> Result<Session, ProviderError> {
        let result = self
            .client
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(false),
            })
            .await?;
        let legacy = matches!(result.thread.history_mode, p::ThreadHistoryMode::Legacy);
        let mut session = summary(&self.instance, result.thread);
        if matches!(session.status, RuntimeStatus::Active { .. }) && !legacy {
            let turns = self
                .client
                .call(&p::ThreadTurnsListParams {
                    thread_id: thread.into(),
                    limit: Some(10),
                    sort_direction: Some(p::SortDirection::Desc),
                    items_view: Some(p::TurnItemsView::NotLoaded),
                    cursor: None,
                })
                .await?;
            session.active_turn = turns
                .data
                .into_iter()
                .find(|t| matches!(t.status, p::TurnStatus::InProgress))
                .map(|t| t.id);
        }
        Ok(session)
    }

    async fn active_turn(&self, thread: &str) -> Result<Option<String>, ProviderError> {
        let metadata = self
            .client
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(false),
            })
            .await?;
        if !matches!(metadata.thread.status, p::ThreadStatus::Active(_)) {
            return Ok(None);
        }
        let turns = match metadata.thread.history_mode {
            p::ThreadHistoryMode::Legacy => {
                self.client
                    .call(&p::ThreadReadParams {
                        thread_id: thread.into(),
                        include_turns: Some(true),
                    })
                    .await?
                    .thread
                    .turns
            }
            p::ThreadHistoryMode::Paginated => {
                self.client
                    .call(&p::ThreadTurnsListParams {
                        thread_id: thread.into(),
                        cursor: None,
                        limit: Some(10),
                        sort_direction: Some(p::SortDirection::Desc),
                        items_view: Some(p::TurnItemsView::NotLoaded),
                    })
                    .await?
                    .data
            }
        };
        Ok(turns
            .into_iter()
            .find(|t| matches!(t.status, p::TurnStatus::InProgress))
            .map(|t| t.id))
    }

    async fn history(&self, thread: &str) -> Result<History, ProviderError> {
        let metadata = self
            .client
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(false),
            })
            .await?;
        if matches!(metadata.thread.history_mode, p::ThreadHistoryMode::Legacy) {
            let full = self
                .client
                .call(&p::ThreadReadParams {
                    thread_id: thread.into(),
                    include_turns: Some(true),
                })
                .await?;
            let mut items = Vec::new();
            let mut bytes = 0_usize;
            for turn in full.thread.turns {
                for item in turn_items(turn) {
                    bytes = bytes.saturating_add(item.text.len());
                    if bytes > 64 * 1024 * 1024 {
                        return Ok(History {
                            items,
                            all_pages_read: false,
                        });
                    }
                    items.push(item);
                }
            }
            return Ok(History {
                items: crate::errors::merge(&self.errors, thread, items),
                all_pages_read: true,
            });
        }
        let mut items = vec![];
        let mut cursor = None::<String>;
        let mut seen_cursors = std::collections::HashSet::new();
        let mut seen_items = std::collections::HashSet::new();
        let mut all_pages_read = false;
        let mut bytes = 0_usize;
        for _ in 0..1000 {
            let result = self
                .client
                .call(&p::ThreadTurnsListParams {
                    thread_id: thread.into(),
                    cursor,
                    limit: Some(100),
                    sort_direction: Some(p::SortDirection::Asc),
                    items_view: Some(p::TurnItemsView::Full),
                })
                .await?;
            for turn in result.data {
                if !matches!(turn.items_view, p::TurnItemsView::Full) {
                    return Err(ProviderError::Incompatible);
                }
                for item in turn_items(turn) {
                    if seen_items.insert((item.turn_id.clone(), item.item_id.clone())) {
                        bytes = bytes.saturating_add(item.text.len());
                        if bytes > 64 * 1024 * 1024 {
                            return Ok(History {
                                items,
                                all_pages_read: false,
                            });
                        }
                        items.push(item);
                    }
                }
            }
            cursor = result.next_cursor;
            match &cursor {
                None => {
                    all_pages_read = true;
                    break;
                }
                Some(value) if !seen_cursors.insert(value.clone()) => break,
                _ => {}
            }
        }
        Ok(History {
            items: crate::errors::merge(&self.errors, thread, items),
            all_pages_read,
        })
    }

    async fn execute(&self, mutation: Mutation) -> Result<Receipt, ProviderError> {
        if !self.control {
            return Err(ProviderError::Unsupported);
        }
        match mutation {
            Mutation::SetTitleIfMissing { thread, title } => {
                if title.trim().is_empty() {
                    return Err(ProviderError::Conflict);
                }
                let current = self
                    .client
                    .call(&p::ThreadReadParams {
                        thread_id: thread.clone(),
                        include_turns: Some(false),
                    })
                    .await?;
                if current
                    .thread
                    .name
                    .as_deref()
                    .is_none_or(|name| name.trim().is_empty())
                {
                    self.client
                        .write(&p::ThreadSetNameParams {
                            thread_id: thread.clone(),
                            name: title,
                        })
                        .await?;
                }
                Ok(Receipt { thread, turn: None })
            }
            Mutation::Create { cwd } => {
                let params: p::ThreadStartParams = p::ThreadStartParams::builder()
                    .cwd(Some(cwd))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread: result.thread.id,
                    turn: None,
                })
            }
            Mutation::Resume { thread } => {
                let params: p::ThreadResumeParams = p::ThreadResumeParams::builder()
                    .thread_id(thread.clone())
                    .exclude_turns(Some(true))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                if result.thread.id != thread {
                    return Err(ProviderError::Incompatible);
                }
                Ok(Receipt { thread, turn: None })
            }
            Mutation::Start {
                thread,
                text,
                client_id,
            } => {
                // Never convert a detected active turn into an unguarded turn/start.
                let current = self.read(&thread).await?;
                if current.status != RuntimeStatus::Idle {
                    return Err(ProviderError::Conflict);
                }
                let params: p::TurnStartParams = p::TurnStartParams::builder()
                    .thread_id(thread.clone())
                    .input(text_input(text)?)
                    .client_user_message_id(Some(client_id))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread,
                    turn: Some(result.turn.id),
                })
            }
            Mutation::Steer {
                thread,
                expected_turn,
                text,
                client_id,
            } => {
                let params = p::TurnSteerParams {
                    thread_id: thread.clone(),
                    expected_turn_id: expected_turn,
                    input: text_input(text)?,
                    client_user_message_id: Some(client_id),
                };
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread,
                    turn: Some(result.turn_id),
                })
            }
        }
    }
}

fn text_input(text: String) -> Result<Vec<p::UserInput>, ProviderError> {
    if text.trim().is_empty() || text.len() > 1024 * 1024 {
        return Err(ProviderError::Conflict);
    }
    Ok(vec![p::UserInput::TextUserInput {
        text,
        text_elements: vec![],
        type_: p::TextUserInputType::Text,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_turns_preserve_primary_errors_without_internal_details() {
        let turn: p::Turn = serde_json::from_str(r#"{"id":"t","status":"failed","itemsView":"full","items":[{"type":"userMessage","id":"u","content":[{"type":"text","text":"question"}]}],"error":{"message":"request failed","additionalDetails":"private transport diagnostics"}}"#).unwrap();
        let items = turn_items(turn);
        assert_eq!(items.len(), 2);
        assert!(items[1].is_error());
        assert_eq!(items[1].text, "request failed");
        assert!(
            !serde_json::to_string(&items)
                .unwrap()
                .contains("private transport")
        );
        for status in ["failed", "interrupted"] {
            let turn = serde_json::from_str(&format!(
                r#"{{"id":"t","status":"{status}","itemsView":"full","items":[]}}"#
            ))
            .unwrap();
            assert!(turn_items(turn)[0].is_error());
        }
    }

    #[test]
    fn user_images_keep_only_metadata_even_without_text() {
        let item = serde_json::from_str(r#"{"type":"userMessage","id":"u","content":[{"type":"localImage","path":"/private/attachments/picture.jpg"},{"type":"image","url":"data:image/png;base64,SECRET"}]}"#).unwrap();
        let item = text_item("t", true, item).unwrap();
        assert!(item.text.is_empty());
        assert_eq!(item.attachments[0].name, "picture.jpg");
        assert_eq!(item.attachments[1].name, "image");
        let encoded = serde_json::to_string(&item).unwrap();
        assert!(!encoded.contains("SECRET") && !encoded.contains("/private"));
    }

    #[derive(serde::Deserialize)]
    struct Frame {
        id: Option<p::RequestId>,
        method: String,
        params: Option<Box<serde_json::value::RawValue>>,
    }

    const THREAD: &str = r#"{"id":"thread","sessionId":"thread","cliVersion":"0.162.1","projectId":null,"preview":"Title","cwd":"/workspace","ephemeral":false,"source":"appServer","modelProvider":"openai","createdAt":100,"updatedAt":100,"status":{"type":"idle"},"turns":[]}"#;

    #[tokio::test]
    async fn user_input_preserves_execution_policy_and_drop_sends_no_lifecycle_action() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_util::codec::{Framed, LinesCodec};
        let (local, remote) = tokio::io::duplex(16384);
        let (read, write) = tokio::io::split(local);
        let provider = Codex::new(
            "instance".into(),
            Client::spawn(
                Box::new(crate::transport::JsonLines::new(read, write, None)),
                std::time::Duration::from_secs(2),
            ),
        )
        .with_control(true);
        let server = tokio::spawn(async move {
            let mut server = Framed::new(remote, LinesCodec::new());
            let read: Frame = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(read.method, "thread/read");
            let params: p::ThreadReadParams =
                serde_json::from_str(read.params.unwrap().get()).unwrap();
            assert_eq!(params.include_turns, Some(false));
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"thread\":{THREAD}}}}}",
                    read.id.unwrap()
                ))
                .await
                .unwrap();
            let start: Frame =
                serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(start.method, "turn/start");
            let params: p::TurnStartParams =
                serde_json::from_str(start.params.unwrap().get()).unwrap();
            assert_eq!(params.thread_id, "thread");
            assert_eq!(params.client_user_message_id.as_deref(), Some("message-1"));
            assert!(
                params.model.is_none()
                    && params.cwd.is_none()
                    && params.approval_policy.is_none()
                    && params.approvals_reviewer.is_none()
                    && params.sandbox_policy.is_none()
            );
            server.send(format!("{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"active-turn\",\"status\":\"inProgress\",\"items\":[]}}}}}}",start.id.unwrap())).await.unwrap();
            let steer: Frame =
                serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(steer.method, "turn/steer");
            let params: p::TurnSteerParams =
                serde_json::from_str(steer.params.unwrap().get()).unwrap();
            assert_eq!(params.expected_turn_id, "active-turn");
            assert_eq!(params.client_user_message_id.as_deref(), Some("message-2"));
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"turnId\":\"active-turn\"}}}}",
                    steer.id.unwrap()
                ))
                .await
                .unwrap();
            assert!(
                server.next().await.is_none(),
                "disconnect emitted an unexpected lifecycle request"
            );
        });
        let first = provider
            .execute(Mutation::Start {
                thread: "thread".into(),
                text: "first".into(),
                client_id: "message-1".into(),
            })
            .await
            .unwrap();
        assert_eq!(first.turn.as_deref(), Some("active-turn"));
        provider
            .execute(Mutation::Steer {
                thread: "thread".into(),
                expected_turn: "active-turn".into(),
                text: "second".into(),
                client_id: "message-2".into(),
            })
            .await
            .unwrap();
        drop(provider);
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn unverified_provider_refuses_mutation_before_sending_any_request() {
        use futures_util::StreamExt;
        use tokio_util::codec::{Framed, LinesCodec};
        let (local, remote) = tokio::io::duplex(1024);
        let (read, write) = tokio::io::split(local);
        let provider = Codex::new(
            "instance".into(),
            Client::spawn(
                Box::new(crate::transport::JsonLines::new(read, write, None)),
                std::time::Duration::from_secs(1),
            ),
        );
        assert!(matches!(
            provider
                .execute(Mutation::Create {
                    cwd: "/workspace".into()
                })
                .await,
            Err(ProviderError::Unsupported)
        ));
        drop(provider);
        assert!(
            Framed::new(remote, LinesCodec::new())
                .next()
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn missing_title_is_written_without_starting_a_turn_and_existing_titles_are_preserved() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_util::codec::{Framed, LinesCodec};
        for existing in [None, Some("Native title")] {
            let (local, remote) = tokio::io::duplex(16384);
            let (read, write) = tokio::io::split(local);
            let provider = Codex::new(
                "instance".into(),
                Client::spawn(
                    Box::new(crate::transport::JsonLines::new(read, write, None)),
                    std::time::Duration::from_secs(2),
                ),
            )
            .with_control(true);
            let server = tokio::spawn(async move {
                let mut stream = Framed::new(remote, LinesCodec::new());
                let frame: Frame =
                    serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                assert_eq!(frame.method, "thread/read");
                let mut thread: p::Thread = serde_json::from_str(THREAD).unwrap();
                thread.name = existing.map(str::to_owned);
                let result = p::ThreadReadResponse { thread };
                stream
                    .send(format!(
                        "{{\"id\":{},\"result\":{}}}",
                        frame.id.unwrap(),
                        serde_json::to_string(&result).unwrap()
                    ))
                    .await
                    .unwrap();
                if existing.is_none() {
                    let frame: Frame =
                        serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                    assert_eq!(frame.method, "thread/name/set");
                    let params: p::ThreadSetNameParams =
                        serde_json::from_str(frame.params.unwrap().get()).unwrap();
                    assert_eq!(params.name, "Bridge title");
                    assert_eq!(params.thread_id, "thread");
                    stream
                        .send(format!("{{\"id\":{},\"result\":{{}}}}", frame.id.unwrap()))
                        .await
                        .unwrap();
                }
                assert!(
                    stream.next().await.is_none(),
                    "metadata must not start, resume or stop a turn"
                );
            });
            let receipt = provider
                .execute(Mutation::SetTitleIfMissing {
                    thread: "thread".into(),
                    title: "Bridge title".into(),
                })
                .await
                .unwrap();
            assert!(receipt.turn.is_none());
            drop(provider);
            server.await.unwrap();
        }
    }

    #[test]
    fn schema_decodes_active_flags_and_rejects_unknown_state() {
        let status: p::ThreadStatus =
            serde_json::from_str(r#"{"type":"active","activeFlags":["waitingOnApproval"]}"#)
                .unwrap();
        assert_eq!(runtime(&status).label(), "🟠 等待审批");
        assert!(serde_json::from_str::<p::ThreadStatus>(r#"{"type":"future-state"}"#).is_err());
    }

    #[test]
    fn typed_tool_outputs_are_excluded_from_chat_history() {
        let item: p::ThreadItem =
            serde_json::from_str(r#"{"type":"reasoning","id":"i","summary":["private"]}"#).unwrap();
        assert!(text_item("turn", true, item).is_none());
    }

    #[test]
    fn failed_or_active_turn_text_is_not_mislabeled_final() {
        let partial: p::ThreadItem =
            serde_json::from_str(r#"{"type":"agentMessage","id":"i","text":"partial"}"#).unwrap();
        assert!(!text_item("turn", false, partial).unwrap().complete);
        let final_item: p::ThreadItem = serde_json::from_str(
            r#"{"type":"agentMessage","id":"i","text":"answer","phase":"final_answer"}"#,
        )
        .unwrap();
        assert!(text_item("turn", false, final_item).unwrap().complete);
    }

    #[test]
    fn failed_turn_error_codes_accept_official_string_and_structured_shapes() {
        let string_error = r#"{"message":"fixture failed","codexErrorInfo":"other"}"#;
        let structured_error = r#"{"message":"fixture failed","codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":502}}}"#;
        assert!(
            serde_json::from_str::<p::TurnError>(string_error).is_ok(),
            "official string error shape must not prevent reading history"
        );
        assert!(
            serde_json::from_str::<p::TurnError>(structured_error).is_ok(),
            "official structured error shape must not prevent reading history"
        );
        assert!(
            serde_json::from_str::<p::TurnError>(
                r#"{"message":"fixture","codexErrorInfo":"futureError"}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<p::TurnError>(
                r#"{"message":"fixture","codexErrorInfo":{"futureError":{"code":42}}}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<p::TurnError>(r#"{"message":"fixture","codexErrorInfo":123}"#)
                .is_err()
        );
    }
}
