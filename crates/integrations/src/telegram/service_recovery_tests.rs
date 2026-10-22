//! Service behavior using the shared in-process fixtures.
use super::*;

#[tokio::test]
async fn management_navigation_survives_recovery_and_non_authority_config_changes() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(300, 0, "/start"))
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM navigation")
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
    fixture
        .service
        .store
        .navigation_receipt(&id, 777, now() - 86400)
        .await
        .unwrap();
    fixture.service.store.recover_inbox(1).await.unwrap();
    let path = &fixture.service.config.path;
    let config = std::fs::read_to_string(path)
        .unwrap()
        .replace(
            "telegram_messages_per_minute=60000",
            "telegram_messages_per_minute=120",
        )
        .replace("Fixture workspace", "New display name");
    std::fs::write(path, config).unwrap();
    fixture.service.config.reload().unwrap();
    fixture.service.handle(&input(301, 9, "1")).await.unwrap();
    let menu = fixture.outputs().await.into_iter().last().unwrap();
    assert!(menu_labels(&menu).contains("New display name"));
    assert!(!menu.buttons.is_empty());
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn numbered_confirmation_never_replays_a_stale_input_or_ignores_changed_scope() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(310, 9, "/new workspace task"))
        .await
        .unwrap();
    fixture.service.handle(&input(311, 9, "1")).await.unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    let id: String = sqlx::query_scalar("SELECT id FROM navigation")
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
    fixture
        .service
        .store
        .navigation_receipt(&id, 777, now())
        .await
        .unwrap();
    let mut stale = input(312, 9, "1");
    stale.requires_confirmation = true;
    fixture.service.handle(&stale).await.unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    let path = &fixture.service.config.path;
    let config = std::fs::read_to_string(path)
        .unwrap()
        .replace("cwd=\"/workspace\"", "cwd=\"/workspace/changed\"");
    std::fs::write(path, config).unwrap();
    fixture.service.config.reload().unwrap();
    fixture.choose(313, "1").await;
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    let menu = fixture.outputs().await.into_iter().last().unwrap();
    assert!(menu.text.contains("请发送会话标题"));
}

#[tokio::test]
async fn replying_to_an_old_management_screen_refreshes_instead_of_selecting_its_number() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(320, 9, "/new workspace task"))
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM navigation")
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
    fixture
        .service
        .store
        .navigation_receipt(&id, 777, now())
        .await
        .unwrap();
    let mut reply = input(321, 9, "1");
    reply.reply_to_message_id = Some(123);
    fixture.service.handle(&reply).await.unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("请发送会话标题")
    );
}

#[tokio::test]
async fn old_queued_title_updates_and_stale_keyboards_are_not_replayed_after_restart() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let mut status = Outbound::text("old status");
    status.topic_title = Some("old state in title".into());
    store.enqueue_status(&binding, &status).await.unwrap();
    let mut menu = Outbound::text("stale menu");
    menu.buttons = vec![vec![crate::telegram::api::Button {
        label: "stale".into(),
        data: "stale".into(),
    }]];
    store.enqueue(1, 10, 0, "stale menu", &menu).await.unwrap();
    store.recover_inbox(1).await.unwrap();
    let pending = store.pending_deliveries(1, now()).await.unwrap();
    assert_eq!(pending.len(), 1);
    let body: Outbound = serde_json::from_str(&pending[0].payload).unwrap();
    assert!(body.topic_title.is_none());
    assert_eq!(body.text, "old status");
}

