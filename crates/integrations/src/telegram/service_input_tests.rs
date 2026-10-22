//! Service behavior using the shared in-process fixtures.
use super::*;

#[tokio::test]
async fn session_messages_are_quiet_and_own_inputs_are_not_echoed() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    fixture.service.refresh_binding(&binding).await.unwrap();
    let before = fixture.outputs().await;
    assert!(
        before
            .iter()
            .any(|b| b.text.contains("🤖 **Agent** · ✅\n\noriginal answer"))
    );
    assert!(
        before
            .iter()
            .any(|b| b.text.contains("👤 **用户**\n\noriginal question"))
    );
    assert!(before.iter().all(|b| !b.text.contains("thread：")
        && !b.text.contains("用户\n")
        && !b.text.contains("Agent\n")));
    fixture
        .service
        .handle(&input(330, 11, "do not echo this input"))
        .await
        .unwrap();
    assert_eq!(
        fixture.outputs().await.len(),
        before.len(),
        "successful input must not create an ACK message"
    );
    fixture.service.refresh_binding(&binding).await.unwrap();
    assert_eq!(
        fixture.outputs().await.len(),
        before.len(),
        "observed user echo must only advance its cursor"
    );
    fixture.state.lock().await.items.push(serde_json::from_str::<p::ThreadItem>(r#"{"type":"agentMessage","id":"new-answer","text":"Only this reply","phase":"final_answer"}"#).unwrap());
    fixture.service.refresh_binding(&binding).await.unwrap();
    let outputs = fixture.outputs().await;
    assert_eq!(
        outputs.last().unwrap().text,
        "🤖 **Agent** · ✅\n\nOnly this reply"
    );
    assert!(outputs.iter().all(|b| !b.text.contains("do not echo")
        && !b.text.contains("已追加")
        && !b.text.contains("已开始")));
    fixture
        .service
        .handle(&input(331, 11, "/info"))
        .await
        .unwrap();
    let info = fixture.outputs().await.into_iter().last().unwrap();
    assert!(info.text.contains("⚡"));
    assert!(!info.text.contains("观察时间") && !info.text.contains("thread："));
}

#[tokio::test]
async fn picker_hides_only_sessions_proven_to_be_local_operator_probes() {
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    let id = store
        .reserve_operation("probe:fixture:create", "instance", "__new")
        .await
        .unwrap();
    store
        .transition(
            &id,
            session::operation::OperationState::Received,
            session::operation::OperationState::Prepared,
        )
        .await
        .unwrap();
    store
        .transition(
            &id,
            session::operation::OperationState::Prepared,
            session::operation::OperationState::Dispatching,
        )
        .await
        .unwrap();
    store
        .accept_receipt(
            &id,
            &Receipt {
                thread: "thread".into(),
                turn: None,
            },
        )
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(220, 0, "/sessions workspace"))
        .await
        .unwrap();
    let menu = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.navigation_id.is_some() && b.text.contains("会话"))
        .unwrap();
    assert!(!menu.buttons.is_empty());
    assert!(!menu_labels(&menu).contains("Fixture title"));
    assert!(menu_labels(&menu).contains("新建会话"));
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
async fn a_message_older_than_the_new_process_is_a_draft_even_within_five_minutes() {
    let mut fixture = Fixture::new().await;
    let boot = now();
    Arc::get_mut(&mut fixture.service).unwrap().started_at = boot;
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
    let mut queued = input(80, 11, "offline before boot");
    queued.date = boot - 1;
    assert!((0..=300).contains(&now().saturating_sub(queued.date)));
    fixture.service.handle(&queued).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|b| b.text.contains("offline before boot"))
    );
}

#[tokio::test]
async fn a_steer_race_preserves_text_and_waits_for_confirmation_before_starting_another_turn() {
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
    fixture.state.lock().await.turn_changed_on_steer = true;
    fixture
        .service
        .handle(&input(94, 11, "preserve this race input"))
        .await
        .unwrap();
    {
        let state = fixture.state.lock().await;
        assert_eq!(
            state
                .requests
                .iter()
                .filter(|r| **r == "turn/steer")
                .count(),
            1
        );
        assert!(!state.requests.contains(&"turn/start"));
        assert!(!state.items.iter().any(|i|matches!(i,p::ThreadItem::UserMessage{content,..} if content.iter().any(|c|matches!(c,p::UserInput::TextUserInput{text,..} if text=="preserve this race input")))));
    }
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("preserve this race input"))
        .unwrap();
    assert!(draft.text.contains("尚未发送"));
    let mut confirm = input(95, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    fixture.service.handle(&confirm).await.unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/start")
            .count(),
        1
    );
}

