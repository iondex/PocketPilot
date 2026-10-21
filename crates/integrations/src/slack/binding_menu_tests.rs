use super::*;
use crate::bindings::{Manager, Target};

#[tokio::test]
async fn bound_views_use_thread_links_and_independent_single_use_management_handles() {
    let (_dir, service, config) = fixture().await;
    let snapshot = config.snapshot();
    let project = &snapshot.config.workspaces[0];
    let endpoint =
        engine::runtimes::fingerprint(&snapshot.config.hosts[0], &snapshot.config.providers[0]);
    sqlx::query("INSERT INTO slack_bindings(id,app,team,bot,channel,root,owner,workspace,instance,thread,cwd,endpoint) VALUES('binding','A1','T1','U2','D1','123.000001','U1',?,?,'native-thread',?,?)")
        .bind(&project.id).bind(&project.provider_id).bind(&project.cwd).bind(endpoint).execute(service.store.core.pool()).await.unwrap();
    let request = Request {
        id: "binding-menu".into(),
        channel: "D1".into(),
        user: "U1".into(),
        root: Some("123.000001".into()),
        binding: Some("binding".into()),
        revision: Some(1),
        uploads: vec![],
        menu_message: Some("200.000001".into()),
        date: engine::now(),
        command: Command::Bindings { offset: 0 },
    };
    service
        .binding_list(&request, &snapshot.hash, &snapshot, 0)
        .await
        .unwrap();
    let target = Target::Slack {
        id: "binding".into(),
    };
    let manager = Manager::new(&service.store, service.binding_scope());
    let entry = manager
        .entry(&snapshot.config, &target, "en")
        .await
        .unwrap();
    let mut detail = request.clone();
    detail.id = "binding-detail".into();
    service
        .binding_detail(&detail, &snapshot.hash, &snapshot, &target)
        .await
        .unwrap();
    let payload: String =
        sqlx::query_scalar("SELECT content FROM slack_outbox WHERE source='binding-detail'")
            .fetch_one(service.store.core.pool())
            .await
            .unwrap();
    assert!(payload.contains("https://app.slack.com/client/T1/D1/thread/D1-123.000001"));
    let mut confirm = request.clone();
    confirm.id = "binding-confirm".into();
    service
        .binding_confirmation(&confirm, &snapshot.hash, &target, entry.version, &snapshot)
        .await
        .unwrap();
    assert!(service.store.binding("binding").await.unwrap().is_some());
    let rows = sqlx::query("SELECT nonce,action,binding,root FROM slack_actions")
        .fetch_all(service.store.core.pool())
        .await
        .unwrap();
    #[derive(serde::Deserialize)]
    struct Saved {
        #[serde(flatten)]
        action: Action,
    }
    let row = rows
        .iter()
        .find(|row| {
            let saved: Saved = serde_json::from_str(row.get("action")).unwrap();
            matches!(
                saved.action,
                Action::Run(Command::BindingChange {
                    change: crate::bindings::Change::Detach,
                    ..
                })
            )
        })
        .unwrap();
    assert!(row.get::<Option<String>, _>("binding").is_none());
    assert!(row.get::<Option<String>, _>("root").is_none());
    let nonce: String = row.get("nonce");
    assert!(
        service
            .click(
                &nonce,
                Some("D1"),
                "other",
                None,
                InteractionSource {
                    id: Some("bad"),
                    message: None
                }
            )
            .await
            .unwrap()
            .is_none()
    );
    let (resolved, _) = service
        .click(
            &nonce,
            Some("D1"),
            "U1",
            None,
            InteractionSource {
                id: Some("confirm"),
                message: Some("200.000001"),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(resolved.command, Command::BindingChange { .. }));
    assert!(
        service
            .click(
                &nonce,
                Some("D1"),
                "U1",
                None,
                InteractionSource {
                    id: Some("again"),
                    message: None
                }
            )
            .await
            .unwrap()
            .is_none()
    );
}
