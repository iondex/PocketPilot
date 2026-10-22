//! Service behavior using the shared in-process fixtures.
use super::*;

#[tokio::test]
async fn management_creation_and_existing_topic_repair_keep_the_icon_out_of_the_title() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1800, 0, "/start"))
        .await
        .unwrap();
    // An existing address can belong to an older version with an emoji in its name.
    fixture
        .service
        .handle(&input(1801, 0, "/start"))
        .await
        .unwrap();

    #[derive(Deserialize)]
    struct TopicAppearance {
        name: String,
        icon_custom_emoji_id: String,
        message_thread_id: Option<i32>,
    }

    let requests = fixture.telegram.received_requests().await.unwrap();
    let created: Vec<_> = requests
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/createforumtopic")
        })
        .collect();
    assert_eq!(created.len(), 1, "reuse the existing management topic");
    let created: TopicAppearance = serde_json::from_slice(&created[0].body).unwrap();
    assert_eq!(created.name, "管理");
    assert_eq!(created.icon_custom_emoji_id, "folder");
    let repaired = requests
        .iter()
        .find(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/editforumtopic")
        })
        .unwrap();
    let repaired: TopicAppearance = serde_json::from_slice(&repaired.body).unwrap();
    assert_eq!(repaired.name, "管理");
    assert_eq!(repaired.icon_custom_emoji_id, "folder");
    assert_eq!(repaired.message_thread_id, Some(9));
    assert_eq!(
        crate::telegram::i18n::text("en", "tg.management.title"),
        "Manage"
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn topic_removal_requires_confirmation_and_only_removes_the_telegram_association() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1900, 0, "/open workspace thread"))
        .await
        .unwrap();
    let before = fixture.state.lock().await.requests.clone();
    // Unbound topics must remain manageable until they are actually deleted.
    detach_topic(&fixture.service.store).await;
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/deleteforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .handle(&input(1901, 9, "/topics"))
        .await
        .unwrap();
    fixture.choose(1902, "1").await;
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("全部聊天记录")
    );
    assert!(
        fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/deleteforumtopic"))
    );
    fixture.choose(1903, "1").await;
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .topic(1, 10, "instance", "thread")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .begin_topic(1, 10, "instance", "thread")
            .await
            .unwrap(),
        "a fresh topic may be created for the same native session"
    );
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM deliveries WHERE topic_id=11 AND state='pending'")
            .fetch_one(&fixture.service.store.pool)
            .await
            .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(
        fixture.state.lock().await.requests,
        before,
        "topic management must not touch the agent"
    );
    fixture.service.store.recover_inbox(1).await.unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|o| o.text.contains("Codex 会话继续保留"))
    );
    #[derive(Deserialize)]
    struct Target {
        chat_id: i64,
        message_thread_id: i32,
    }
    let requests = fixture.telegram.received_requests().await.unwrap();
    let request = requests
        .iter()
        .find(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/deleteforumtopic")
        })
        .unwrap();
    let target: Target = serde_json::from_slice(&request.body).unwrap();
    assert_eq!((target.chat_id, target.message_thread_id), (10, 11));
}

#[tokio::test]
async fn manually_deleted_topics_sync_without_treating_closed_or_uncertain_topics_as_deleted() {
    for (description, removed) in [
        ("Bad Request: message thread not found", true),
        ("Bad Request: TOPIC_CLOSED", false),
        ("Bad Request: TOPIC_NOT_MODIFIED", false),
        ("Bad Request: unrelated failure", false),
    ] {
        let fixture = Fixture::new().await;
        fixture
            .service
            .handle(&input(1920, 0, "/open workspace thread"))
            .await
            .unwrap();
        let before = fixture.state.lock().await.requests.clone();
        Mock::given(method("POST"))
            .and(path_regex("(?i).*/editforumtopic$"))
            .respond_with(ResponseTemplate::new(400).set_body_string(format!(
                r#"{{"ok":false,"error_code":400,"description":"{description}"}}"#
            )))
            .with_priority(1)
            .mount(&fixture.telegram)
            .await;
        fixture
            .service
            .handle(&input(1921, 9, "/topics"))
            .await
            .unwrap();
        assert_eq!(
            fixture
                .service
                .store
                .binding(1, 10, 11)
                .await
                .unwrap()
                .is_none(),
            removed,
            "{description}"
        );
        assert_eq!(
            fixture
                .service
                .store
                .topic(1, 10, "instance", "thread")
                .await
                .unwrap()
                .is_none(),
            removed,
            "{description}"
        );
        assert_eq!(fixture.state.lock().await.requests, before);
        let requests = fixture.telegram.received_requests().await.unwrap();
        #[derive(Deserialize)]
        struct Check {
            name: Option<String>,
            icon_custom_emoji_id: Option<String>,
        }
        let last = requests
            .iter()
            .rev()
            .find(|r| {
                r.url
                    .path()
                    .to_ascii_lowercase()
                    .ends_with("/editforumtopic")
            })
            .unwrap();
        let check: Check = serde_json::from_slice(&last.body).unwrap();
        assert!(
            check.name.is_none() && check.icon_custom_emoji_id.is_none(),
            "existence checks must preserve user metadata"
        );
    }
}

