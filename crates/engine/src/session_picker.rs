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
#[path = "session_picker_tests.rs"]
mod tests;
