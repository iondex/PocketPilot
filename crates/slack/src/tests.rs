pub(crate) struct NoNetwork;
#[async_trait::async_trait]
impl crate::api::Platform for NoNetwork {
    async fn channel(&self, _: &str) -> Result<()> {
        panic!("callback performed a network lookup")
    }

    async fn post(&self, _: &str, _: Option<&str>, _: &Content) -> Result<String> {
        panic!("callback posted a message")
    }

    async fn update(&self, _: &str, _: &str, _: &Content) -> Result<()> {
        panic!("callback edited a message")
    }
}

use crate::{model::*, storage::*, *};

fn settings() -> config::Config {
    config::Config {
        app_token: "xapp-fixture".into(),
        proxy: Default::default(),
        bot_token: "xoxb-fixture".into(),
        app_id: "A1".into(),
        team_id: "T1".into(),
        owner_user_id: "U1".into(),
    }
}

fn request(id: &str) -> Request {
    Request {
        uploads: Vec::new(),
        menu_message: None,
        id: id.into(),
        channel: "D1".into(),
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
        channel: "D1".into(),
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
    assert!(!config.permits("T1", "U3"));
    let output = render::message("<@U3> <!channel> & <https://example.com>".into());
    let wire = serde_json::to_string(&crate::api::Api::content(&output)).unwrap();
    assert!(wire.contains("plain_text"));
    assert!(wire.contains("&lt;!channel&gt;"));
    assert_eq!(
        render::parts(&render::message("界".repeat(2801)))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn markdown_uses_typed_rich_text_and_old_outboxes_remain_literal() {
    let source = "# 标题\n\n前文**中文。**后文 _italic_ ~~old~~ `a < b`\n\n> note\n\n3. first\n4. second\n\n```rust\nlet s = \"<&>\";\n```\n\n<@U3> <!channel> :custom: 👩‍💻 [docs](https://example.com)";
    let content = render::markdown(source.into());
    assert!(!content.document);
    assert_eq!(content.text, source);
    let wire = serde_json::to_value(crate::api::Api::content(&content)).unwrap();
    assert_eq!(wire["blocks"][0]["type"], "rich_text");
    let elements = wire["blocks"][0]["elements"].as_array().unwrap();
    assert!(
        elements
            .iter()
            .any(|e| e["type"] == "rich_text_preformatted")
    );
    assert!(elements.iter().any(|e| e["type"] == "rich_text_quote"));
    assert!(
        elements
            .iter()
            .any(|e| e["type"] == "rich_text_list" && e["offset"] == 2)
    );
    let serialized = serde_json::to_string(&wire["blocks"]).unwrap();
    assert!(serialized.contains("\"bold\":true"));
    assert!(serialized.contains("\"italic\":true"));
    assert!(serialized.contains("\"strike\":true"));
    assert!(serialized.contains("\"code\":true"));
    assert!(serialized.contains("\"type\":\"link\""));
    assert!(!serialized.contains("\"type\":\"user\""));
    assert!(!serialized.contains("\"type\":\"broadcast\""));
    assert!(serialized.contains("<@U3>"));
    assert!(serialized.contains(":custom: 👩‍💻"));
    assert!(wire["text"].as_str().unwrap().contains("&lt;!channel&gt;"));
    let old: Content = serde_json::from_str(r#"{"text":"**literal** <@U3>"}"#).unwrap();
    assert_eq!(old.format, session::markdown::Format::Plain);
}

#[test]
fn markdown_tables_and_limits_fall_back_without_losing_the_source() {
    let table = render::markdown("| Name | Value |\n|---|---|\n| Alpha | **bold** |".into());
    assert!(!table.document);
    let text = crate::api::Api::content(&table).text.unwrap();
    assert!(text.contains("Name: Alpha"));
    assert!(text.contains("Value: bold"));
    let source = "x\n\n".repeat(51);
    let content = render::markdown(source.clone());
    assert!(!content.document);
    assert!(render::parts(&content).unwrap().len() > 1);
    assert_eq!(content.text, source);
    assert!(
        render::parts(&render::markdown("😀".repeat(1401)))
            .unwrap()
            .len()
            > 1
    );
}

#[tokio::test]
async fn acknowledged_message_parts_survive_restart_without_replaying_the_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let store = Store::new(core, "A1", "T1", "U2");
    let r = request("long-output");
    let content = render::markdown("**text** 👩‍💻\n\n".repeat(500));
    let parts = render::parts(&content).unwrap();
    assert!(parts.len() > 2);
    assert!(
        parts
            .iter()
            .all(|p| !p.document && session::markdown::fits(&p.text, 2800))
    );
    store.output(&r, &content, "scope").await.unwrap();
    let first = store.outgoing().await.unwrap().remove(0);
    store.prepare_parts(&first, &parts).await.unwrap();
    assert!(store.claim_output(&first).await.unwrap());
    store.acknowledge_part(&first, true).await.unwrap();
    store.recover().await.unwrap();
    sqlx::query("UPDATE slack_outbox SET not_before=0")
        .execute(store.core.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE slack_channel_budget SET next_send=0")
        .execute(store.core.pool())
        .await
        .unwrap();
    let second = store.outgoing().await.unwrap().remove(0);
    assert_eq!(second.next_part, 1);
    let saved: Vec<Content> =
        serde_json::from_str(second.rendered_parts.as_ref().unwrap()).unwrap();
    assert_eq!(saved[1].text, parts[1].text);
    assert!(store.claim_output(&second).await.unwrap());
    store.recover().await.unwrap();
    assert!(store.outgoing().await.unwrap().is_empty());
}

#[tokio::test]
async fn markdown_post_and_update_keep_fallback_literal_and_unfurls_disabled() {
    use crate::api::Platform;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method, path},
    };
    let server = MockServer::start().await;
    for endpoint in ["chat.postMessage", "chat.update"] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{endpoint}")))
            .and(body_partial_json(serde_json::json!({
                "channel":"D1", "mrkdwn":false, "link_names":false,
                "text":"Bold &lt;@U3&gt;"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok":true, "channel":"D1", "ts":"123.000001",
                "message":{"ts":"123.000001", "text":"Bold <@U3>"}
            })))
            .expect(1)
            .mount(&server)
            .await;
    }
    let api = fixture_api(&server.uri());
    let content = render::markdown("**Bold** <@U3>".into());
    api.post("D1", None, &content).await.unwrap();
    api.update("D1", "123.000001", &content).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let post: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(post["unfurl_links"], false);
    assert_eq!(post["unfurl_media"], false);
    assert_eq!(post["parse"], "none");
    assert_eq!(post["blocks"][0]["type"], "rich_text");
}

#[tokio::test]
async fn modal_navigation_source_survives_persistence_and_does_not_retarget_session_controls() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        engine::storage::Store::open(dir.path()).await.unwrap(),
        "A1",
        "T1",
        "U2",
    );
    let mut menu = request("menu-modal");
    menu.binding = None;
    menu.revision = None;
    menu.root = None;
    let nonce = store
        .action(
            &menu,
            Action::NewModal {
                workspace: "project".into(),
            },
            "scope",
        )
        .await
        .unwrap();
    let (modal, _) = store
        .consume(
            &nonce,
            Some("D1"),
            "U1",
            "scope",
            None,
            InteractionSource {
                id: Some("modal"),
                message: Some("200.000001"),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let nonce = store
        .action(
            &modal,
            Action::NewSubmit {
                workspace: "project".into(),
            },
            "scope",
        )
        .await
        .unwrap();
    let (submitted, _) = store
        .consume(
            &nonce,
            None,
            "U1",
            "scope",
            Some("Session title"),
            InteractionSource::default(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(submitted.menu_message.as_deref(), Some("200.000001"));
    assert!(matches!(submitted.command, Command::New { .. }));
    let bound = request("bound-status");
    let nonce = store
        .action(&bound, Action::Run(Command::Status), "scope")
        .await
        .unwrap();
    let (status, _) = store
        .consume(
            &nonce,
            Some("D1"),
            "U1",
            "scope",
            None,
            InteractionSource {
                id: Some("status"),
                message: Some("200.000001"),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(status.menu_message.is_none());
    assert_eq!(status.root, bound.root);
}

#[tokio::test]
async fn one_owner_dm_can_bind_sessions_from_different_projects() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(
        engine::storage::Store::open(dir.path()).await.unwrap(),
        "A1",
        "T1",
        "U2",
    );
    let first = binding();
    store.reserve(&first).await.unwrap();
    store.bind(&first).await.unwrap();
    let mut second = first.clone();
    second.id = "another-binding".into();
    second.workspace = "another-project".into();
    second.instance = "another-instance".into();
    second.thread = "another-session".into();
    second.root = "124.000001".into();
    store.reserve(&second).await.unwrap();
    store.bind(&second).await.unwrap();
    assert_eq!(
        store
            .at("D1", &first.root)
            .await
            .unwrap()
            .unwrap()
            .workspace,
        "project"
    );
    assert_eq!(
        store
            .at("D1", &second.root)
            .await
            .unwrap()
            .unwrap()
            .workspace,
        "another-project"
    );
    let mut channel = second;
    channel.channel = "C1".into();
    assert!(store.reserve(&channel).await.is_err());
}

fn fixture_api(endpoint: &str) -> crate::api::Api {
    use slack_morphism::prelude::*;
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .unwrap()
        .https_or_http()
        .enable_http1()
        .wrap_connector(network::Connector::default());
    crate::api::Api {
        proxy: Default::default(),
        client: std::sync::Arc::new(SlackClient::new(
            SlackClientHyperConnector::with_connector(connector)
                .with_slack_api_url(&format!("{endpoint}/api")),
        )),
        token: SlackApiToken::new("xoxb-fixture".into()).with_team_id("T1".into()),
        direct_channel: "D1".into(),
    }
}

#[tokio::test]
async fn owner_dm_resolution_and_output_never_use_channel_or_ephemeral_apis() {
    use crate::api::Platform;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, body_partial_json, method, path},
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.open"))
        .and(body_json(serde_json::json!({"users":["U1"]})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"channel":{"id":"D1"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(body_partial_json(
            serde_json::json!({"channel":"D1","thread_ts":"123.000001"}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"channel":"D1","ts":"123.000002","message":{"ts":"123.000002","text":"reply"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let api = fixture_api(&server.uri());
    assert_eq!(
        crate::api::Api::owner_conversation(&api.client, &api.token, "U1")
            .await
            .unwrap(),
        "D1"
    );
    let content = render::message("reply".into());
    assert!(api.post("C1", None, &content).await.is_err());
    assert!(api.post("D2", None, &content).await.is_err());
    assert_eq!(
        api.post("D1", Some("123.000001"), &content).await.unwrap(),
        "123.000002"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn owner_resolution_rejects_a_non_dm_response() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    Mock::given(path("/api/conversations.open"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"channel":{"id":"C1"}})),
        )
        .mount(&server)
        .await;
    let api = fixture_api(&server.uri());
    assert!(
        crate::api::Api::owner_conversation(&api.client, &api.token, "U1")
            .await
            .is_err()
    );
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
        serde_json::from_str::<SlackPushEventCallback>(&format!(r#"{{"team_id":"T1","api_app_id":"A1","event_id":"Ev1","event_time":123,"event":{{"type":"message","ts":"123.000002","channel":"D1","channel_type":"im","user":"{user}","text":"hello"{root}{extra}}}}}"#)).unwrap()
    };
    assert!(socket::message(make("", "U1", true), &settings(), "U2", "D1").is_some());
    for event in [
        make("", "U3", true),
        make(r#", "subtype":"message_changed""#, "U1", true),
        make(r#", "bot_id":"B1""#, "U1", true),
        make(r#", "hidden":true"#, "U1", true),
    ] {
        assert!(socket::message(event, &settings(), "U2", "D1").is_none());
    }
    let menu = socket::message(make("", "U1", false), &settings(), "U2", "D1").unwrap();
    assert!(matches!(menu.command, Command::Menu));
    assert!(menu.root.is_none());
    assert!(socket::message(make("", "U1", true), &settings(), "U2", "D2").is_none());
    let channel_event = make("", "U1", true);
    let wire = serde_json::to_string(&channel_event)
        .unwrap()
        .replace("\"im\"", "\"channel\"");
    let channel_event = serde_json::from_str(&wire).unwrap();
    assert!(socket::message(channel_event, &settings(), "U2", "D1").is_none());
    let event = make(r#", "subtype":"file_share""#, "U1", true);
    assert!(socket::message(event, &settings(), "U2", "D1").is_some());
    let file = make(
        r#", "subtype":"file_share", "files":[{"id":"F1","name":"notes.txt","mimetype":"text/plain"}]"#,
        "U1",
        true,
    );
    let file = socket::message(file, &settings(), "U2", "D1").unwrap();
    assert_eq!(file.uploads[0].location, "F1");
    assert_eq!(file.uploads[0].name, "notes.txt");
    let untrusted = make(r#", "files":[{"id":"F1","name":"notes.txt"}]"#, "U3", true);
    assert!(socket::message(untrusted, &settings(), "U2", "D1").is_none());
}

#[tokio::test]
async fn durable_intake_actions_and_unknown_delivery_survive_recovery_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let store = Store::new(core, "A1", "T1", "U2");
    let r = request("message-1");
    assert!(
        store
            .enqueue("D1:123.000002", &r, "scope", false)
            .await
            .unwrap()
    );
    let mut duplicate = r.clone();
    duplicate.id = "another-event-id".into();
    assert!(
        !store
            .enqueue("D1:123.000002", &duplicate, "scope", false)
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
            .consume(
                &nonce,
                Some("D1"),
                "U3",
                "scope",
                None,
                InteractionSource::default()
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .consume(
                &nonce,
                Some("D2"),
                "U1",
                "scope",
                None,
                InteractionSource::default()
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .consume(
                &nonce,
                Some("D1"),
                "U1",
                "scope",
                None,
                InteractionSource::default()
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .consume(
                &nonce,
                Some("D1"),
                "U1",
                "scope",
                None,
                InteractionSource::default()
            )
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
        .output(&r, &render::message("synthetic output".into()), "scope")
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
        .output(&r, &render::message("another thread".into()), "scope")
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
    socket_ack_fixture(false).await;
}

#[tokio::test]
async fn api_and_socket_both_use_the_configured_connect_proxy() {
    socket_ack_fixture(true).await;
}

async fn socket_ack_fixture(proxied: bool) {
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
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = if proxied {
        network::Proxy::try_from(format!("http://{}", proxy_listener.local_addr().unwrap()))
            .unwrap()
    } else {
        network::Proxy::default()
    };
    let proxy_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = proxy_hits.clone();
    let proxy_task = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (mut client, _) = proxy_listener.accept().await.unwrap();
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tasks.spawn(async move {
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(client.read_u8().await.unwrap());
                }
                let header = String::from_utf8(header).unwrap();
                let target = header.split_whitespace().nth(1).unwrap();
                assert!(header.starts_with("CONNECT 127.0.0.1:"));
                let mut remote = tokio::net::TcpStream::connect(target).await.unwrap();
                client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    let mut settings = settings();
    settings.proxy = proxy.clone();
    let service = Arc::new(runtime::Service {
        settings,
        context,
        store: Store::new(core, "A1", "T1", "U2"),
        api: Arc::new(NoNetwork),
        bot: "U2".into(),
        direct_channel: "D1".into(),
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
        .wrap_connector(network::Connector(proxy));
    let client = Arc::new(SlackClient::new(
        SlackClientHyperConnector::with_connector(connector)
            .with_slack_api_url(&format!("{}/api", http.uri())),
    ));
    let api = Arc::new(crate::api::Api {
        proxy: Default::default(),
        client,
        token: SlackApiToken::new("xoxb-fixture".into()).with_team_id("T1".into()),
        direct_channel: "D1".into(),
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
            r#"{{"type":"events_api","envelope_id":"{envelope}","accepts_response_payload":false,"payload":{{"team_id":"T1","api_app_id":"A1","event_id":"Ev1","event_time":123,"event":{{"type":"message","ts":"123.000002","channel":"D1","channel_type":"im","user":"U1","text":"menu"}}}}}}"#
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
    assert_eq!(
        proxy_hits.load(std::sync::atomic::Ordering::SeqCst),
        if proxied { 2 } else { 0 }
    );
    stop.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    proxy_task.abort();
    let _ = proxy_task.await;
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
                .consume(
                    &nonce,
                    Some("D1"),
                    "U1",
                    "scope",
                    None,
                    InteractionSource {
                        id: Some(click),
                        message: None
                    }
                )
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
    second.channel = "D2".into();
    store
        .output(&first, &render::message("one".into()), "scope")
        .await
        .unwrap();
    store
        .output(&second, &render::message("two".into()), "scope")
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
        direct_channel: "D1".into(),
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

#[test]
fn conversation_parts_repeat_roles_keep_rich_text_and_fit_mobile_message_budgets() {
    let content = crate::render::conversation(vec![
        session::presentation::Section {
            heading: session::presentation::role_heading(session::Role::User, "en"),
            body: "**Question** <@123>".into(),
        },
        session::presentation::Section {
            heading: session::presentation::message_heading(
                session::Role::Agent,
                Some("final_answer"),
                "en",
            ),
            body: format!(
                "| Name | Value |\n|---|---|\n| **Alpha** | [docs](https://example.com) |\n\n```rust\n{}\n```",
                "let a = 1;\n".repeat(400)
            ),
        },
    ]);
    let parts = crate::render::parts(&content).unwrap();
    assert!(parts.len() > 2);
    assert!(parts[0].text.starts_with("👤 User\n\n"));
    assert!(
        parts[1..]
            .iter()
            .all(|part| part.text.starts_with("🤖 Agent · ✅\n\n"))
    );
    assert!(
        parts
            .iter()
            .all(|part| session::markdown::fits(&part.text, 2800))
    );
    let wire = serde_json::to_string(&parts[1].blocks).unwrap();
    assert!(wire.contains("Alpha") && wire.contains("https://example.com"));
    assert!(parts.iter().any(|part| {
        serde_json::to_string(&part.blocks)
            .unwrap()
            .contains("rich_text_preformatted")
    }));
    assert!(
        !serde_json::to_string(&parts[0].blocks)
            .unwrap()
            .contains("\"type\":\"user\"")
    );
}
