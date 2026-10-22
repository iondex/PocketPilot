use super::*;

enum Listing {
    Paths,
    RepeatingEmptyPage,
}

#[async_trait::async_trait]
impl AgentProvider for Listing {
    fn capabilities(&self) -> session::Capabilities {
        session::Capabilities::default()
    }

    async fn list(
        &self,
        q: session::SessionQuery,
    ) -> Result<session::Page<session::Session>, session::ProviderError> {
        if matches!(self, Self::RepeatingEmptyPage) {
            // Let the timeout cancel this fixture if the pagination guard regresses.
            tokio::task::yield_now().await;
            return Ok(session::Page {
                data: Vec::new(),
                next_cursor: Some("same-position".into()),
            });
        }
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
    let first = list_workspace(&Listing::Paths, &ws, None, 1).await.unwrap();
    assert_eq!(first.data[0].cwd, "/base");
    let second = list_workspace(&Listing::Paths, &ws, first.next_cursor.clone(), 1)
        .await
        .unwrap();
    assert_eq!(second.data[0].cwd, "/worktree");
    assert!(second.next_cursor.is_none());
    ws.additional_cwds = vec!["/changed".into()];
    assert!(
        list_workspace(&Listing::Paths, &ws, first.next_cursor, 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cursor_cannot_cross_projects_or_unknown_versions() {
    let mut workspace = Workspace {
        id: "project-a".into(),
        provider_id: "fixture".into(),
        name: "project".into(),
        short_name: "project".into(),
        cwd: "/base".into(),
        additional_cwds: vec!["/worktree".into()],
        updated_at: None,
    };
    let first = list_workspace(&Listing::Paths, &workspace, None, 1)
        .await
        .unwrap();
    let cursor = first.next_cursor.unwrap();
    workspace.id = "project-b".into();
    assert!(matches!(
        list_workspace(&Listing::Paths, &workspace, Some(cursor.clone()), 1).await,
        Err(session::ProviderError::Incompatible)
    ));
    workspace.id = "project-a".into();
    let unknown_version = cursor.replacen("ws1:", "ws2:", 1);
    assert!(matches!(
        list_workspace(&Listing::Paths, &workspace, Some(unknown_version), 1).await,
        Err(session::ProviderError::Incompatible)
    ));
}

#[tokio::test]
async fn repeating_empty_backend_page_terminates_instead_of_spinning() {
    let workspace = Workspace {
        id: "project".into(),
        provider_id: "fixture".into(),
        name: "project".into(),
        short_name: "project".into(),
        cwd: "/base".into(),
        additional_cwds: Vec::new(),
        updated_at: None,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        list_workspace(&Listing::RepeatingEmptyPage, &workspace, None, 1),
    )
    .await
    .expect("repeated cursor must not keep a listing alive");
    assert!(matches!(result, Err(session::ProviderError::Incompatible)));
    assert!(require_workspace("/base", &workspace).is_ok());
    assert!(require_workspace("/base/child", &workspace).is_err());
    assert!(require_workspace("/base-sibling", &workspace).is_err());
}