#[tokio::test]
async fn uncertain_topic_deletion_is_not_replayed_and_cannot_reactivate_the_old_binding() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1940, 0, "/open workspace thread"))
        .await
        .unwrap();
    let before = fixture.state.lock().await.requests.clone();
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/deleteforumtopic$"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .handle(&input(1941, 9, "/topics"))
        .await
        .unwrap();
    fixture.choose(1942, "1").await;
    fixture.choose(1943, "1").await;
    fixture.service.store.recover_inbox(1).await.unwrap();
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .bind(1, 10, 11, "instance", "thread")
            .await
            .is_err()
    );
    let state: String = sqlx::query_scalar("SELECT state FROM topic_removals")
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
    assert_eq!(state, "outcome_unknown");
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|o| o.text.contains("删除结果未确认"))
    );
    assert_eq!(fixture.state.lock().await.requests, before);
}

#[tokio::test]
async fn delayed_topic_removal_confirmation_needs_a_new_confirmation() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1960, 0, "/open workspace thread"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(1961, 9, "/topics"))
        .await
        .unwrap();
    fixture.choose(1962, "1").await;
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
    let mut stale = input(1963, 9, "1");
    stale.requires_confirmation = true;
    fixture.service.handle(&stale).await.unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("全部聊天记录")
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/deleteforumtopic"))
    );
}

#[tokio::test]
async fn topic_removal_cannot_target_another_private_chat_or_the_management_topic() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1980, 0, "/open workspace thread"))
        .await
        .unwrap();
    fixture
        .service
        .store
        .begin_topic(1, 20, "instance", "other")
        .await
        .unwrap();
    fixture
        .service
        .store
        .end_topic(1, 20, "instance", "other", "accepted", Some(22))
        .await
        .unwrap();
    fixture
        .service
        .remove_topic(&input(1981, 9, "1"), &fixture.service.config.snapshot(), 22)
        .await
        .unwrap();
    assert!(
        fixture
            .service
            .remove_topic(&input(1982, 9, "1"), &fixture.service.config.snapshot(), 9)
            .await
            .is_err()
    );
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(9)
    );
    assert_eq!(
        fixture
            .service
            .store
            .topic(1, 20, "instance", "other")
            .await
            .unwrap(),
        Some(22)
    );
    assert!(
        fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/deleteforumtopic"))
    );
}

