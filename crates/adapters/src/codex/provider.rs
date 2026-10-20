//! Codex provider capabilities and read-only session observation.
use crate::codex::{Client, Error, protocol as p};
use mapping::{summary, turn_items};
use session::provider::{Mutation, Receipt};
use session::{
    AgentProvider, Capabilities, History, Page, ProviderError, RuntimeStatus, Session, SessionQuery,
};

#[path = "mapping.rs"]
mod mapping;

#[path = "listing.rs"]
mod listing;

#[path = "mutations.rs"]
mod mutations;

#[path = "retry.rs"]
mod retry;

pub use mapping::runtime;

#[derive(Clone)]
pub struct Codex {
    instance: String,
    client: Client,
    control: bool,
    errors: crate::codex::errors::Errors,
    changes: tokio::sync::watch::Receiver<()>,
}

impl Codex {
    pub fn new(instance: String, client: Client) -> Self {
        let (sender, changes) = tokio::sync::watch::channel(());
        let errors = crate::codex::errors::listen(&client, sender);
        Self {
            instance,
            client,
            control: false,
            errors,
            changes,
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

#[async_trait::async_trait]
impl AgentProvider for Codex {
    fn session_changes(&self) -> Option<tokio::sync::watch::Receiver<()>> {
        Some(self.changes.clone())
    }

    async fn retryable_failure(
        &self,
        thread: &str,
    ) -> Result<Option<session::retry::RetryableFailure>, ProviderError> {
        self.latest_retryable_failure(thread).await
    }

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
        self.list_sessions(query).await
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
                items: crate::codex::errors::merge(&self.errors, thread, items),
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
            items: crate::codex::errors::merge(&self.errors, thread, items),
            all_pages_read,
        })
    }

    async fn execute(&self, mutation: Mutation) -> Result<Receipt, ProviderError> {
        self.execute_mutation(mutation).await
    }
}
