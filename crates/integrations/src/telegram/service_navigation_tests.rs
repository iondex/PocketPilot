//! Service behavior using the shared in-process fixtures.
use super::*;

#[tokio::test]
async fn management_topic_is_separate_and_unbound_messages_never_reach_an_agent() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(200, 0, "/start"))
        .await
        .unwrap();
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(9)
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 9)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .bind(1, 10, 9, "instance", "thread")
            .await
            .is_err()
    );
    fixture
        .service
        .handle(&input(201, 0, "default user message"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(202, 9, "management user message"))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let outputs = fixture.outputs().await;
    assert!(
        outputs
            .iter()
            .all(|b| !b.text.contains("default user message")
                && !b.text.contains("management user message"))
    );
    assert!(outputs.iter().any(|b| b.text.contains("这里只接受")));
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
    fixture
        .service
        .handle(&input(203, 11, "only this session input"))
        .await
        .unwrap();
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
}

#[tokio::test]
async fn management_buttons_edit_the_same_message_and_keep_a_compact_workspace_layout() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(210, 0, "/start"))
        .await
        .unwrap();
    let home = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.navigation_id.is_some())
        .unwrap();
    assert!(home.markdown && !home.buttons.is_empty());
    assert!(home.text.contains("🌐 主机"));
    assert!(home.buttons.iter().flatten().all(|b| b.data.len() <= 64));
    fixture.deliver_pending().await;
    fixture.choose(211, "1").await;
    let workspace = fixture.outputs().await.pop().unwrap();
    assert!(workspace.text.contains("工作区"));
    assert_eq!(workspace.edit_message, Some(700));
    fixture.deliver_pending().await;
    fixture.choose(212, "1").await;
    let sessions = fixture.outputs().await.pop().unwrap();
    assert!(menu_labels(&sessions).contains("⚡"));
    assert!(!menu_labels(&sessions).contains("运行中"));
    assert!(menu_labels(&sessions).contains("新建会话"));
    fixture.deliver_pending().await;
    #[derive(Deserialize)]
    struct Sent {
        message_thread_id: Option<i32>,
        message_id: Option<i32>,
        parse_mode: Option<String>,
        reply_markup: Markup,
    }
    #[derive(Deserialize)]
    struct Markup {
        inline_keyboard: Vec<Vec<serde::de::IgnoredAny>>,
    }
    let requests = fixture.telegram.received_requests().await.unwrap();
    let menus = requests
        .iter()
        .filter(|r| r.url.path().to_ascii_lowercase().ends_with("/sendmessage"))
        .map(|r| serde_json::from_slice::<Sent>(&r.body).unwrap())
        .filter(|r| r.message_thread_id == Some(9))
        .collect::<Vec<_>>();
    assert_eq!(menus.len(), 1);
    assert_eq!(menus[0].parse_mode.as_deref(), Some("MarkdownV2"));
    assert!(!menus[0].reply_markup.inline_keyboard.is_empty());
    let edits = requests
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/editmessagetext")
        })
        .map(|r| serde_json::from_slice::<Sent>(&r.body).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|r| r.message_id == Some(700) && !r.reply_markup.inline_keyboard.is_empty())
    );
    fixture.choose(213, "0").await;
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("工作区")
    );
}

#[tokio::test]
async fn workspace_and_session_buttons_only_contain_actual_entries() {
    let fixture = Fixture::new().await;
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file).unwrap();
    for id in ["second", "third"] {
        text.push_str(&format!("\n[[workspaces]]\nid='{id}'\nprovider_id='instance'\nname='{id}'\nshort_name='{id}'\ncwd='/workspace/{id}'\n"));
    }
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&input(215, 0, "/start"))
        .await
        .unwrap();
    fixture.choose(216, "1").await;
    let menu = fixture.outputs().await.into_iter().last().unwrap();
    let choices = menu
        .buttons
        .iter()
        .flatten()
        .filter(|b| {
            b.data
                .rsplit(':')
                .next()
                .is_some_and(|n| n.parse::<usize>().is_ok())
        })
        .count();
    assert_eq!(choices, 3);
    assert!(menu.buttons.iter().filter(|row| row.len() == 2).count() >= 1);
    assert!(!menu.text.contains("*1*"));
    fixture
        .service
        .handle(&input(217, 9, "/sessions workspace"))
        .await
        .unwrap();
    let menu = fixture.outputs().await.into_iter().last().unwrap();
    let choices = menu
        .buttons
        .iter()
        .flatten()
        .filter(|b| {
            b.data
                .rsplit(':')
                .next()
                .is_some_and(|n| n.parse::<usize>().is_ok())
        })
        .count();
    assert_eq!(choices, 2);
}