#[tokio::test]
async fn localized_command_menus_are_registered_only_for_private_chats() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/setmycommands$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#))
        .expect(3)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .api
        .register_commands("zh-CN")
        .await
        .unwrap();
    #[derive(Deserialize)]
    struct Scope {
        #[serde(rename = "type")]
        kind: String,
    }
    #[derive(Deserialize)]
    struct Command {
        command: String,
        description: String,
    }
    #[derive(Deserialize)]
    struct Menu {
        language_code: String,
        scope: Scope,
        commands: Vec<Command>,
    }
    let requests = fixture.telegram.received_requests().await.unwrap();
    let menus = requests
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/setmycommands")
        })
        .map(|r| serde_json::from_slice::<Menu>(&r.body).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(menus.len(), 3);
    for menu in &menus {
        assert_eq!(menu.scope.kind, "all_private_chats");
        assert!(menu.commands.iter().any(|c| c.command == "topics"));
        assert!(
            menu.commands
                .iter()
                .all(|c| !matches!(c.command.as_str(), "cleartopics" | "unbind"))
        );
        assert!(menu.commands.iter().all(|c| !matches!(
            c.command.as_str(),
            "stop" | "cancel" | "kill"
        ) && !c.description.starts_with("cmd_")));
        let start = menu.commands.iter().find(|c| c.command == "start").unwrap();
        assert_eq!(
            start.description,
            if menu.language_code == "en" {
                "Open management"
            } else {
                "打开管理首页"
            }
        );
    }
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn bulk_topic_cleanup_confirms_a_snapshot_and_preserves_new_topics_management_and_native_sessions()
 {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(2000, 0, "/open workspace thread"))
        .await
        .unwrap();
    let before = fixture.state.lock().await.requests.clone();
    fixture
        .service
        .store
        .begin_topic(1, 10, "instance", "second")
        .await
        .unwrap();
    fixture
        .service
        .store
        .end_topic(1, 10, "instance", "second", "accepted", Some(12))
        .await
        .unwrap();
    fixture
        .service
        .store
        .bind(1, 10, 12, "instance", "second")
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/deleteforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#))
        .expect(2)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .handle(&input(2001, 9, "/topics clear"))
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
    let mut delayed = input(2002, 9, "1");
    delayed.requires_confirmation = true;
    fixture.service.handle(&delayed).await.unwrap();
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("2 个会话话题")
    );
    fixture
        .service
        .store
        .begin_topic(1, 10, "instance", "third")
        .await
        .unwrap();
    fixture
        .service
        .store
        .end_topic(1, 10, "instance", "third", "accepted", Some(13))
        .await
        .unwrap();
    fixture.choose(2003, "1").await;
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 12)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .service
            .store
            .topic(1, 10, "instance", "third")
            .await
            .unwrap(),
        Some(13)
    );
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(9)
    );
    assert_eq!(fixture.state.lock().await.requests, before);
    let outputs = fixture.outputs().await;
    assert!(
        outputs
            .iter()
            .any(|o| o.text.contains("已清除 2 个会话话题"))
    );
}

#[tokio::test]
async fn bulk_topic_cleanup_stops_at_an_uncertain_result_without_replaying_or_touching_the_rest() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(2020, 0, "/open workspace thread"))
        .await
        .unwrap();
    fixture
        .service
        .store
        .begin_topic(1, 10, "instance", "second")
        .await
        .unwrap();
    fixture
        .service
        .store
        .end_topic(1, 10, "instance", "second", "accepted", Some(12))
        .await
        .unwrap();
    fixture
        .service
        .store
        .bind(1, 10, 12, "instance", "second")
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/deleteforumtopic$"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .handle(&input(2021, 9, "/topics clear"))
        .await
        .unwrap();
    fixture.choose(2022, "1").await;
    fixture.service.store.recover_inbox(1).await.unwrap();
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 12)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|o| o.text.contains("已清除 0 / 2") && o.text.contains("不会自动重试"))
    );
}

#[tokio::test]
async fn native_topic_icons_replace_automatic_status_bubbles_and_agent_markdown_is_rendered() {
    let fixture = Fixture::new().await;
    fixture.state.lock().await.items=vec![serde_json::from_str(r##"{"type":"agentMessage","id":"m","text":"# 推荐\n\n**字段说明： **示例。\n\n**结论。**后续中文。 [链接](https://example.com)\n\n| 客户端 | 平台 |\n|---|---|\n| **Example** | Android |","phase":"final_answer"}"##).unwrap()];
    fixture
        .service
        .handle(&input(1600, 0, "/open workspace thread"))
        .await
        .unwrap();
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().deliver(receiver));
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deliveries WHERE state IN ('pending','dispatching')",
        )
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
        if pending == 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap();
    #[derive(Deserialize)]
    struct TopicEdit {
        icon_custom_emoji_id: Option<String>,
    }
    let requests = fixture.telegram.received_requests().await.unwrap();
    let session_messages = requests
        .iter()
        .filter_map(sent_rich)
        .filter(|request| request.message_thread_id == Some(11))
        .collect::<Vec<_>>();
    assert_eq!(
        session_messages.len(),
        1,
        "only the agent reply belongs in the session"
    );
    let message = &session_messages[0].rich_message;
    let plain = message.plain();
    assert!(message.skip_entity_detection);
    assert!(plain.starts_with("🤖 Agent · ✅\n\n推荐"));
    assert!(plain.contains("字段说明： 示例。"));
    assert!(plain.contains("结论。后续中文。"));
    assert!(plain.contains("平台: Android"));
    assert!(
        message
            .blocks
            .iter()
            .any(|block| matches!(block, crate::telegram::rich::Block::Heading { size: 1, .. }))
    );
    assert!(
        message
            .blocks
            .iter()
            .any(|block| matches!(block, crate::telegram::rich::Block::Table { .. }))
    );
    assert!(!plain.contains("**"));
    assert!(
        requests
            .iter()
            .filter(|r| r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/editforumtopic"))
            .any(|r| serde_json::from_slice::<TopicEdit>(&r.body)
                .unwrap()
                .icon_custom_emoji_id
                .as_deref()
                == Some("busy"))
    );
    let binding = fixture
        .service
        .store
        .binding(1, 10, 11)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.topic_title.as_deref(), Some("Fixture title"));
    assert!(binding.status_message_id.is_none());
}

