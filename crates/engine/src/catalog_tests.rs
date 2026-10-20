use super::*;

fn configuration(path: &Path, text: &str) -> Config {
    let mut config: Config = toml::from_str(text).unwrap();
    config.prepare(path, text).unwrap();
    config
}

const ROOT: &str = "[[hosts]]\nid='host'\nname='fixture'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n";

struct PagedWorkspaceAgent {
    instance: String,
    queries: std::sync::Mutex<Vec<(bool, Option<String>)>>,
}

#[async_trait::async_trait]
impl session::AgentProvider for PagedWorkspaceAgent {
    fn capabilities(&self) -> session::Capabilities {
        session::Capabilities {
            list: true,
            ..Default::default()
        }
    }

    async fn list(
        &self,
        query: session::SessionQuery,
    ) -> Result<session::Page<session::Session>, session::ProviderError> {
        assert!(query.cwd.is_none());
        self.queries
            .lock()
            .unwrap()
            .push((query.archived, query.cursor.clone()));
        let (paths, next) = match (query.archived, query.cursor.as_deref()) {
            (false, None) => (
                vec![
                    "/projects/existing",
                    "/projects/shared-worktree",
                    "/outside/forbidden",
                ],
                Some("second".into()),
            ),
            (false, Some("second")) => (
                vec![
                    "/projects/paged-project",
                    "/projects/paged-project",
                    "/projects/../outside",
                ],
                None,
            ),
            (true, None) => (vec!["/projects/archived-only"], None),
            _ => panic!("unexpected paging request"),
        };
        Ok(session::Page {
            data: paths
                .into_iter()
                .enumerate()
                .map(|(i, cwd)| session::Session {
                    reference: session::SessionRef {
                        instance: self.instance.clone(),
                        thread: format!("thread-{i}"),
                    },
                    title: "fixture".into(),
                    native_title: None,
                    cwd: cwd.into(),
                    status: session::RuntimeStatus::NotLoaded,
                    active_turn: None,
                    updated_at: None,
                })
                .collect(),
            next_cursor: next,
        })
    }

    async fn read(&self, _: &str) -> Result<session::Session, session::ProviderError> {
        panic!("workspace discovery must only list metadata")
    }

    async fn history(&self, _: &str) -> Result<session::History, session::ProviderError> {
        panic!("workspace discovery must not load history")
    }

    async fn execute(
        &self,
        _: session::provider::Mutation,
    ) -> Result<session::provider::Receipt, session::ProviderError> {
        panic!("workspace discovery must not mutate a session")
    }
}

#[tokio::test]
async fn discovers_projects_beyond_first_session_page_and_preserves_explicit_removal() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    let provider = catalog
        .discover(
            "host",
            adapters::ServerEndpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/control.sock".into(),
            },
        )
        .await
        .unwrap();
    let mut config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    let mut existing = catalog
        .add_workspace(
            &config,
            &provider.id,
            "/projects/existing",
            Some("Custom project name"),
        )
        .await
        .unwrap();
    existing
        .additional_cwds
        .push("/projects/shared-worktree".into());
    sqlx::query("UPDATE catalog_workspaces SET payload=? WHERE id=?")
        .bind(serde_json::to_string(&existing).unwrap())
        .bind(&existing.id)
        .execute(&catalog.pool)
        .await
        .unwrap();
    let agent = PagedWorkspaceAgent {
        instance: provider.id.clone(),
        queries: std::sync::Mutex::new(vec![]),
    };
    let report = catalog
        .discover_workspaces(&config, &provider.id, &agent)
        .await
        .unwrap();
    assert!(report.complete && report.pages == 3 && report.added == 2);
    catalog.load(&mut config).await.unwrap();
    assert_eq!(config.workspaces.len(), 3);
    assert!(
        config
            .workspaces
            .iter()
            .any(|w| w.cwd == "/projects/paged-project")
    );
    assert_eq!(
        config
            .workspaces
            .iter()
            .find(|w| w.id == existing.id)
            .unwrap()
            .name,
        "Custom project name"
    );
    let removed = config
        .workspaces
        .iter()
        .find(|w| w.cwd == "/projects/paged-project")
        .unwrap()
        .clone();
    catalog.remove_workspace(&removed.id).await.unwrap();
    catalog
        .discover_workspaces(&config, &provider.id, &agent)
        .await
        .unwrap();
    catalog.load(&mut config).await.unwrap();
    assert!(
        !config
            .workspaces
            .iter()
            .any(|w| w.id == removed.id || w.cwd == removed.cwd)
    );
    catalog
        .add_workspace(&config, &provider.id, &removed.cwd, None)
        .await
        .unwrap();
    catalog.load(&mut config).await.unwrap();
    assert!(config.workspaces.iter().any(|w| w.cwd == removed.cwd));
    assert!(
        agent
            .queries
            .lock()
            .unwrap()
            .contains(&(false, Some("second".into())))
    );
}