#[tokio::test]
async fn menus_do_not_depend_on_editing_deleted_management_messages() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(350, 0, "/start"))
        .await
        .unwrap();
    fixture.choose(351, "1").await;
    Mock::given(method("POST")).and(path_regex("(?i).*/editmessagetext$"))
        .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"ok":false,"error_code":400,"description":"Bad Request: message to edit not found"}"#)).with_priority(1).mount(&fixture.telegram).await;
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().deliver(receiver));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let requests = fixture.telegram.received_requests().await.unwrap();
        let sends = requests
            .iter()
            .filter(|r| r.url.path().to_ascii_lowercase().ends_with("/sendmessage"))
            .count();
        let edits = requests
            .iter()
            .filter(|r| {
                r.url
                    .path()
                    .to_ascii_lowercase()
                    .ends_with("/editmessagetext")
            })
            .count();
        if edits == 1 && sends == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "known missing menu did not fall back to one new send"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(true).unwrap();
    worker.await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM navigation WHERE published_at IS NOT NULL"
        )
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn management_adds_and_removes_catalog_entries_without_agent_operations() {
    let fixture = Fixture::new().await;
    let catalog = engine::catalog::Catalog::new(fixture.service.store.pool().clone());
    let initial = fixture.service.config.snapshot();
    catalog
        .save_provider(&initial.config.providers[0])
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(900, 0, "/start"))
        .await
        .unwrap();
    fixture.choose(901, "1").await;
    fixture
        .service
        .handle(&input(902, 9, "/add /workspace/new dynamic"))
        .await
        .unwrap();
    let snapshot = fixture.service.config.snapshot();
    assert!(
        snapshot
            .config
            .workspaces
            .iter()
            .any(|w| w.short_name == "dynamic" && w.cwd == "/workspace/new")
    );
    fixture
        .service
        .handle(&input(903, 9, "/remove dynamic"))
        .await
        .unwrap();
    assert!(
        fixture
            .service
            .config
            .snapshot()
            .config
            .workspaces
            .is_empty()
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn each_off_topic_start_has_a_visible_reply_even_when_the_management_menu_exists() {
    let fixture = Fixture::new().await;
    let (stop, receiver) = watch::channel(false);
    let worker = tokio::spawn(fixture.service.clone().deliver(receiver));

    async fn drain(fixture: &Fixture) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM deliveries WHERE state IN ('pending','dispatching')",
            )
            .fetch_one(&fixture.service.store.pool)
            .await
            .unwrap();
            if pending == 0 {
                return;
            }
            assert!(Instant::now() < deadline, "fixture delivery did not settle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fixture
        .service
        .handle(&input(1000, 0, "/start"))
        .await
        .unwrap();
    drain(&fixture).await;
    fixture
        .service
        .handle(&input(1001, 0, "/start"))
        .await
        .unwrap();
    drain(&fixture).await;
    stop.send(true).unwrap();
    worker.await.unwrap();

    #[derive(Deserialize)]
    struct Sent {
        text: String,
        message_thread_id: Option<i32>,
    }

    let requests = fixture.telegram.received_requests().await.unwrap();
    let root_replies = requests
        .iter()
        .filter(|r| {
            if !r.url.path().to_ascii_lowercase().ends_with("/sendmessage") {
                return false;
            }
            let sent: Sent = serde_json::from_slice(&r.body).unwrap();
            sent.message_thread_id.is_none() && sent.text.contains("管理")
        })
        .count();
    assert_eq!(
        root_replies, 2,
        "each distinct /start must visibly answer in its origin"
    );
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
    assert_eq!(
        requests
            .iter()
            .filter(|r| {
                if !r.url.path().to_ascii_lowercase().ends_with("/sendmessage") {
                    return false;
                }
                let sent: Sent = serde_json::from_slice(&r.body).unwrap();
                sent.message_thread_id == Some(9)
            })
            .count(),
        2,
        "each explicit start restores a visible management menu"
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn off_topic_numbers_use_the_management_navigation_and_task_text_stays_rejected() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1100, 0, "/start"))
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM navigation WHERE topic_id=9")
        .fetch_one(&fixture.service.store.pool)
        .await
        .unwrap();
    fixture
        .service
        .store
        .navigation_receipt(&id, 777, now())
        .await
        .unwrap();
    fixture.service.handle(&input(1101, 0, "1")).await.unwrap();
    let menu = fixture
        .outputs()
        .await
        .into_iter()
        .rev()
        .find(|b| b.navigation_id.is_some())
        .unwrap();
    assert!(
        menu.text.contains("工作区"),
        "Off-topic selection must advance the shared management menu"
    );
    fixture
        .service
        .handle(&input(1102, 0, "do not send this task"))
        .await
        .unwrap();
    let last = fixture.outputs().await.into_iter().last().unwrap();
    assert!(last.text.contains("这里只接受"));
    assert!(!last.text.contains("do not send this task"));
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn explicit_start_replaces_only_a_definitively_missing_management_topic() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1300, 0, "/start"))
        .await
        .unwrap();
    Mock::given(method("POST")).and(path_regex("(?i).*/editforumtopic$"))
        .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"ok":false,"error_code":400,"description":"Bad Request: message thread not found"}"#))
        .with_priority(1).mount(&fixture.telegram).await;
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/createforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"ok":true,"result":{"message_thread_id":19,"name":"管理","icon_color":7322096}}"#,
        ))
        .with_priority(1)
        .mount(&fixture.telegram)
        .await;
    fixture
        .service
        .handle(&input(1301, 0, "/start"))
        .await
        .unwrap();
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(19)
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn english_and_chinese_preferences_are_persistent_and_peer_scoped() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1500, 0, "/language en"))
        .await
        .unwrap();
    let english = fixture.outputs().await.into_iter().last().unwrap();
    assert!(english.text.contains("Hosts"));
    assert!(!english.text.contains("主机"));
    let mut next = input(1501, 0, "/start");
    next.locale = Some("zh-CN".into());
    fixture.service.handle(&next).await.unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("Hosts")
    );
    let before = fixture.outputs().await.len();
    let mut other = input(1502, 0, "/start");
    other.user = 20;
    other.chat = 20;
    fixture.service.handle(&other).await.unwrap();
    assert_eq!(fixture.outputs().await.len(), before);
    assert_eq!(
        fixture
            .service
            .store
            .locale(1, 10)
            .await
            .unwrap()
            .as_deref(),
        Some("en")
    );
    assert_eq!(fixture.service.store.locale(1, 20).await.unwrap(), None);
}

