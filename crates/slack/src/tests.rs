struct NoNetwork;
#[async_trait::async_trait]
impl crate::api::Platform for NoNetwork {
    async fn channel(&self, _: &str) -> Result<()> {
        panic!("callback performed a network lookup")
    }

    async fn post(&self, _: &str, _: Option<&str>, _: Option<&str>, _: &Content) -> Result<String> {
        panic!("callback posted a message")
    }

    async fn update(&self, _: &str, _: &str, _: &Content) -> Result<()> {
        panic!("callback edited a message")
    }

    async fn prepare_file(&self, _: usize) -> Result<crate::api::Upload> {
        panic!("callback uploaded a file")
    }

    async fn upload_file(&self, _: &crate::api::Upload, _: Vec<u8>) -> Result<()> {
        panic!("callback uploaded a file")
    }

    async fn complete_file(&self, _: &crate::api::Upload, _: &str, _: Option<&str>) -> Result<()> {
        panic!("callback uploaded a file")
    }
}

use crate::{model::*, storage::*, *};

fn settings() -> config::Config {
    config::Config {
        app_token: "xapp-fixture".into(),
        bot_token: "xoxb-fixture".into(),
        app_id: "A1".into(),
        team_id: "T1".into(),
        owner_user_id: "U1".into(),
        allowed_channel_ids: vec!["C1".into()],
    }
}

fn request(id: &str) -> Request {
    Request {
        id: id.into(),
        channel: "C1".into(),
        user: "U1".into(),
        root: Some("123.000001".into()),
        binding: Some("binding".into()),
        revision: Some(1),
        date: engine::now(),
        command: Command::Input {
            text: "synthetic input".into(),
        },
    }
}

fn binding() -> Binding {
    Binding {
        id: "binding".into(),
        app: "A1".into(),
        team: "T1".into(),
        bot: "U2".into(),
        channel: "C1".into(),
        root: "123.000001".into(),
        owner: "U1".into(),
        workspace: "project".into(),
        instance: "instance".into(),
        thread: "thread".into(),
        cwd: "/project".into(),
        endpoint: "endpoint".into(),
        active: true,
        revision: 1,
        observed: "{}".into(),
        card: String::new(),
        history_ready: false,
    }
}

#[test]
fn credentials_and_mentions_are_not_exposed() {
    let config = settings();
    assert!(config.valid() && config.credentials_valid());
    assert!(!format!("{config:?}").contains("xoxb-fixture"));
    assert!(!config.permits("T1", "U3", "C1"));
    let output = render::message("<@U3> <!channel> & <https://example.com>".into());
    let wire = serde_json::to_string(&crate::api::Api::content(&output)).unwrap();
    assert!(wire.contains("plain_text"));
    assert!(wire.contains("&lt;!channel&gt;"));
    assert!(render::message("界".repeat(2801)).document);
}