#[tokio::test]
async fn a_management_creation_with_a_lost_reply_is_not_retried_or_bound_to_an_agent() {
    let fixture = Fixture::new().await;
    fixture.telegram.reset().await;
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/createforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string("incompatible fixture response"))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    assert!(
        fixture
            .service
            .handle(&input(230, 0, "/start"))
            .await
            .is_err()
    );
    assert!(matches!(
        fixture.service.handle(&input(231, 0, "/start")).await,
        Err(Error::Unknown)
    ));
    assert!(
        fixture
            .service
            .store
            .management_topic(1, 10)
            .await
            .unwrap()
            .is_none()
    );
    assert!(fixture.service.store.bindings(1).await.unwrap().is_empty());
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn queued_input_after_restart_requires_a_fresh_binding_scoped_confirmation() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    let queued = input(70, 11, "queued while offline");
    let old_button = store
        .callback(
            1,
            &queued,
            &fixture.service.config.snapshot().hash,
            "old action",
            now(),
        )
        .await
        .unwrap();
    let (epoch, _) = store.cursor(1, now()).await.unwrap();
    store
        .ingest(1, epoch, &[(70, Some(queued.clone()))], now())
        .await
        .unwrap();
    store.recover_inbox(1).await.unwrap();
    let recovered = store.claim_input(1).await.unwrap().unwrap();
    assert!(recovered.input.requires_confirmation);
    assert_eq!(recovered.input.date, queued.date);
    fixture.service.handle(&recovered.input).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let mut old_click = input(71, 11, "");
    old_click.callback = Some(old_button);
    fixture.service.handle(&old_click).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("尚未发送"))
        .unwrap();
    assert!(draft.text.contains("queued while offline"));
    let mut confirm = input(72, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    fixture.service.handle(&confirm).await.unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    // A still-visible draft cannot be sent after unbinding/rebinding the topic.
    let mut delayed = input(73, 11, "second queued draft");
    delayed.requires_confirmation = true;
    fixture.service.handle(&delayed).await.unwrap();
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("second queued draft"))
        .unwrap();
    detach_topic(store).await;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    let mut confirm = input(74, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    assert!(matches!(
        fixture.service.handle(&confirm).await,
        Err(Error::Expired)
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
}

#[tokio::test]
async fn updates_fetched_after_a_poll_outage_are_drafts_without_replaying_agent_input() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new().await;
    let attempts = AtomicUsize::new(0);
    Mock::given(method("POST")).and(path_regex("(?i).*/getupdates$"))
        .respond_with(move |_:&wiremock::Request| match attempts.fetch_add(1,Ordering::SeqCst) {
            0=>ResponseTemplate::new(500).set_body_string(r#"{"ok":false,"error_code":500,"description":"fixture outage"}"#),
            1=>ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[{"update_id":90,"message":{"message_id":90,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"Fixture"},"chat":{"id":10,"type":"private","first_name":"Fixture"},"text":"offline input","is_topic_message":true}}]}"#),
            _=>ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[]}"#).set_delay(Duration::from_millis(50)),
        }).mount(&fixture.telegram).await;
    let (stop, receiver) = watch::channel(false);
    let poll = tokio::spawn(fixture.service.clone().poll(receiver));
    let deadline = Instant::now() + Duration::from_secs(6);
    let recovered = loop {
        if let Some(input) = fixture.service.store.claim_input(1).await.unwrap() {
            break input;
        }
        assert!(
            Instant::now() < deadline,
            "fixture recovery update was not received"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    stop.send(true).unwrap();
    poll.await.unwrap();
    assert!(recovered.input.requires_confirmation);
    assert_eq!(recovered.input.text, "offline input");
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn a_lost_write_receipt_cannot_cause_a_second_steer() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture.state.lock().await.drop_write = true;
    let event = input(1, 11, "uncertain steer");
    assert!(matches!(
        fixture.service.handle(&event).await,
        Err(Error::Unknown)
    ));
    assert!(matches!(
        fixture.service.handle(&event).await,
        Err(Error::Unknown)
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
}

#[tokio::test]
async fn uncertain_management_topic_check_does_not_create_another_topic() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1400, 0, "/start"))
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/editforumtopic$"))
        .respond_with(ResponseTemplate::new(502))
        .with_priority(1)
        .mount(&fixture.telegram)
        .await;
    assert!(
        fixture
            .service
            .handle(&input(1401, 0, "/start"))
            .await
            .is_err()
    );
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(9)
    );
    let requests = fixture.telegram.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/createforumtopic"))
            .count(),
        1
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn read_only_buttons_survive_recovery_and_display_changes_without_an_expiry_error() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(5000, 0, "/start"))
        .await
        .unwrap();
    let menu = fixture.outputs().await.pop().unwrap();
    fixture
        .service
        .store
        .navigation_receipt(menu.navigation_id.as_ref().unwrap(), 700, now() - 86400)
        .await
        .unwrap();
    fixture.service.store.recover_inbox(1).await.unwrap();
    let file = &fixture.service.config.path;
    let text = std::fs::read_to_string(file)
        .unwrap()
        .replace("Fixture workspace", "Updated project");
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&navigation_click(&menu, 5001, "1", 700))
        .await
        .unwrap();
    let current = fixture.outputs().await.pop().unwrap();
    assert_eq!(current.edit_message, Some(700));
    assert!(menu_labels(&current).contains("Updated project"));
    assert!(!current.text.contains("过期"));
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn expired_or_recovered_create_clicks_reconfirm_and_duplicate_buttons_never_create_again() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(5010, 9, "/new workspace Synthetic title"))
        .await
        .unwrap();
    let original = fixture.outputs().await.pop().unwrap();
    fixture
        .service
        .store
        .navigation_receipt(original.navigation_id.as_ref().unwrap(), 700, now() - 86400)
        .await
        .unwrap();
    fixture
        .service
        .handle(&navigation_click(&original, 5011, "1", 700))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let renewed = fixture.outputs().await.pop().unwrap();
    assert!(renewed.text.contains("Synthetic title"));
    assert_eq!(renewed.edit_message, Some(700));
    fixture
        .service
        .store
        .navigation_receipt(renewed.navigation_id.as_ref().unwrap(), 700, now())
        .await
        .unwrap();
    let mut recovered = navigation_click(&renewed, 5012, "1", 700);
    recovered.requires_confirmation = true;
    fixture.service.handle(&recovered).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    fixture.choose(5013, "1").await;
    fixture
        .service
        .handle(&navigation_click(&original, 5014, "1", 700))
        .await
        .unwrap();
    let state = fixture.state.lock().await;
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|method| **method == "thread/start")
            .count(),
        1
    );
    assert!(!state.requests.contains(&"turn/start"));
}

