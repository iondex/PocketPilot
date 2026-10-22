//! Project-scoped listing and opaque cursors, independent of connection setup.
use super::Error;
use crate::config::Workspace;
use session::{AgentProvider, ProviderError};

/// Require an exact configured cwd, including explicitly registered worktrees.
///
/// This is stricter than allowed-root discovery: a parent or sibling project is
/// not the selected project, even when both are under the same permitted root.
///
/// # Errors
///
/// Returns [`Error::Selection`] when the native session cwd is outside this project.
pub fn require_workspace(cwd: &str, workspace: &Workspace) -> Result<(), Error> {
    if cwd == workspace.cwd || workspace.additional_cwds.iter().any(|p| p == cwd) {
        Ok(())
    } else {
        Err(Error::Selection)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkspaceCursor {
    workspace: String,
    paths: Vec<String>,
    index: usize,
    backend: Option<String>,
}

impl WorkspaceCursor {
    // The path set travels with the cursor. Reusing a cursor after authorization
    // changes must fail rather than continue into a previously selected cwd.
    fn decode(workspace: &Workspace, value: Option<String>) -> Result<Self, ProviderError> {
        let mut paths = vec![workspace.cwd.clone()];
        for cwd in &workspace.additional_cwds {
            if !paths.contains(cwd) {
                paths.push(cwd.clone());
            }
        }
        let cursor = match value {
            Some(value) => serde_json::from_str::<Self>(
                value
                    .strip_prefix("ws1:")
                    .ok_or(ProviderError::Incompatible)?,
            )
            .map_err(|_| ProviderError::Incompatible)?,
            None => Self {
                workspace: workspace.id.clone(),
                paths: paths.clone(),
                index: 0,
                backend: None,
            },
        };
        if cursor.workspace != workspace.id || cursor.paths != paths || cursor.index >= paths.len()
        {
            return Err(ProviderError::Incompatible);
        }
        Ok(cursor)
    }

    fn encode(&self) -> Result<String, ProviderError> {
        // Keep the existing version tag: menus may still hold cursors from before a restart.
        Ok(format!(
            "ws1:{}",
            serde_json::to_string(self).map_err(|_| ProviderError::Incompatible)?
        ))
    }
}

/// Page through the explicitly configured cwd and worktrees without broadening
/// the provider query to unrelated projects.
///
/// # Errors
///
/// Rejects cursors from another project/path set, malformed tokens and providers
/// that repeat a pagination position. Native provider errors are preserved.
pub async fn list_workspace(
    provider: &dyn AgentProvider,
    workspace: &Workspace,
    cursor: Option<String>,
    limit: u32,
) -> Result<session::Page<session::Session>, session::ProviderError> {
    let mut cursor = WorkspaceCursor::decode(workspace, cursor)?;
    let limit = limit.clamp(1, 100) as usize;
    let mut data = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while data.len() < limit && cursor.index < cursor.paths.len() {
        // A provider can return an empty page with a cursor. Track positions so
        // a repeated empty page cannot keep this read-only request alive forever.
        if !seen.insert((cursor.index, cursor.backend.clone())) {
            return Err(ProviderError::Incompatible);
        }
        let page = provider
            .list(session::SessionQuery {
                cwd: Some(cursor.paths[cursor.index].clone()),
                cursor: cursor.backend.clone(),
                limit: (limit - data.len()) as u32,
                archived: false,
            })
            .await?;
        data.extend(page.data);
        cursor.backend = page.next_cursor;
        if cursor.backend.is_none() {
            cursor.index += 1;
        }
    }
    let next_cursor = if cursor.index < cursor.paths.len() {
        Some(cursor.encode()?)
    } else {
        None
    };
    Ok(session::Page { data, next_cursor })
}

/// Native menus need the resolved page, including snapshot expiry/reset behavior.
pub struct PickerPage {
    pub data: Vec<session::Session>,
    pub next_cursor: Option<String>,
    pub offset: usize,
    pub total: usize,
}

/// Hide only dedicated probes proven by this bridge's durable create receipts.
/// Never infer a test session from its title or mutate/archive it remotely.
pub async fn list_picker(
    store: &crate::storage::Store,
    provider: &dyn AgentProvider,
    workspace: &Workspace,
    cursor: Option<String>,
    limit: u32,
) -> Result<PickerPage, session::ProviderError> {
    super::session_picker::list(store, provider, workspace, cursor, limit).await
}