#[test]
fn typed_socket_events_only_route_new_authorized_thread_text() {
    use slack_morphism::prelude::*;
    let make = |extra: &str, user: &str, root: bool| {
        let root = if root {
            r#", "thread_ts":"123.000001""#
        } else {
            ""
        };
        serde_json::from_str::<SlackPushEventCallback>(&format!(r#"{{"team_id":"T1","api_app_id":"A1","event_id":"Ev1","event_time":123,"event":{{"type":"message","ts":"123.000002","channel":"C1","user":"{user}","text":"hello"{root}{extra}}}}}"#)).unwrap()
    };
    assert!(socket::message(make("", "U1", true), &settings(), "U2").is_some());
    for event in [
        make("", "U3", true),
        make("", "U1", false),
        make(r#", "subtype":"message_changed""#, "U1", true),
        make(r#", "bot_id":"B1""#, "U1", true),
        make(r#", "hidden":true"#, "U1", true),
    ] {
        assert!(socket::message(event, &settings(), "U2").is_none());
    }
    let event = make(r#", "subtype":"file_share""#, "U1", true);
    assert!(socket::message(event, &settings(), "U2").is_some());
}

#[tokio::test]
async fn durable_intake_actions_and_unknown_delivery_survive_recovery_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let store = Store::new(core, "A1", "T1", "U2");
    let r = request("message-1");
    assert!(
        store
            .enqueue("C1:123.000002", &r, "scope", false)
            .await
            .unwrap()
    );
    let mut duplicate = r.clone();
    duplicate.id = "another-event-id".into();
    assert!(
        !store
            .enqueue("C1:123.000002", &duplicate, "scope", false)
            .await
            .unwrap()
    );
    assert!(store.state(&r.id, "pending", "processing").await.unwrap());
    let b = binding();
    store.reserve(&b).await.unwrap();
    store.bind(&b).await.unwrap();
    let nonce = store
        .action(&r, Action::Run(Command::Unbind), "scope")
        .await
        .unwrap();
    assert!(
        store
            .consume(&nonce, Some("C1"), "U3", "scope", None, None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .consume(&nonce, Some("C2"), "U1", "scope", None, None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .consume(&nonce, Some("C1"), "U1", "scope", None, None)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .consume(&nonce, Some("C1"), "U1", "scope", None, None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .inbox(&format!("action:{nonce}"))
            .await
            .unwrap()
            .is_some()
    );
    store
        .output(
            &r,
            &render::message("synthetic output".into()),
            "scope",
            false,
        )
        .await
        .unwrap();
    let outgoing = store.outgoing().await.unwrap();
    assert_eq!(outgoing.len(), 1);
    assert!(store.claim_output(&outgoing[0]).await.unwrap());
    store.recover().await.unwrap();
    assert_eq!(
        store.inbox(&r.id).await.unwrap().unwrap().state,
        "uncertain"
    );
    assert_eq!(
        store
            .inbox(&format!("action:{nonce}"))
            .await
            .unwrap()
            .unwrap()
            .state,
        "draft"
    );
    assert!(store.outgoing().await.unwrap().is_empty());
    assert!(store.reserve(&b).await.is_err());
    let changed = sqlx::query("UPDATE slack_bindings SET thread='another' WHERE id='binding'")
        .execute(store.core.pool())
        .await;
    assert!(changed.is_err());
}

#[tokio::test]
async fn observation_cursor_and_outbox_commit_together_and_channel_threads_share_budget() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        engine::storage::Store::open(dir.path()).await.unwrap(),
        "A1",
        "T1",
        "U2",
    );
    let b = binding();
    store.reserve(&b).await.unwrap();
    store.bind(&b).await.unwrap();
    store
        .publish(
            &b,
            &render::message("answer".into()),
            "{\"i\":\"hash\"}",
            "scope",
        )
        .await
        .unwrap();
    store
        .publish(
            &b,
            &render::message("answer".into()),
            "{\"i\":\"hash\"}",
            "scope",
        )
        .await
        .unwrap();
    assert_eq!(store.outgoing().await.unwrap().len(), 1);
    let mut r = request("second-thread");
    r.root = Some("124.000001".into());
    r.binding = None;
    store
        .output(
            &r,
            &render::message("another thread".into()),
            "scope",
            false,
        )
        .await
        .unwrap();
    let first = store.outgoing().await.unwrap();
    assert_eq!(first.len(), 1);
    assert!(store.claim_output(&first[0]).await.unwrap());
    store
        .finish_output(&first[0].id, "delivered", 0)
        .await
        .unwrap();
    let next = store.outgoing().await.unwrap();
    assert_eq!(next.len(), 1);
    assert!(!store.claim_output(&next[0]).await.unwrap());
}

#[tokio::test]
async fn sdk_socket_ack_follows_durable_intake_and_replayed_envelopes_do_not_duplicate_jobs() {
    use futures_util::{SinkExt, StreamExt};
    use slack_morphism::prelude::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pocketpilot.toml");
    std::fs::write(&config_path, "[service]\ndata_dir='state'\n").unwrap();
    let config = Arc::new(
        engine::reload::ConfigFile::open(config_path, |p, t| {
            let mut config: engine::config::Config =
                toml::from_str(t).map_err(|_| engine::config::ConfigError::Parse)?;
            config.prepare(p, t)?;
            Ok(engine::reload::ParsedConfig {
                config,
                restart_key: String::new(),
            })
        })
        .unwrap(),
    );
    let core = engine::storage::Store::open(&config.snapshot().config.service.data_dir)
        .await
        .unwrap();
    let context = engine::integrations::Context {
        config,
        store: core.clone(),
        connections: Arc::new(engine::connections::Connections::new(core.clone())),
    };
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let service = Arc::new(runtime::Service {
        settings: settings(),
        context,
        store: Store::new(core, "A1", "T1", "U2"),
        api: Arc::new(NoNetwork),
        bot: "U2".into(),
        stop: receiver.clone(),
        cutoff: AtomicI64::new(engine::now()),
        verified: AtomicBool::new(false),
    });
    let http = MockServer::start().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let response = format!(r#"{{"ok":true,"url":"{url}"}}"#);
    Mock::given(method("POST"))
        .and(path("/api/apps.connections.open"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(response, "application/json"))
        .mount(&http)
        .await;
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .unwrap()
        .https_or_http()
        .enable_http1()
        .build();
    let client = Arc::new(SlackClient::new(
        SlackClientHyperConnector::with_connector(connector)
            .with_slack_api_url(&format!("{}/api", http.uri())),
    ));
    let api = Arc::new(crate::api::Api {
        client,
        token: SlackApiToken::new("xoxb-fixture".into()).with_team_id("T1".into()),
    });
    let task = tokio::spawn(socket::run(service.clone(), api, receiver));
    let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    socket.send(tokio_tungstenite::tungstenite::Message::Text(r#"{"type":"hello","connection_info":{"app_id":"A1"},"num_connections":1,"debug_info":{"host":"fixture"}}"#.into())).await.unwrap();
    for envelope in ["envelope-1", "envelope-2"] {
        let frame = format!(
            r#"{{"type":"slash_commands","envelope_id":"{envelope}","accepts_response_payload":true,"payload":{{"team_id":"T1","channel_id":"C1","user_id":"U1","command":"/pp","response_url":"https://example.invalid/response","trigger_id":"trigger-one"}}}}"#
        );
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
            .await
            .unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = reply.to_text().unwrap();
        assert!(text.contains(envelope));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_inbox WHERE namespace=?")
            .bind(&service.store.namespace)
            .fetch_one(service.store.core.pool())
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "ACK was emitted before durable intake or duplicate source was accepted twice"
        );
    }
    stop.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn read_controls_are_reusable_while_replayed_clicks_and_rate_limits_remain_durable() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        engine::storage::Store::open(dir.path()).await.unwrap(),
        "A1",
        "T1",
        "U2",
    );
    let r = request("status");
    let nonce = store
        .action(&r, Action::Run(Command::Status), "scope")
        .await
        .unwrap();
    for click in ["click-one", "click-one", "click-two"] {
        assert!(
            store
                .consume(&nonce, Some("C1"), "U1", "scope", None, Some(click))
                .await
                .unwrap()
                .is_some()
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_inbox")
        .fetch_one(store.core.pool())
        .await
        .unwrap();
    assert_eq!(count, 2);
    let mut first = r.clone();
    first.id = "first".into();
    first.binding = None;
    let mut second = first.clone();
    second.id = "second".into();
    second.channel = "C2".into();
    store
        .output(&first, &render::message("one".into()), "scope", false)
        .await
        .unwrap();
    store
        .output(&second, &render::message("two".into()), "scope", false)
        .await
        .unwrap();
    let cached = store.outgoing().await.unwrap();
    assert_eq!(cached.len(), 2);
    assert!(store.claim_output(&cached[0]).await.unwrap());
    store.rate_limit(&cached[0].id, 60).await.unwrap();
    assert!(store.outgoing().await.unwrap().is_empty());
    assert!(!store.claim_output(&cached[1]).await.unwrap());
}

#[tokio::test]
async fn recovery_waits_for_socket_readiness_and_rearms_confirmation_after_reconnect() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, "[service]\ndata_dir='state'\n").unwrap();
    let config = Arc::new(
        engine::reload::ConfigFile::open(path, |path, text| {
            let mut config: engine::config::Config =
                toml::from_str(text).map_err(|_| engine::config::ConfigError::Parse)?;
            config.prepare(path, text)?;
            Ok(engine::reload::ParsedConfig {
                config,
                restart_key: String::new(),
            })
        })
        .unwrap(),
    );
    let core = engine::storage::Store::open(&config.snapshot().config.service.data_dir)
        .await
        .unwrap();
    let context = engine::integrations::Context {
        config,
        store: core.clone(),
        connections: Arc::new(engine::connections::Connections::new(core.clone())),
    };
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let service = Arc::new(runtime::Service {
        settings: settings(),
        context,
        store: Store::new(core, "A1", "T1", "U2"),
        api: Arc::new(NoNetwork),
        bot: "U2".into(),
        stop: receiver,
        cutoff: AtomicI64::new(engine::now()),
        verified: AtomicBool::new(false),
    });
    let b = binding();
    service.store.reserve(&b).await.unwrap();
    service.store.bind(&b).await.unwrap();
    let r = request("recovered");
    let hash = service.context.config.snapshot().hash.clone();
    service
        .store
        .enqueue("event-one", &r, &hash, true)
        .await
        .unwrap();
    let work = tokio::spawn(service.clone().work());
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        service.store.inbox(&r.id).await.unwrap().unwrap().state,
        "draft"
    );
    for _ in 0..2 {
        service.connected().await.unwrap();
        service.verified.store(true, Ordering::SeqCst);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if service.store.inbox(&r.id).await.unwrap().unwrap().state == "awaiting_confirmation" {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let available: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM slack_actions WHERE namespace=? AND consumed=0",
        )
        .bind(&service.store.namespace)
        .fetch_one(service.store.core.pool())
        .await
        .unwrap();
        assert_eq!(
            available, 1,
            "each live connection needs a usable fresh confirmation"
        );
        service.verified.store(false, Ordering::SeqCst);
    }
    stop.send(true).unwrap();
    work.await.unwrap().unwrap();
}
