//! Exercise discovery, protobuf dispatch and callback ACK through the real SDK.
use super::*;
use futures_util::{SinkExt, StreamExt};
use lark_websocket_protobuf::pbbp2::{Frame, Header as FrameHeader};
use prost_lark::Message as ProstMessage;
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message as WsMessage};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

#[derive(Deserialize)]
struct Ack {
    code: u16,
    data: Option<String>,
}

async fn exchange(peer: &mut WebSocketStream<TcpStream>, id: &str, payload: Vec<u8>) -> Ack {
    let frame = Frame {
        seq_id: 1,
        log_id: 1,
        service: 42,
        method: 1,
        headers: vec![
            FrameHeader {
                key: "type".into(),
                value: "event".into(),
            },
            FrameHeader {
                key: "message_id".into(),
                value: id.into(),
            },
        ],
        payload_encoding: None,
        payload_type: None,
        payload: Some(payload),
        log_id_new: None,
    };
    peer.send(WsMessage::Binary(frame.encode_to_vec().into()))
        .await
        .unwrap();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), peer.next())
            .await
            .expect("SDK ACK timeout")
            .unwrap()
            .unwrap();
        if let WsMessage::Binary(bytes) = message {
            let response = Frame::decode(bytes.as_ref()).unwrap();
            if response.method == 1 {
                assert!(
                    response
                        .headers
                        .iter()
                        .any(|h| h.key == "message_id" && h.value == id)
                );
                return serde_json::from_slice(&response.payload.unwrap()).unwrap();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_discovery_messages_and_callback_acks_are_application_scoped() {
    for region in [Region::Feishu, Region::Lark] {
        let (_directory, service) = service(region).await;
        let (shutdown, stop) = watch::channel(false);
        let mut service = Arc::into_inner(service).unwrap();
        service.stop = stop;
        let service = Arc::new(service);
        let server = MockServer::start().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/?service_id=42", listener.local_addr().unwrap());
        Mock::given(method("POST"))
            .and(path("/callback/ws/endpoint"))
            .and(body_json(
                serde_json::json!({"AppID":"cli_example","AppSecret":"synthetic-secret"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code":0,"data":{"URL":url,"ClientConfig":{"PingInterval":60}}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let api =
            Arc::new(crate::feishu::api::Api::fixture(&service.settings, &server.uri()).unwrap());
        let task = tokio::spawn(crate::feishu::socket::run(service.clone(), api));
        let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut peer = accept_async(stream).await.unwrap();
        let event = message("om_socket", "event_socket", "ou_owner", "tenant_example");
        assert_eq!(
            exchange(&mut peer, "wire-one", event.clone()).await.code,
            200
        );
        assert_eq!(
            exchange(&mut peer, "wire-redelivery", event).await.code,
            200
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM feishu_inbox WHERE namespace=?")
            .bind(&service.store.namespace)
            .fetch_one(service.store.core.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
        let hash = service.context.config.snapshot().hash.clone();
        let nonce = service
            .store
            .action("oc_private", Action::Home { offset: 0 }, &hash)
            .await
            .unwrap();
        sqlx::query("UPDATE feishu_actions SET message='om_menu' WHERE nonce=?")
            .bind(&nonce)
            .execute(service.store.core.pool())
            .await
            .unwrap();
        let callback = serde_json::to_vec(&serde_json::json!({
            "header":{"event_id":"event_callback","event_type":"card.action.trigger","app_id":"cli_example","tenant_key":"tenant_example","create_time":(engine::now()*1000).to_string()},
            "event":{"operator":{"open_id":"ou_owner","tenant_key":"tenant_example"},"context":{"open_message_id":"om_menu","open_chat_id":"oc_private"},"action":{"value":{"nonce":nonce}}}
        })).unwrap();
        let ack = exchange(&mut peer, "wire-callback", callback).await;
        assert_eq!(ack.code, 200);
        assert!(ack.data.is_some_and(|data| !data.is_empty()));
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM feishu_inbox WHERE namespace=? AND id='card:event_callback'",
        )
        .bind(&service.store.namespace)
        .fetch_one(service.store.core.pool())
        .await
        .unwrap();
        assert_eq!(count, 1, "ACK must follow durable callback enqueue");
        let discovery = server.received_requests().await.unwrap();
        assert!(!discovery[0].headers.contains_key("authorization"));
        shutdown.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[test]
fn transport_ignores_proxy_environment_in_an_isolated_process() {
    // Change only the child environment: global mutation is unsafe in parallel tests.
    let test = format!(
        "{}::websocket_discovery_messages_and_callback_acks_are_application_scoped",
        module_path!().strip_prefix("integrations::").unwrap()
    );
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", &test, "--nocapture"]);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(key, "http://127.0.0.1:9");
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