#[tokio::test]
async fn new_session_selects_host_and_workspace_ids_when_names_and_paths_match() {
    let fixture = Fixture::new().await;
    let config = fixture.service.config.snapshot().config.clone();
    let endpoint = config.hosts[0].endpoint.as_deref().unwrap();
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file)
        .unwrap()
        .replace("Fixture workspace", "sample");
    text.push_str(&format!(
        r#"
[[hosts]]
id="other"
name="Other host"
transport="websocket"
endpoint="{endpoint}"
allowed_workspace_roots=["/workspace"]
[[providers]]
id="other-instance"
host_id="other"
kind="codex"
connection_mode="existing_proxy"
compatibility_profile="fixture"
control_verified=true
[[workspaces]]
id="other-workspace"
provider_id="other-instance"
name="sample"
short_name="OTHER"
cwd="/workspace"
"#
    ));
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    assert!(matches!(
        fixture
            .service
            .handle(&input(2110, 9, "/new sample ambiguous"))
            .await,
        Err(Error::AmbiguousWorkspace)
    ));
    fixture
        .service
        .handle(&input(2111, 9, "/new other/sample Qualified title"))
        .await
        .unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("Other host")
    );
    fixture
        .service
        .handle(&input(2112, 9, "/new"))
        .await
        .unwrap();
    fixture.choose(2113, "2").await;
    fixture.choose(2114, "1").await;
    let prompt = fixture.outputs().await.pop().unwrap();
    assert!(
        prompt.text.contains("Other host")
            && prompt.text.contains("sample")
            && prompt.text.contains("请发送会话标题")
    );
    fixture.deliver_pending().await;
    fixture
        .service
        .handle(&input(2115, 9, "123"))
        .await
        .unwrap();
    let confirmation = fixture.outputs().await.pop().unwrap();
    assert!(confirmation.text.contains("Other host") && confirmation.text.contains("123"));
    assert!(
        fixture.state.lock().await.requests.is_empty(),
        "choosing a host, workspace and title must not contact the agent"
    );
    fixture.choose(2116, "1").await;
    let provider: String =
        sqlx::query_scalar("SELECT instance_id FROM operations WHERE source_key LIKE '%:create'")
            .fetch_one(&fixture.service.store.pool)
            .await
            .unwrap();
    assert_eq!(provider, "other-instance");
    let state = fixture.state.lock().await;
    assert!(state.requests.contains(&"thread/start"));
    assert!(!state.requests.contains(&"turn/start"));
    assert_eq!(state.native_name.as_deref(), Some("123"));
}