#[tokio::test]
async fn new_session_requires_confirmation_and_an_unloaded_session_resumes_only_on_user_input() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1, 0, "/new workspace first task"))
        .await
        .unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    fixture.choose(2, "1").await;
    assert!(
        fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    {
        let state = fixture.state.lock().await;
        assert!(!state.requests.contains(&"turn/start"));
        assert!(state.requests.contains(&"thread/name/set"));
        assert_eq!(state.native_name.as_deref(), Some("first task"));
        assert!(state.items.is_empty());
    }
    fixture.state.lock().await.status = p::ThreadStatus::NotLoaded;
    fixture
        .service
        .handle(&input(3, 11, "/status"))
        .await
        .unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/resume")
    );
    fixture
        .service
        .handle(&input(4, 11, "resume task"))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "thread/resume")
            .count(),
        1
    );
}

#[tokio::test]
async fn waiting_approval_still_accepts_steer_without_answering_the_approval() {
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
    fixture.state.lock().await.status =
        p::ThreadStatus::Active(vec![p::ThreadActiveFlag::WaitingOnApproval]);
    fixture
        .service
        .handle(&input(1, 11, "/steer investigate another file meanwhile"))
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
    assert!(matches!(
        fixture.state.lock().await.status,
        p::ThreadStatus::Active(_)
    ));
}

#[tokio::test]
async fn display_and_rate_changes_preserve_authorized_live_outputs() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1, 0, "/open workspace thread"))
        .await
        .unwrap();
    let before = fixture.outputs().await;
    let history = before
        .iter()
        .find(|b| b.text.contains("🤖 **Agent** · ✅\n\noriginal answer"))
        .unwrap();
    let scope = history.scope.as_ref().unwrap();
    let path = fixture.service.config.path.clone();
    let contents = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Fixture workspace", "Renamed workspace")
        .replace(
            "telegram_messages_per_minute=60000",
            "telegram_messages_per_minute=100",
        );
    std::fs::write(path, contents).unwrap();
    fixture.service.config.reload().unwrap();
    assert!(
        fixture
            .service
            .scope_allowed(&fixture.service.config.snapshot().config, scope)
    );
    fixture
        .service
        .handle(&input(2, 11, "/rename new display name"))
        .await
        .unwrap();
    let binding = fixture
        .service
        .store
        .binding(1, 10, 11)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.binding_revision, Some(binding.revision));
}

#[tokio::test]
async fn legacy_attachment_inputs_without_file_references_require_reupload_without_starting_turns()
{
    let fixture = Fixture::new().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    let mut photo = input(2250, 11, "");
    photo.attachments = vec![session::Attachment {
        name: "image".into(),
    }];
    fixture.service.handle(&photo).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    assert!(
        fixture
            .outputs()
            .await
            .last()
            .unwrap()
            .text
            .contains("重新上传")
    );
    photo.update_id += 1;
    photo.event_key = "message:10:2251".into();
    photo.message_id += 1;
    photo.text = "caption only".into();
    fixture.service.handle(&photo).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn authorized_file_only_messages_are_downloaded_staged_and_submitted_to_the_selected_codex() {
    let fixture = Fixture::new().await;
    let path = &fixture.service.config.path;
    let config = std::fs::read_to_string(path).unwrap().replace("control_verified=true", "control_verified=true\nendpoint={binary='/fixture/codex',socket='/private/account/.codex/app-server-control/app-server-control.sock'}");
    std::fs::write(path, config).unwrap();
    fixture.service.config.reload().unwrap();
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
    Mock::given(path_regex("(?i).*/getfile$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":{"file_id":"private-file","file_unique_id":"fixture","file_size":12,"file_path":"documents/notes.txt"}})))
        .expect(1).mount(&fixture.telegram).await;
    Mock::given(path_regex(".*/file/bot.*/documents/notes.txt$"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fixture file".to_vec()))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    let mut file = input(2255, 11, "");
    file.uploads = vec![session::files::Incoming {
        name: "notes.txt".into(),
        location: "private-file".into(),
        image: false,
    }];
    fixture.service.handle(&file).await.unwrap();
    let state = fixture.state.lock().await;
    assert_eq!(state.uploads.len(), 1);
    assert_eq!(state.uploads[0].data_base64, "Zml4dHVyZSBmaWxl");
    assert!(state.uploads[0].path.0.ends_with("/0/notes.txt"));
    assert!(state.items.iter().any(|item| matches!(item, p::ThreadItem::UserMessage { content, .. } if content.iter().any(|input| matches!(input, p::UserInput::TextUserInput { text, .. } if text.contains("## notes.txt:") && text.contains("## My request:"))))));
    let writes = state
        .requests
        .iter()
        .filter(|r| matches!(**r, "fs/writeFile" | "turn/steer"))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(writes, ["fs/writeFile", "turn/steer"]);
    drop(state);
    file.user = 99;
    fixture.service.handle(&file).await.unwrap();
    assert_eq!(fixture.state.lock().await.uploads.len(), 1);
}
