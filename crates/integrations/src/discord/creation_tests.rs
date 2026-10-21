use super::*;

fn button_labels(screen: &Screen) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Row {
        components: Vec<Control>,
    }
    #[derive(serde::Deserialize)]
    struct Control {
        label: Option<String>,
    }
    let rows: Vec<Row> =
        serde_json::from_str(&serde_json::to_string(&screen.components).unwrap()).unwrap();
    rows.into_iter()
        .flat_map(|row| row.components)
        .filter_map(|control| control.label)
        .collect()
}

#[tokio::test]
async fn short_session_lists_fit_one_screen_and_expired_page_history_has_no_paging_buttons() {
    let (_dir, _server, service, _api) = fixture().await;
    let snapshot = service.config.snapshot();
    let project = snapshot.config.workspaces[0].id.clone();
    for total in [0, 1, 25] {
        let data = (0..total)
            .map(|index| session::Session {
                reference: session::SessionRef {
                    instance: snapshot.config.workspaces[0].provider_id.clone(),
                    thread: format!("session-{index}"),
                },
                title: format!("Session {index}"),
                native_title: None,
                cwd: snapshot.config.workspaces[0].cwd.clone(),
                status: session::RuntimeStatus::Idle,
                active_turn: None,
                updated_at: None,
            })
            .collect();
        let loaded = crate::navigation::SessionPage {
            selection: Navigator::new(&snapshot.config).parent(&project).unwrap(),
            sessions: engine::runtimes::PickerPage {
                data,
                next_cursor: None,
                offset: 0,
                total,
            },
        };
        let menu = service
            .session_page(
                &input(),
                &snapshot,
                loaded,
                vec![None, Some("expired".into())],
            )
            .unwrap();
        let screen = menu
            .bind(&service.store, &input(), &snapshot.hash, "zh-CN")
            .await
            .unwrap();
        let labels = button_labels(&screen);
        assert!(
            !labels
                .iter()
                .any(|label| matches!(label.as_str(), "上一页" | "下一页"))
        );
        assert!(labels.iter().any(|label| label.contains("新建会话")));
        if total > 0 {
            assert_eq!(selector(&screen).1.as_array().unwrap().len(), total);
        }
    }
}

#[tokio::test]
async fn creation_form_and_confirmation_are_scoped_single_use_and_do_not_contact_runtime() {
    use crate::discord::Backend;
    let (_dir, server, service, api) = fixture().await;
    let snapshot = service.config.snapshot();
    let project = snapshot.config.workspaces[0].id.clone();
    let token = service
        .store
        .ui_action(
            &input(),
            &snapshot.hash,
            Action::Navigate(View::NewSession { project }),
        )
        .await
        .unwrap();
    let mut other = input();
    other.user = 99;
    assert!(service.new_session_form(&api, other, &token).await.is_err());
    let form = service
        .new_session_form(&api, input(), &token)
        .await
        .unwrap();
    assert!(form.title.contains("Project 00"));
    assert!(
        service
            .new_session_form(&api, input(), &token)
            .await
            .is_err()
    );
    assert!(
        service
            .menu_screen(&api, input(), Some(form.token.clone()), Some(" ".into()))
            .await
            .is_err()
    );
    let confirmation = service
        .menu_screen(
            &api,
            input(),
            Some(form.token.clone()),
            Some("**literal task** <@123>".into()),
        )
        .await
        .unwrap();
    assert!(
        confirmation.text.contains("确认")
            && confirmation.text.contains("\\*\\*literal task\\*\\*")
    );
    assert!(
        button_labels(&confirmation)
            .iter()
            .any(|label| label.contains("创建并发送"))
    );
    assert!(
        service
            .menu_screen(&api, input(), Some(form.token), Some("same draft".into()))
            .await
            .is_err()
    );
    let operations: i64 = sqlx::query_scalar("SELECT count(*) FROM operations")
        .fetch_one(service.store.pool())
        .await
        .unwrap();
    assert_eq!(operations, 0);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
    let create = button_token(&confirmation, "创建并发送任务");
    // The final action is consumed even if its execution outcome later becomes unknown.
    assert!(matches!(
        service
            .store
            .ui_resolve(&input(), &snapshot.hash, &create, None)
            .await
            .unwrap(),
        Some(View::Create { .. })
    ));
    assert!(
        service
            .store
            .ui_resolve(&input(), &snapshot.hash, &create, None)
            .await
            .unwrap()
            .is_none()
    );
}
