use super::*;
use crate::codex::transport::JsonLines;
use futures_util::{SinkExt, StreamExt};
use tokio_util::codec::{Framed, LinesCodec};

#[tokio::test]
async fn retries_only_the_known_empty_rollout_read_race() {
    let (local, remote) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(local);
    let client = Client::spawn(
        Box::new(JsonLines::new(read, write, None)),
        Duration::from_secs(1),
    );
    let task = tokio::spawn(async move {
        let mut server = Framed::new(remote, LinesCodec::new());
        for attempt in 0..2 {
            let request: Incoming =
                serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(request.method.as_deref(), Some("thread/turns/list"));
            let body = if attempt == 0 {
                r#""error":{"code":-32603,"message":"failed to read rollout: rollout at /private/fixture is empty"}"#
            } else {
                r#""result":{"data":[]}"#
            };
            server
                .send(format!("{{\"id\":{},{body}}}", request.id.unwrap()))
                .await
                .unwrap();
        }
    });
    let params: p::ThreadTurnsListParams = p::ThreadTurnsListParams::builder()
        .thread_id("fixture")
        .try_into()
        .unwrap();
    assert!(client.call(&params).await.unwrap().data.is_empty());
    task.await.unwrap();
    let error = Failure {
        code: -32603,
        message: "rollout at /fixture is empty".into(),
    };
    assert!(!history_pending("turn/start", &error));
    assert!(!history_pending("turn/steer", &error));
    assert!(!history_pending("thread/resume", &error));
}

#[tokio::test]
async fn websocket_handshake_uses_schema_types_and_initialized_notification() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let request: Incoming = serde_json::from_str(&frame).unwrap();
        assert_eq!(request.method.as_deref(), Some("initialize"));
        let params: p::InitializeParams =
            serde_json::from_str(request.params.unwrap().get()).unwrap();
        assert!(params.capabilities.unwrap().experimental_api);
        socket.send(tokio_tungstenite::tungstenite::Message::Text(format!(
            "{{\"id\":{},\"result\":{{\"codexHome\":\"/tmp/test-codex\",\"userAgent\":\"test\",\"platformFamily\":\"unix\",\"platformOs\":\"test\"}}}}", request.id.unwrap()
        ).into())).await.unwrap();
        let initialized: Incoming =
            serde_json::from_str(&socket.next().await.unwrap().unwrap().into_text().unwrap())
                .unwrap();
        assert_eq!(initialized.method.as_deref(), Some("initialized"));
        assert!(initialized.id.is_none());
    });
    let client = Client::connect(
        Endpoint::Websocket { endpoint },
        Duration::from_secs(2),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    server.await.unwrap();
    drop(client);
}

#[tokio::test]
async fn multiplexes_typed_responses_and_never_answers_server_requests() {
    let (local, remote) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(local);
    let client = Client::spawn(
        Box::new(JsonLines::new(read, write, None)),
        Duration::from_secs(2),
    );
    let mut server = Framed::new(remote, LinesCodec::new());
    let task = tokio::spawn(async move {
        let a: Incoming = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
        let b: Incoming = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(a.method.as_deref(), Some("thread/list"));
        assert_eq!(b.method.as_deref(), Some("thread/loaded/list"));
        server.send("{\"id\":\"approval-1\",\"method\":\"item/commandExecution/requestApproval\",\"params\":{\"threadId\":\"t\"}}".to_owned()).await.unwrap();
        server
            .send(format!(
                "{{\"id\":{},\"result\":{{\"data\":[\"t\"]}}}}",
                b.id.unwrap()
            ))
            .await
            .unwrap();
        server
            .send(format!(
                "{{\"id\":{},\"result\":{{\"data\":[]}}}}",
                a.id.unwrap()
            ))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.next())
                .await
                .is_err()
        );
    });
    let mut events = client.subscribe();
    let list = p::ThreadListParams::default();
    let loaded = p::ThreadLoadedListParams::default();
    let (a, b) = tokio::join!(client.call(&list), client.call(&loaded));
    assert!(a.unwrap().data.is_empty());
    assert_eq!(b.unwrap().data, vec!["t"]);
    assert!(
        matches!(events.recv().await.unwrap().request_id,Some(p::RequestId::String(id)) if id == "approval-1")
    );
    task.await.unwrap();
}

#[tokio::test]
async fn timeout_does_not_resend_or_cancel() {
    let (local, remote) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(local);
    let client = Client::spawn(
        Box::new(JsonLines::new(read, write, None)),
        Duration::from_millis(30),
    );
    let mut server = Framed::new(remote, LinesCodec::new());
    let request = tokio::spawn(async move { client.call(&p::ThreadListParams::default()).await });
    let incoming: Incoming = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
    assert_eq!(incoming.method.as_deref(), Some("thread/list"));
    assert!(matches!(request.await.unwrap(), Err(Error::Timeout)));
    assert!(server.next().await.is_none());
}