#[tokio::test]
async fn typing_is_renewed_without_new_text_and_stops_after_unbinding() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1700, 0, "/open workspace thread"))
        .await
        .unwrap();
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().typing(receiver));
    let deadline = Instant::now() + Duration::from_secs(7);
    loop {
        let count = fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| {
                r.url
                    .path()
                    .to_ascii_lowercase()
                    .ends_with("/sendchataction")
            })
            .count();
        if count >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "typing was not renewed");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    detach_topic(&fixture.service.store).await;
    let before = fixture
        .telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/sendchataction")
        })
        .count();
    tokio::time::sleep(Duration::from_millis(4300)).await;
    stop.send(true).unwrap();
    worker.await.unwrap();
    let after = fixture
        .telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/sendchataction")
        })
        .count();
    assert_eq!(before, after);
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
}

#[tokio::test]
async fn icon_discovery_backoff_does_not_confirm_the_icon_or_block_session_messages() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    Mock::given(path_regex("(?i).*/getforumtopiciconstickers$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"ok":false,"error_code":500,"description":"Internal Server Error"}"#,
        ))
        .expect(1)
        .with_priority(1)
        .mount(&fixture.telegram)
        .await;
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    fixture.service.refresh_binding(&binding).await.unwrap();
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().deliver(receiver));
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let states: Vec<String> = sqlx::query_scalar(
            "SELECT state FROM deliveries WHERE source_key LIKE 'initial-history:%'",
        )
        .fetch_all(&store.pool)
        .await
        .unwrap();
        if states == vec!["delivered".to_string()] {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "icon backoff blocked the visible message"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    assert!(
        binding.status_text.is_none(),
        "failed icon discovery was recorded as applied"
    );
    let deferred: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM deliveries WHERE source_key LIKE 'appearance:%' AND state='pending' AND not_before>?)")
        .bind(now()).fetch_one(&store.pool).await.unwrap();
    assert!(deferred);
}

#[tokio::test]
async fn unverified_appearance_receipts_are_repaired_once_without_replaying_history() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    fixture.service.refresh_binding(&binding).await.unwrap();
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().deliver(receiver));
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .unwrap()
            .status_text
            .as_deref()
            == Some("icon:v1:⚡️")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap();
    // Simulate a prior version's receipt that did not prove an icon was in the request.
    sqlx::query(
        "UPDATE bindings SET status_text='⚡️' WHERE bot_id=1 AND chat_id=10 AND topic_id=11",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    // Remove only this fixture's new receipt row to model an older queue identity.
    sqlx::query(
        "UPDATE deliveries SET source_key='appearance:legacy' WHERE source_key LIKE 'appearance:%'",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let before = fixture
        .outputs()
        .await
        .iter()
        .filter(|b| !b.text.is_empty())
        .count();
    fixture.service.refresh_binding(&binding).await.unwrap();
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM deliveries WHERE source_key LIKE 'appearance:v2:%' AND state='pending'")
        .fetch_one(&store.pool).await.unwrap();
    assert_eq!(pending, 1);
    assert_eq!(
        fixture
            .outputs()
            .await
            .iter()
            .filter(|b| !b.text.is_empty())
            .count(),
        before
    );
}