#[tokio::test]
async fn help_is_topic_aware_refresh_is_read_only_and_removed_commands_do_not_detach() {
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
    fixture
        .service
        .handle(&input(2120, 11, "/help"))
        .await
        .unwrap();
    let session_help = fixture.outputs().await.pop().unwrap();
    assert!(session_help.text.contains("上次用户输入与最终结果"));
    fixture
        .service
        .handle(&input(2121, 11, "/unbind"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(2122, 11, "/cleartopics"))
        .await
        .unwrap();
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
            .contains("未知命令")
    );
    fixture
        .service
        .handle(&input(2123, 9, "/help"))
        .await
        .unwrap();
    let management_help = fixture.outputs().await.pop().unwrap();
    assert!(management_help.text.contains("重新获取当前管理页"));
    assert!(
        !management_help.text.contains("/unbind") && !management_help.text.contains("/cleartopics")
    );
    fixture
        .service
        .handle(&input(2124, 9, "/new"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(2125, 9, "/refresh"))
        .await
        .unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("选择新会话所在的主机")
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn host_workspace_menu_pages_all_registered_projects_including_paged_project() {
    let fixture = Fixture::new().await;
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file).unwrap();
    for index in 0..8 {
        text.push_str(&format!("\n[[workspaces]]\nid='extra-{index}'\nprovider_id='instance'\nname='project-{index}'\nshort_name='P{index}'\ncwd='/workspace/project-{index}'\n"));
    }
    text.push_str("\n[[workspaces]]\nid='paged'\nprovider_id='instance'\nname='paged-project'\nshort_name='PAGED'\ncwd='/workspace/paged-project'\n");
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&input(2140, 0, "/start"))
        .await
        .unwrap();
    fixture.choose(2141, "1").await;
    let first = fixture.outputs().await.pop().unwrap();
    assert!(
        !first.text.contains("tg.workspace.page"),
        "workspace page translation ID leaked into the menu"
    );

    assert!(
        menu_labels(&first).contains("下一页"),
        "registered workspaces beyond a page need a next-page choice"
    );
    fixture.choose(2142, "9").await;
    let next = fixture.outputs().await.pop().unwrap();
    assert!(!menu_labels(&next).contains("下一页"));
    assert!(
        menu_labels(&first).contains("paged-project")
            || menu_labels(&next).contains("paged-project")
    );
    fixture
        .service
        .handle(&input(2143, 9, "/new"))
        .await
        .unwrap();
    fixture.choose(2144, "1").await;
    assert!(menu_labels(fixture.outputs().await.last().unwrap()).contains("下一页"));
    fixture.choose(2145, "9").await;
    fixture.choose(2146, "1").await;
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("请发送会话标题")
    );
    assert!(
        fixture.state.lock().await.requests.is_empty(),
        "paging must not start or load an agent session"
    );
}