#[tokio::test]
async fn queued_obsolete_menus_do_not_overwrite_the_current_page() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(5030, 0, "/start"))
        .await
        .unwrap();
    let home = fixture.outputs().await.pop().unwrap();
    fixture.choose(5031, "1").await;
    let before = fixture.outputs().await.len();
    fixture
        .service
        .handle(&navigation_click(&home, 5032, "1", 5030))
        .await
        .unwrap();
    assert_eq!(
        fixture.outputs().await.len(),
        before,
        "old clicks must not enqueue duplicate pending pages"
    );
    fixture.deliver_pending().await;
    let obsolete: String = sqlx::query_scalar(
        "SELECT state FROM deliveries WHERE json_extract(payload,'$.navigation_id')=?",
    )
    .bind(home.navigation_id)
    .fetch_one(&fixture.service.store.pool)
    .await
    .unwrap();
    assert_eq!(obsolete, "rejected");
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn consumed_write_buttons_do_not_recreate_confirmation_after_an_interrupted_ui_update() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(5040, 9, "/new workspace Synthetic title"))
        .await
        .unwrap();
    let menu = fixture.outputs().await.pop().unwrap();
    fixture
        .service
        .store
        .navigation_receipt(menu.navigation_id.as_ref().unwrap(), 700, now())
        .await
        .unwrap();
    sqlx::query("UPDATE navigation SET consumed=1 WHERE id=?")
        .bind(menu.navigation_id.as_ref().unwrap())
        .execute(&fixture.service.store.pool)
        .await
        .unwrap();
    fixture
        .service
        .handle(&navigation_click(&menu, 5041, "1", 700))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let current = fixture.outputs().await.pop().unwrap();
    assert!(current.text.contains("请发送会话标题"));
    assert!(!menu_labels(&current).contains("确认创建"));
}
