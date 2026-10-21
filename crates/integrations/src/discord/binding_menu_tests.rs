use super::*;
use crate::bindings::{Manager, Scope, State, Target};

#[tokio::test]
async fn binding_menu_uses_scoped_handles_and_confirms_detach_without_runtime_io() {
    let (_dir, server, service, api) = fixture().await;
    let snapshot = service.config.snapshot();
    let project = &snapshot.config.workspaces[0];
    let endpoint =
        engine::runtimes::fingerprint(&snapshot.config.hosts[0], &snapshot.config.providers[0]);
    sqlx::query("INSERT INTO discord_bindings(bot,guild,channel,owner,workspace,instance,thread,cwd,endpoint) VALUES('1','2','7','10',?,?,'native-thread',?,?)")
        .bind(&project.id).bind(&project.provider_id).bind(&project.cwd).bind(endpoint).execute(service.store.pool()).await.unwrap();
    let mut request = input();
    request.command = Command::Bindings;
    let list = service
        .menu_screen(&api, request.clone(), None, None)
        .await
        .unwrap();
    let (token, options) = selector(&list);
    assert_eq!(options.as_array().unwrap().len(), 1);
    assert!(!list.text.contains("native-thread"));
    let detail = service
        .menu_screen(&api, request.clone(), Some(token), Some("0".into()))
        .await
        .unwrap();
    assert!(detail.text.contains("Alpha") && detail.text.contains("Project 00"));
    let paused = service
        .menu_screen(
            &api,
            request.clone(),
            Some(button_token(
                &detail,
                &i18n::text("zh-CN", "bindings.pause"),
            )),
            None,
        )
        .await
        .unwrap();
    assert!(paused.text.contains("已暂停"));
    assert!(
        service
            .menu_screen(
                &api,
                request.clone(),
                Some(button_token(
                    &detail,
                    &i18n::text("zh-CN", "bindings.pause")
                )),
                None
            )
            .await
            .is_err(),
        "mutation handles are single-use"
    );
    let requests_before = server.received_requests().await.unwrap().len();
    let binding = service
        .store
        .discord_binding(1, 2, 7)
        .await
        .unwrap()
        .unwrap();
    service.refresh(&api, &binding).await.unwrap();
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        requests_before,
        "paused observation performs no authorization or runtime network reads"
    );
    let confirmation = service
        .menu_screen(
            &api,
            request.clone(),
            Some(button_token(
                &paused,
                &i18n::text("zh-CN", "bindings.detach"),
            )),
            None,
        )
        .await
        .unwrap();
    let scope = Scope::Discord {
        bot: 1,
        guild: 2,
        owner: 10,
    };
    let target = Target::Discord {
        channel: "7".into(),
    };
    let manager = Manager::new(&service.store, scope);
    assert_eq!(
        manager
            .entry(&snapshot.config, &target, "en")
            .await
            .unwrap()
            .state,
        State::Paused
    );
    let token = button_token(
        &confirmation,
        &i18n::text("zh-CN", "bindings.confirm_button"),
    );
    service
        .menu_screen(&api, request.clone(), Some(token.clone()), None)
        .await
        .unwrap();
    assert_eq!(
        manager
            .entry(&snapshot.config, &target, "en")
            .await
            .unwrap()
            .state,
        State::Detached
    );
    assert!(
        service
            .menu_screen(&api, request, Some(token), None)
            .await
            .is_err()
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
}
