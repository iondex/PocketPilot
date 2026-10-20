use super::*;
use sqlx::Row;

#[tokio::test]
async fn workspace_then_project_menus_filter_scope_and_update_the_existing_screen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, "[[hosts]]\nid='alpha'\nname='Alpha'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n[[hosts]]\nid='beta'\nname='Beta'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']").unwrap();
    let config = Arc::new(
        engine::reload::ConfigFile::open(path, |path, text| {
            let mut config: engine::config::Config = toml::from_str(text).unwrap();
            config.prepare(path, text)?;
            for id in ["alpha", "beta"] {
                config.providers.push(engine::model::Provider {
                    id: format!("p-{id}"),
                    host_id: id.into(),
                    kind: "codex".into(),
                    connection_mode: "existing_proxy".into(),
                    compatibility_profile: "fixture".into(),
                    control_verified: false,
                    endpoint: None,
                });
                config.workspaces.push(engine::config::Workspace {
                    id: format!("project-{id}"),
                    provider_id: format!("p-{id}"),
                    name: format!("Project {id}"),
                    short_name: id.into(),
                    cwd: format!("/projects/{id}"),
                    additional_cwds: Vec::new(),
                    updated_at: None,
                });
            }
            let mut recent = config.workspaces[0].clone();
            recent.id = "recent-alpha".into();
            recent.name = "ZZ recent project".into();
            recent.short_name = "recent".into();
            recent.cwd = "/projects/alpha-recent".into();
            recent.updated_at = Some(500);
            config.workspaces.push(recent);
            config.validate()?;
            Ok(engine::reload::ParsedConfig {
                config,
                restart_key: String::new(),
            })
        })
        .unwrap(),
    );
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let (_stop, receiver) = watch::channel(false);
    let service = Service {
        settings: crate::slack::config::Config {
            proxy: Default::default(),
            app_token: "xapp-fixture".into(),
            bot_token: "xoxb-fixture".into(),
            app_id: "A1".into(),
            team_id: "T1".into(),
            owner_user_id: "U1".into(),
        },
        context: Context {
            config: config.clone(),
            runtimes: Arc::new(engine::runtimes::RuntimeRegistry::new(core.clone())),
            store: core.clone(),
        },
        store: Store::new(core, "A1", "T1", "U2"),
        api: Arc::new(crate::slack::tests::NoNetwork),
        bot: "U2".into(),
        direct_channel: "D1".into(),
        stop: receiver,
        cutoff: AtomicI64::new(0),
        verified: AtomicBool::new(true),
    };
    let snapshot = config.snapshot();
    let mut r = Request {
        id: "menu".into(),
        channel: "D1".into(),
        user: "U1".into(),
        root: None,
        uploads: Vec::new(),
        menu_message: None,
        binding: None,
        revision: None,
        date: engine::now(),
        command: Command::Menu,
    };
    service
        .workspace_menu(&r, &snapshot.hash, &snapshot, 0)
        .await
        .unwrap();
    let row = sqlx::query("SELECT content FROM slack_outbox WHERE source='menu'")
        .fetch_one(service.store.core.pool())
        .await
        .unwrap();
    let workspace_menu: Content = serde_json::from_str(row.get("content")).unwrap();
    assert_eq!(
        workspace_menu.blocks.len(),
        3,
        "one header, one hint and compact workspace buttons"
    );
    assert!(
        workspace_menu.blocks[2..]
            .iter()
            .all(|b| matches!(b, SlackBlock::Actions(a) if a.elements.len() == 2))
    );
    r.id = "projects".into();
    r.menu_message = Some("100.000001".into());
    service
        .project_menu(&r, &snapshot.hash, &snapshot, "alpha", 0)
        .await
        .unwrap();
    let row = sqlx::query("SELECT content,root FROM slack_outbox WHERE source='projects'")
        .fetch_one(service.store.core.pool())
        .await
        .unwrap();
    let project_menu: Content = serde_json::from_str(row.get("content")).unwrap();
    assert!(project_menu.edit);
    assert_eq!(row.get::<String, _>("root"), "100.000001");
    let SlackBlock::Actions(actions) = &project_menu.blocks[2] else {
        panic!("expected a project selector")
    };
    let SlackActionBlockElement::StaticSelect(select) = &actions.elements[0] else {
        panic!("expected a native static select")
    };
    let groups = select.option_groups.as_ref().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].options.len(),
        2,
        "projects from Beta must not appear under Alpha"
    );
    assert_eq!(
        serde_json::to_value(&groups[0].options[0].text).unwrap()["text"],
        "ZZ recent project"
    );
    let nonce = &groups[0].options[0].value;
    let (selected, _) = service
        .store
        .consume(
            nonce,
            Some("D1"),
            "U1",
            &snapshot.hash,
            None,
            InteractionSource {
                id: Some("selection"),
                message: Some("100.000001"),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(selected.command, Command::Sessions { workspace, .. } if workspace == "recent-alpha")
    );
    assert_eq!(selected.menu_message.as_deref(), Some("100.000001"));
    assert!(
        selected.root.is_none(),
        "navigation message must not become an agent session root"
    );
    if let Some(output) = std::env::var_os("POCKETPILOT_BLOCK_PREVIEW") {
        let output = std::path::PathBuf::from(output);
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(
            output.join("slack-workspaces.json"),
            serde_json::to_string_pretty(&workspace_menu).unwrap(),
        )
        .unwrap();
        std::fs::write(
            output.join("slack-projects.json"),
            serde_json::to_string_pretty(&project_menu).unwrap(),
        )
        .unwrap();
        let mut sessions = render::message("Project alpha · sessions".into());
        sessions.blocks = vec![
            render::header("Project alpha"),
            render::context("Alpha · /projects/alpha"),
            render::section_action(
                "Review deployment",
                &render::state(&session::RuntimeStatus::NotLoaded, "en"),
                "Open session",
                "fixture-a",
            ),
            render::section_action(
                "Inspect logs",
                &render::state(&session::RuntimeStatus::Idle, "en"),
                "Open session",
                "fixture-b",
            ),
            render::button("New session", "fixture-new"),
            render::button("Switch project", "fixture-back"),
        ];
        render::compact_actions(&mut sessions);
        assert_eq!(
            sessions.blocks.len(),
            5,
            "session buttons belong beside their titles; related footer controls share one row"
        );
        std::fs::write(
            output.join("slack-sessions.json"),
            serde_json::to_string_pretty(&sessions).unwrap(),
        )
        .unwrap();
    }
}
