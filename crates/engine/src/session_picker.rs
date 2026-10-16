//! Sort the whole workspace session collection before paginating a stable snapshot.
use super::*;
use session::{Page, ProviderError, Session};
use sqlx::Row;

#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    id: String,
    offset: usize,
}

pub(super) async fn list(
    store: &crate::storage::Store,
    provider: &dyn AgentProvider,
    workspace: &Workspace,
    cursor: Option<String>,
    limit: u32,
) -> Result<Page<Session>, ProviderError> {
    let size = limit.clamp(1, 100) as usize;
    let paths = serde_json::to_string(&(workspace.cwd.clone(), workspace.additional_cwds.clone()))
        .map_err(|_| ProviderError::Incompatible)?;
    if let Some(encoded) = cursor.as_deref().and_then(|s| s.strip_prefix("sp2:")) {
        let cursor: Cursor = serde_json::from_str(encoded).map_err(|_| ProviderError::Conflict)?;
        let row=sqlx::query("SELECT payload FROM session_list_snapshots WHERE id=? AND workspace_id=? AND instance_id=? AND paths=? AND expires_at>?")
            .bind(&cursor.id).bind(&workspace.id).bind(&workspace.provider_id).bind(&paths).bind(crate::now())
            .fetch_optional(store.pool()).await.map_err(|_|ProviderError::Disconnected)?;
        if let Some(row) = row {
            let data: Vec<Session> = serde_json::from_str(row.get("payload"))
                .map_err(|_| ProviderError::Incompatible)?;
            if cursor.offset <= data.len() {
                return page(data, cursor.id, cursor.offset, size);
            }
        }
        // Expired read-only pages restart at a fresh collection rather than a protocol error.
    }
    let hidden = store
        .probe_threads(&workspace.provider_id)
        .await
        .map_err(|_| ProviderError::Disconnected)?;
    let mut data = Vec::new();
    let mut cursor = None;
    let mut cursors = std::collections::HashSet::new();
    let mut identities = std::collections::HashSet::new();
    let mut bytes = 0usize;
    let mut complete = false;
    for _ in 0..1000 {
        if !cursors.insert(cursor.clone()) {
            return Err(ProviderError::Incompatible);
        }
        let batch = list_workspace(provider, workspace, cursor, 100).await?;
        for item in batch.data {
            if item.reference.instance != workspace.provider_id
                || require_workspace(&item.cwd, workspace).is_err()
                || hidden.contains(&item.reference.thread)
                || !identities.insert(item.reference.thread.clone())
            {
                continue;
            }
            bytes = bytes.saturating_add(
                item.title.len() + item.cwd.len() + item.reference.thread.len() + 256,
            );
            if bytes > 16 * 1024 * 1024 {
                return Err(ProviderError::Unsupported);
            }
            data.push(item);
        }
        cursor = batch.next_cursor;
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(ProviderError::Timeout);
    }
    data.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.reference.thread.cmp(&a.reference.thread))
    });
    let id = uuid::Uuid::new_v4().to_string();
    if data.len() > size {
        let payload = serde_json::to_string(&data).map_err(|_| ProviderError::Incompatible)?;
        sqlx::query("INSERT INTO session_list_snapshots(id,workspace_id,instance_id,paths,payload,expires_at) VALUES(?,?,?,?,?,?)")
            .bind(&id).bind(&workspace.id).bind(&workspace.provider_id).bind(paths).bind(payload).bind(crate::now()+3600)
            .execute(store.pool()).await.map_err(|_|ProviderError::Disconnected)?;
        sqlx::query("DELETE FROM session_list_snapshots WHERE expires_at<=? OR id IN (SELECT id FROM session_list_snapshots ORDER BY rowid DESC LIMIT -1 OFFSET 64)")
            .bind(crate::now()).execute(store.pool()).await.map_err(|_|ProviderError::Disconnected)?;
    }
    page(data, id, 0, size)
}

fn page(
    data: Vec<Session>,
    id: String,
    offset: usize,
    size: usize,
) -> Result<Page<Session>, ProviderError> {
    let next = offset.saturating_add(size);
    let cursor = if next < data.len() {
        Some(format!(
            "sp2:{}",
            serde_json::to_string(&Cursor { id, offset: next })
                .map_err(|_| ProviderError::Incompatible)?
        ))
    } else {
        None
    };
    Ok(Page {
        data: data.into_iter().skip(offset).take(size).collect(),
        next_cursor: cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Agent(Mutex<Vec<Session>>);

    #[async_trait::async_trait]
    impl AgentProvider for Agent {
        fn capabilities(&self) -> session::Capabilities {
            session::Capabilities::default()
        }

        async fn list(&self, q: session::SessionQuery) -> Result<Page<Session>, ProviderError> {
            let offset: usize = q.cursor.as_deref().unwrap_or("0").parse().unwrap();
            let items = self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|i| Some(&i.cwd) == q.cwd.as_ref())
                .cloned()
                .collect::<Vec<_>>();
            Ok(Page {
                data: items.iter().skip(offset).take(1).cloned().collect(),
                next_cursor: (offset + 1 < items.len()).then(|| (offset + 1).to_string()),
            })
        }

        async fn read(&self, _: &str) -> Result<Session, ProviderError> {
            panic!("picker must not read/resume sessions")
        }

        async fn history(&self, _: &str) -> Result<session::History, ProviderError> {
            panic!("picker must not load history")
        }
    }

    fn entry(id: &str, cwd: &str, updated: i64) -> Session {
        Session {
            reference: session::SessionRef {
                instance: "instance".into(),
                thread: id.into(),
            },
            title: id.into(),
            native_title: None,
            cwd: cwd.into(),
            status: session::RuntimeStatus::Idle,
            active_turn: None,
            updated_at: Some(updated),
        }
    }

    #[tokio::test]
    async fn sorts_all_paths_and_backend_pages_then_preserves_the_snapshot_during_new_activity() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::storage::Store::open(dir.path()).await.unwrap();
        let workspace = Workspace {
            id: "ws".into(),
            provider_id: "instance".into(),
            name: "ws".into(),
            short_name: "ws".into(),
            cwd: "/base".into(),
            additional_cwds: vec!["/worktree".into()],
            updated_at: None,
        };
        let agent = Agent(Mutex::new(vec![
            entry("old-loaded", "/base", 100),
            entry("recent-stored", "/base", 300),
            entry("latest-worktree", "/worktree", 500),
        ]));
        let first = list(&store, &agent, &workspace, None, 1).await.unwrap();
        assert_eq!(first.data[0].reference.thread, "latest-worktree");
        agent
            .0
            .lock()
            .unwrap()
            .insert(0, entry("newly-active", "/base", 1000));
        let second = list(&store, &agent, &workspace, first.next_cursor, 1)
            .await
            .unwrap();
        assert_eq!(second.data[0].reference.thread, "recent-stored");
        let third = list(&store, &agent, &workspace, second.next_cursor, 1)
            .await
            .unwrap();
        assert_eq!(third.data[0].reference.thread, "old-loaded");
        assert!(third.next_cursor.is_none());
        assert_eq!(
            list(&store, &agent, &workspace, None, 1)
                .await
                .unwrap()
                .data[0]
                .reference
                .thread,
            "newly-active"
        );
    }
}