#[tokio::test]
async fn an_untitled_thread_outside_the_selected_workspace_is_not_renamed_or_loaded() {
    let fixture = Fixture::new().await;
    fixture.state.lock().await.native_name = None;
    let file = &fixture.service.config.path;
    let text = std::fs::read_to_string(file)
        .unwrap()
        .replace("cwd=\"/workspace\"", "cwd=\"/workspace/other\"");
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    assert!(
        fixture
            .service
            .handle(&input(1800, 0, "/open workspace thread"))
            .await
            .is_err()
    );
    let state = fixture.state.lock().await;
    assert!(state.native_name.is_none());
    assert!(!state.requests.iter().any(|method| matches!(
        *method,
        "thread/name/set" | "thread/start" | "thread/resume" | "turn/start" | "turn/steer"
    )));
}

#[tokio::test]
async fn workspace_pages_use_recency_before_name_in_both_navigation_flows() {
    let fixture = Fixture::new().await;
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file).unwrap();
    text.push_str("\n[[workspaces]]\nid='newer'\nprovider_id='instance'\nname='zzz recent'\nshort_name='RECENT'\ncwd='/workspace/recent'\nupdated_at=500\n");
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&input(2260, 0, "/start"))
        .await
        .unwrap();
    fixture.choose(2261, "1").await;
    let text = menu_labels(&fixture.outputs().await.pop().unwrap());
    assert!(text.find("zzz recent").unwrap() < text.find("Fixture workspace").unwrap());
    fixture
        .service
        .handle(&input(2262, 9, "/new"))
        .await
        .unwrap();
    fixture.choose(2263, "1").await;
    let text = menu_labels(&fixture.outputs().await.pop().unwrap());
    assert!(text.find("zzz recent").unwrap() < text.find("Fixture workspace").unwrap());
}

#[tokio::test]
async fn menu_buttons_cannot_authorize_a_changed_workspace_scope_or_another_topic() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(5020, 9, "/new workspace Synthetic title"))
        .await
        .unwrap();
    let menu = fixture.outputs().await.pop().unwrap();
    fixture
        .service
        .store
        .navigation_receipt(menu.navigation_id.as_ref().unwrap(), 700, now())
        .await
        .unwrap();
    let mut wrong_topic = navigation_click(&menu, 5021, "1", 700);
    wrong_topic.topic = 11;
    fixture.service.handle(&wrong_topic).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let mut wrong_message = navigation_click(&menu, 5022, "1", 701);
    fixture.service.handle(&wrong_message).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let current = fixture.outputs().await.pop().unwrap();
    fixture
        .service
        .store
        .navigation_receipt(current.navigation_id.as_ref().unwrap(), 700, now())
        .await
        .unwrap();
    let file = &fixture.service.config.path;
    let text = std::fs::read_to_string(file)
        .unwrap()
        .replace("cwd=\"/workspace\"", "cwd=\"/workspace/changed\"");
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&navigation_click(&current, 5023, "1", 700))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("请发送会话标题")
    );
    wrong_message.user = 20;
    wrong_message.chat = 20;
    fixture.service.handle(&wrong_message).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn host_pages_are_bounded_in_both_browsing_and_creation_without_agent_requests() {
    let fixture = Fixture::new().await;
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file).unwrap();
    for index in 0..9 {
        text.push_str(&format!("\n[[hosts]]\nid='host-{index}'\nname='ZZ host {index:02}'\ntransport='local_proxy'\nallowed_workspace_roots=['/workspace']\n"));
    }
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&input(2400, 0, "/start"))
        .await
        .unwrap();
    let first = fixture.outputs().await.pop().unwrap();
    assert!(menu_labels(&first).contains("下一页"));
    assert!(!menu_labels(&first).contains("ZZ host 08"));
    fixture.choose(2401, "9").await;
    let second = fixture.outputs().await.pop().unwrap();
    assert!(menu_labels(&second).contains("ZZ host 08"));
    assert!(!menu_labels(&second).contains("下一页"));
    fixture
        .service
        .handle(&input(2402, 9, "/new"))
        .await
        .unwrap();
    fixture.choose(2403, "9").await;
    let second = fixture.outputs().await.pop().unwrap();
    assert!(menu_labels(&second).contains("ZZ host 08"));
    assert!(!menu_labels(&second).contains("下一页"));
    assert!(fixture.state.lock().await.requests.is_empty());
}