#[tokio::test]
async fn catalog_administration_coexists_with_the_bridge_and_keeps_stable_ids() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    let service = crate::storage::Store::open(dir.path()).await.unwrap();
    let endpoint = adapters::ServerEndpoint {
        binary: "/bin/codex".into(),
        socket: "/tmp/control.sock".into(),
    };
    let a = catalog.discover("host", endpoint.clone()).await.unwrap();
    let b = catalog.discover("host", endpoint).await.unwrap();
    assert_eq!(a.id, b.id);
    assert!(!a.control_verified);
    let mut config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    let w = catalog
        .add_workspace(&config, &a.id, "/projects/app", None)
        .await
        .unwrap();
    let same = catalog
        .add_workspace(&config, &a.id, "/projects/app", None)
        .await
        .unwrap();
    assert_eq!(w.id, same.id);
    assert!(
        catalog
            .add_workspace(&config, &a.id, "/outside", None)
            .await
            .is_err()
    );
    catalog.load(&mut config).await.unwrap();
    assert_eq!(config.workspaces.len(), 1);
    config.hosts.clear();
    catalog.load(&mut config).await.unwrap();
    assert!(config.providers.is_empty() && config.workspaces.is_empty());
    config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    assert_eq!(config.workspaces[0].id, w.id);
    catalog.remove_workspace(&w.id).await.unwrap();
    catalog.load(&mut config).await.unwrap();
    assert!(config.workspaces.is_empty());
    drop(service);
}

#[tokio::test]
async fn changed_endpoints_do_not_inherit_control_or_retarget_old_providers() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    let mut original = catalog
        .discover(
            "host",
            adapters::ServerEndpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/old.sock".into(),
            },
        )
        .await
        .unwrap();
    original.control_verified = true;
    catalog.save_provider(&original).await.unwrap();
    let new = catalog
        .discover(
            "host",
            adapters::ServerEndpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/new.sock".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(new.id, original.id);
    assert!(!new.control_verified);
    let mut config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    assert!(
        config
            .providers
            .iter()
            .find(|p| p.id == original.id)
            .unwrap()
            .control_verified
    );
}

#[tokio::test]
async fn catalog_refresh_cannot_restore_a_host_removed_by_config_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, ROOT).unwrap();
    let file = crate::reload::ConfigFile::open(path.clone(), |path, text| {
        let mut config: Config = toml::from_str(text).map_err(|_| ConfigError::Parse)?;
        config.prepare(path, text)?;
        Ok(crate::reload::ParsedConfig {
            config,
            restart_key: String::new(),
        })
    })
    .unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    catalog
        .discover(
            "host",
            adapters::ServerEndpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/old.sock".into(),
            },
        )
        .await
        .unwrap();
    let mut config = file.snapshot().config.clone();
    catalog.load(&mut config).await.unwrap();
    file.publish_catalog(config.clone()).unwrap();
    let old_hash = file.snapshot().hash.clone();
    std::fs::write(&path, "").unwrap();
    file.reload().unwrap();
    assert!(file.snapshot().config.providers.is_empty());
    assert!(!file.publish_catalog(config).unwrap());
    assert_ne!(old_hash, file.snapshot().hash);
    assert!(file.snapshot().config.hosts.is_empty());
}
