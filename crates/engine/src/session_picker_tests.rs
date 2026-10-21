use super::*;
use session::Page;
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

#[tokio::test]
async fn a_cursor_at_the_end_of_a_snapshot_restarts_instead_of_showing_an_empty_page() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::storage::Store::open(dir.path()).await.unwrap();
    let workspace = Workspace {
        id: "ws".into(),
        provider_id: "instance".into(),
        name: "Example".into(),
        short_name: "example".into(),
        cwd: "/base".into(),
        additional_cwds: vec![],
        updated_at: None,
    };
    let agent = Agent(Mutex::new(vec![
        entry("first", "/base", 2),
        entry("second", "/base", 1),
    ]));
    let first = list(&store, &agent, &workspace, None, 1).await.unwrap();
    let encoded = first.next_cursor.unwrap();
    let mut cursor: Cursor = serde_json::from_str(encoded.strip_prefix("sp2:").unwrap()).unwrap();
    cursor.offset = 2;
    let encoded = format!("sp2:{}", serde_json::to_string(&cursor).unwrap());
    let page = list(&store, &agent, &workspace, Some(encoded), 25)
        .await
        .unwrap();
    assert_eq!(
        page.data.len(),
        2,
        "an out-of-range page must restart with visible sessions"
    );
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn expired_snapshots_report_page_one_with_the_new_total() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::storage::Store::open(dir.path()).await.unwrap();
    let workspace = Workspace {
        id: "ws".into(),
        provider_id: "instance".into(),
        name: "Example".into(),
        short_name: "example".into(),
        cwd: "/base".into(),
        additional_cwds: vec![],
        updated_at: None,
    };
    let agent = Agent(Mutex::new(
        (0..26)
            .map(|index| entry(&index.to_string(), "/base", index))
            .collect(),
    ));
    let first = list(&store, &agent, &workspace, None, 25).await.unwrap();
    assert_eq!((first.offset, first.total, first.data.len()), (0, 26, 25));
    let cursor = first.next_cursor.unwrap();
    sqlx::query("UPDATE session_list_snapshots SET expires_at=0")
        .execute(store.pool())
        .await
        .unwrap();
    agent.0.lock().unwrap().truncate(3);
    let page = list(&store, &agent, &workspace, Some(cursor), 25)
        .await
        .unwrap();
    assert_eq!((page.offset, page.total, page.data.len()), (0, 3, 3));
    assert!(page.next_cursor.is_none());
}
