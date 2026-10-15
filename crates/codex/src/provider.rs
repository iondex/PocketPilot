use crate::{Client, Error, protocol as p};
use session::{
    AgentProvider, Capabilities, History, Page, ProviderError, Role, RuntimeStatus, Session,
    SessionQuery, SessionRef, TextItem,
};

pub struct Codex {
    instance: String,
    client: Client,
}
impl Codex {
    pub fn new(instance: String, client: Client) -> Self {
        Self { instance, client }
    }
    pub fn client(&self) -> &Client {
        &self.client
    }
}

impl From<Error> for ProviderError {
    fn from(error: Error) -> Self {
        match error {
            Error::Timeout => Self::Timeout,
            Error::Rejected(code) => Self::Rejected(code),
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
        title: thread.name.filter(|s| !s.is_empty()).unwrap_or_else(|| {
            if thread.preview.is_empty() {
                "未命名会话".into()
            } else {
                thread.preview
            }
        }),
        cwd: thread.cwd.to_string(),
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
        } => TextItem {
            turn_id: turn_id.into(),
            item_id: id,
            role: Role::User,
            phase: None,
            text: content
                .into_iter()
                .filter_map(|input| match input {
                    p::UserInput::TextUserInput { text, .. } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            complete: true,
            client_id,
        },
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
        },
        _ => return None,
    };
    (!item.text.is_empty()).then_some(item)
}

#[async_trait::async_trait]
impl AgentProvider for Codex {
    fn capabilities(&self) -> Capabilities {
        // A schema match alone does not certify live observation or lifecycle safety.
        Capabilities {
            list: true,
            history: true,
            ..Default::default()
        }
    }
    async fn list(&self, query: SessionQuery) -> Result<Page<Session>, ProviderError> {
        let result = self
            .client
            .call(&p::ThreadListParams {
                cwd: query.cwd.map(p::ThreadListCwdFilter::String),
                cursor: query.cursor,
                limit: Some(query.limit.clamp(1, 100)),
                archived: Some(query.archived),
                use_state_db_only: Some(true),
                sort_key: Some(p::ThreadSortKey::UpdatedAt),
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
        Ok(Page {
            data: result
                .data
                .into_iter()
                .map(|t| summary(&self.instance, t))
                .collect(),
            next_cursor: result.next_cursor,
        })
    }
    async fn read(&self, thread: &str) -> Result<Session, ProviderError> {
        let result = self
            .client
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(false),
            })
            .await?;
        Ok(summary(&self.instance, result.thread))
    }
    async fn history(&self, thread: &str) -> Result<History, ProviderError> {
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
                let complete = matches!(turn.status, p::TurnStatus::Completed);
                for value in turn.items {
                    if let Some(item) = text_item(&turn.id, complete, value)
                        && seen_items.insert((item.turn_id.clone(), item.item_id.clone()))
                    {
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
            items,
            all_pages_read,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_decodes_active_flags_and_rejects_unknown_state() {
        let status: p::ThreadStatus =
            serde_json::from_str(r#"{"type":"active","activeFlags":["waitingOnApproval"]}"#)
                .unwrap();
        assert_eq!(runtime(&status).label(), "等待审批");
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
}
