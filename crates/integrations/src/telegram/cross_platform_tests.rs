//! Service behavior using the shared in-process fixtures.
use super::*;

#[tokio::test]
async fn discord_prototype_shares_the_telegram_connection_actor_and_write_receipts() {
    use discord::{Backend, Command as DCommand, Input as DInput};
    let (fixture, _http, api, service) = discord_fixture().await;
    let make = |id, channel, command| DInput {
        uploads: Vec::new(),
        locale: None,
        id,
        bot: 1,
        guild: 2,
        channel,
        user: 10,
        date: now(),
        command,
    };
    let pick = |screen: &discord::ui::Screen| {
        let payload = serde_json::to_value(&screen.components).unwrap();
        let select = &payload[0]["components"][0];
        assert_eq!(select["type"], 3);
        (
            select["custom_id"]
                .as_str()
                .unwrap()
                .strip_prefix("pp:")
                .unwrap()
                .to_string(),
            select["options"][0]["value"].as_str().unwrap().to_string(),
        )
    };
    let workspaces = service
        .menu(&api, make(100, 7, DCommand::Workspaces), None, None)
        .await
        .unwrap();
    let (token, value) = pick(&workspaces);
    let projects = service
        .menu(
            &api,
            make(101, 7, DCommand::Workspaces),
            Some(token),
            Some(value),
        )
        .await
        .unwrap();
    let (token, value) = pick(&projects);
    let sessions = service
        .menu(
            &api,
            make(102, 7, DCommand::Workspaces),
            Some(token),
            Some(value),
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(&sessions.components).unwrap();
    assert_eq!(
        wire[0]["components"][0]["options"][0]["label"],
        "Fixture title"
    );
    assert!(
        wire[0]["components"][0]["options"][0]["description"]
            .as_str()
            .unwrap()
            .contains("运行中")
    );
    assert!(
        wire[0]["components"][0]["options"][0]["emoji"]["name"]
            .as_str()
            .unwrap()
            .contains("🔵")
    );
    assert!(!sessions.text.contains("thread") && !sessions.text.contains("cursor"));
    let (token, value) = pick(&sessions);
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
    let opened = service
        .menu(
            &api,
            make(103, 7, DCommand::Workspaces),
            Some(token.clone()),
            Some(value.clone()),
        )
        .await
        .unwrap();
    assert!(opened.text.contains("<#11>"));
    let buttons = serde_json::to_value(&opened.components).unwrap();
    assert_eq!(
        buttons[0]["components"][0]["url"],
        "https://discord.com/channels/2/11"
    );
    assert!(
        service
            .menu(
                &api,
                make(104, 7, DCommand::Workspaces),
                Some(token),
                Some(value)
            )
            .await
            .is_err()
    );
    let status = service
        .handle(&api, make(105, 11, DCommand::Status))
        .await
        .unwrap();
    assert!(status.contains("Fixture title"));
    assert!(!status.contains("thread") && !status.contains("Thread"));
    let history = discord::storage::Store::new(fixture.service.store.core().clone())
        .discord_pending(1, 2, now())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].payload.contains("original question"));
    assert!(!history[0].payload.contains("must never appear"));
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
    let ask = make(
        2,
        11,
        DCommand::Ask {
            text: "continue".into(),
        },
    );
    service.handle(&api, ask.clone()).await.unwrap();
    service.handle(&api, ask).await.unwrap();
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
    let mut denied = make(
        3,
        11,
        DCommand::Ask {
            text: "unauthorized".into(),
        },
    );
    denied.user = 20;
    assert!(service.handle(&api, denied).await.is_err());
    fixture.state.lock().await.drop_write = true;
    let lost = make(
        4,
        11,
        DCommand::Ask {
            text: "lost receipt".into(),
        },
    );
    assert!(service.handle(&api, lost.clone()).await.is_err());
    assert!(service.handle(&api, lost).await.is_err());
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        2
    );
    let confirmation = service
        .menu(&api, make(5, 11, DCommand::Unbind), None, None)
        .await
        .unwrap();
    #[derive(Deserialize)]
    struct Button {
        custom_id: Option<String>,
    }
    #[derive(Deserialize)]
    struct Row {
        components: Vec<Button>,
    }
    let rows: Vec<Row> =
        serde_json::from_str(&serde_json::to_string(&confirmation.components).unwrap()).unwrap();
    let token = rows[0].components[0]
        .custom_id
        .as_ref()
        .unwrap()
        .strip_prefix("pp:")
        .unwrap();
    service
        .menu(
            &api,
            make(500, 11, DCommand::Workspaces),
            Some(token.into()),
            None,
        )
        .await
        .unwrap();
    assert!(
        service
            .handle(
                &api,
                make(
                    6,
                    11,
                    DCommand::Ask {
                        text: "unbound".into()
                    }
                )
            )
            .await
            .is_err()
    );
    assert!(matches!(
        fixture.state.lock().await.status,
        p::ThreadStatus::Active(_)
    ));
}

