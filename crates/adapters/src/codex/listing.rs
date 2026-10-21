//! Read loaded sessions before stored sessions, preserving scoped pagination.
use super::{Codex, mapping::summary};
use crate::codex::{Error, protocol as p};
use futures_util::{StreamExt, stream};
use session::{Page, ProviderError, Session, SessionQuery};
use std::time::Duration;
use tokio::time::Instant;

#[derive(serde::Serialize, serde::Deserialize)]
struct ListCursor {
    instance: String,
    cwd: Option<String>,
    archived: bool,
    loaded: bool,
    backend: Option<String>,
    seen_loaded: Vec<String>,
}

impl Codex {
    pub(super) async fn list_sessions(
        &self,
        query: SessionQuery,
    ) -> Result<Page<Session>, ProviderError> {
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
        // Loaded objects can be blocked by work in another project. Discovery is
        // best-effort enrichment; it must not prevent reading the scoped index.
        let loaded_deadline = Instant::now() + Duration::from_secs(3);
        let mut data = vec![];
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            if !seen.insert((cursor.loaded, cursor.backend.clone())) {
                return Err(ProviderError::Incompatible);
            }
            if cursor.loaded {
                // Newly active threads can precede rollout/state-DB indexing. Read the
                // already-loaded objects first, without subscribing or resuming them.
                let page = match tokio::time::timeout_at(
                    loaded_deadline,
                    self.client.call(&p::ThreadLoadedListParams {
                        cursor: cursor.backend.clone(),
                        limit: Some((limit - data.len()) as u32),
                    }),
                )
                .await
                {
                    Ok(Ok(page)) => page,
                    Ok(Err(Error::Timeout)) | Err(_) => {
                        cursor.loaded = false;
                        cursor.backend = None;
                        continue;
                    }
                    Ok(Err(error)) => return Err(error.into()),
                };
                let reads = stream::iter(page.data.into_iter().enumerate())
                    .map(|(index, id)| async move {
                        let deadline = loaded_deadline.min(Instant::now() + Duration::from_secs(1));
                        let result = tokio::time::timeout_at(
                            deadline,
                            self.client.call(&p::ThreadReadParams {
                                thread_id: id,
                                include_turns: Some(false),
                            }),
                        )
                        .await;
                        (index, result)
                    })
                    .buffer_unordered(8)
                    .collect::<Vec<_>>()
                    .await;
                let mut reads = reads;
                // RPCs may finish out of order; loaded pagination must remain stable.
                reads.sort_by_key(|(index, _)| *index);
                for (_, result) in reads {
                    let result = match result {
                        Ok(Ok(thread)) => thread,
                        Ok(Err(Error::Rejected(_) | Error::Timeout)) | Err(_) => continue,
                        Ok(Err(error)) => return Err(error.into()),
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
}
