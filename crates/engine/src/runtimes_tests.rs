use super::*;

struct Paths;

#[async_trait::async_trait]
impl AgentProvider for Paths {
    fn capabilities(&self) -> session::Capabilities {
        session::Capabilities::default()
    }

    async fn list(
        &self,
        q: session::SessionQuery,
    ) -> Result<session::Page<session::Session>, session::ProviderError> {
        let cwd = q.cwd.unwrap();
        Ok(session::Page {
            data: vec![session::Session {
                reference: session::SessionRef {
                    instance: "fixture".into(),
                    thread: cwd.clone(),
                },
                title: cwd.clone(),
                native_title: Some(cwd.clone()),
                cwd,
                status: session::RuntimeStatus::Idle,
                active_turn: None,
                updated_at: None,
            }],
            next_cursor: None,
        })
    }

    async fn read(&self, _: &str) -> Result<session::Session, session::ProviderError> {
        Err(session::ProviderError::Unsupported)
    }

    async fn history(&self, _: &str) -> Result<session::History, session::ProviderError> {
        Err(session::ProviderError::Unsupported)
    }
}

#[tokio::test]
async fn explicit_worktrees_are_paged_and_old_path_set_cursors_are_rejected() {
    let mut ws = Workspace {
        id: "ws".into(),
        provider_id: "fixture".into(),
        name: "workspace".into(),
        short_name: "ws".into(),
        cwd: "/base".into(),
        additional_cwds: vec!["/worktree".into(), "/worktree".into()],
        updated_at: None,
    };
    let first = list_workspace(&Paths, &ws, None, 1).await.unwrap();
    assert_eq!(first.data[0].cwd, "/base");
    let second = list_workspace(&Paths, &ws, first.next_cursor.clone(), 1)
        .await
        .unwrap();
    assert_eq!(second.data[0].cwd, "/worktree");
    assert!(second.next_cursor.is_none());
    ws.additional_cwds = vec!["/changed".into()];
    assert!(
        list_workspace(&Paths, &ws, first.next_cursor, 1)
            .await
            .is_err()
    );
}