#[tokio::test]
async fn discord_formats_output_as_messages_and_disables_mentions_on_edits() {
    let http = MockServer::start().await;
    let response = serde_json::json!({
        "id":"12", "channel_id":"11",
        "author":{"id":"1", "username":"fixture", "discriminator":"0000", "avatar":null},
        "content":"fixture", "timestamp":"2026-01-01T00:00:00Z", "edited_timestamp":null,
        "tts":false, "mention_everyone":false, "mentions":[], "mention_roles":[],
        "attachments":[], "embeds":[], "pinned":false, "type":0
    });
    Mock::given(path_regex(".*/channels/11/messages(/12)?$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(3)
        .mount(&http)
        .await;
    let api = discord::Api::local_fixture(&http.uri(), 2, 1);
    api.send_in(11, "**Bold** @everyone <@123>", "en")
        .await
        .unwrap();
    api.card(11, 12, "**title** <#123>").await.unwrap();
    let source = "````text\nORIGINAL_三反引号_👩‍💻```\n````";
    api.send_in(11, source, "en").await.unwrap();
    let requests = http.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let edited: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(
        sent["components"][0]["components"][0]["content"],
        "**Bold** @everyone \\<@123\\>"
    );
    assert_eq!(
        edited["components"][0]["components"][0]["content"],
        "\\*\\*title\\*\\* \\<\\#123\\>"
    );
    for request in [sent, edited] {
        assert_eq!(request["allowed_mentions"]["parse"], serde_json::json!([]));
        assert_eq!(request["allowed_mentions"]["replied_user"], false);
    }
    let third: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(
        third["components"][0]["components"][0]["content"]
            .as_str()
            .unwrap()
            .contains("三反引号")
    );
    assert!(
        third
            .get("attachments")
            .is_none_or(|a| a.as_array().is_some_and(|a| a.is_empty()))
    );
    assert!(!String::from_utf8_lossy(&requests[2].body).contains("session.md"));
}

#[tokio::test]
async fn slack_threads_share_native_sessions_and_never_replay_unknown_input() {
    use slack::{
        api::Platform,
        model::{Command as SCommand, Content, Request as SRequest},
    };

    struct SlackFixture;
    #[async_trait::async_trait]
    impl Platform for SlackFixture {
        async fn channel(&self, _: &str) -> slack::Result<()> {
            Ok(())
        }

        async fn post(&self, _: &str, _: Option<&str>, _: &Content) -> slack::Result<String> {
            Ok("123.000001".into())
        }

        async fn update(&self, _: &str, _: &str, _: &Content) -> slack::Result<()> {
            Ok(())
        }
    }
    let fixture = Fixture::new().await;
    let core = fixture.service.store.core().clone();
    let settings = slack::config::Config {
        app_token: "xapp-fixture".into(),
        proxy: Default::default(),
        bot_token: "xoxb-fixture".into(),
        app_id: "A1".into(),
        team_id: "T1".into(),
        owner_user_id: "U1".into(),
    };
    let context = crate::Context {
        config: fixture.service.config.clone(),
        store: core.clone(),
        runtimes: fixture.service.runtimes.clone(),
    };
    let (_stop, receiver) = watch::channel(false);
    let service = slack::runtime::Service::local_fixture(
        settings,
        context,
        Arc::new(SlackFixture),
        "U2".into(),
        "D1".into(),
        receiver,
    );
    let open = SRequest {
        uploads: Vec::new(),
        menu_message: None,
        id: "open".into(),
        channel: "D1".into(),
        user: "U1".into(),
        root: None,
        binding: None,
        revision: None,
        date: now(),
        command: SCommand::Open {
            workspace: "workspace".into(),
            thread: "thread".into(),
        },
    };
    service.handle_fixture(&open).await.unwrap();
    let full_reads = fixture.state.lock().await.full_history_reads;
    let mut reopened = open.clone();
    reopened.id = "reopen".into();
    service.handle_fixture(&reopened).await.unwrap();
    assert_eq!(
        fixture.state.lock().await.full_history_reads,
        full_reads,
        "reopening initialized Slack threads must not fetch full history again"
    );
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
    let store = slack::storage::Store::new(core, "A1", "T1", "U2");
    let b = store.at("D1", "123.000001").await.unwrap().unwrap();
    let r = SRequest {
        uploads: Vec::new(),
        menu_message: None,
        id: "ask".into(),
        channel: b.channel.clone(),
        user: "U1".into(),
        root: Some(b.root.clone()),
        binding: Some(b.id.clone()),
        revision: Some(b.revision),
        date: now(),
        command: SCommand::Input {
            text: "continue from Slack".into(),
        },
    };
    service.handle_fixture(&r).await.unwrap();
    service.handle_fixture(&r).await.unwrap();
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
    let mut denied = r.clone();
    denied.id = "denied".into();
    denied.user = "U3".into();
    assert!(service.handle_fixture(&denied).await.is_err());
    fixture.state.lock().await.drop_write = true;
    let mut lost = r.clone();
    lost.id = "lost".into();
    assert!(service.handle_fixture(&lost).await.is_err());
    assert!(service.handle_fixture(&lost).await.is_err());
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        2
    );
    let mut unbind = r.clone();
    unbind.id = "unbind".into();
    let entry = crate::bindings::Manager::new(
        &store,
        crate::bindings::Scope::Slack {
            app: "A1".into(),
            team: "T1".into(),
            bot: "U2".into(),
            channel: "D1".into(),
            owner: "U1".into(),
        },
    )
    .entry(
        &fixture.service.config.snapshot().config,
        &crate::bindings::Target::Slack { id: b.id.clone() },
        "en",
    )
    .await
    .unwrap();
    unbind.command = SCommand::BindingChange {
        target: entry.record.target,
        version: entry.version,
        change: crate::bindings::Change::Detach,
    };
    service.handle_fixture(&unbind).await.unwrap();
    assert!(service.handle_fixture(&r).await.is_err());
    assert!(matches!(
        fixture.state.lock().await.status,
        p::ThreadStatus::Active(_)
    ));
}
