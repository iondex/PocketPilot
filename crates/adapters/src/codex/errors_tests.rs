use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_util::codec::{Framed, LinesCodec};

#[test]
fn terminal_notification_and_native_error_share_one_cursor_identity() {
    let error = TextItem {
        turn_id: "t".into(),
        item_id: "pp:turn-error".into(),
        role: Role::Agent,
        phase: Some("error".into()),
        text: "failed".into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let cache: Errors = Arc::new(Mutex::new(VecDeque::from([(
        "thread".into(),
        error.clone(),
    )])));
    assert_eq!(merge(&cache, "thread", vec![error.clone()]).len(), 1);
    let mut generic = error.clone();
    generic.text.clear();
    generic.phase = Some("turn_failed".into());
    let merged = merge(&cache, "thread", vec![generic]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].text, "failed");
    assert_eq!(merged[0].item_id, error.item_id);
}

#[tokio::test]
async fn received_errors_are_scoped_deduplicated_and_do_not_send_subscription_requests() {
    let (local, remote) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(local);
    let client = Client::spawn(
        Box::new(crate::codex::transport::JsonLines::new(read, write, None)),
        std::time::Duration::from_secs(1),
    );
    let cache = listen(&client, tokio::sync::watch::channel(()).0);
    let mut server = Framed::new(remote, LinesCodec::new());
    let notification = r#"{"method":"error","params":{"threadId":"thread","turnId":"turn","willRetry":true,"error":{"message":"network problem","additionalDetails":"private diagnostics"}}}"#;
    for _ in 0..2 {
        server.send(notification.to_owned()).await.unwrap();
    }
    let marker = r#"{"method":"error","params":{"threadId":"marker","turnId":"done","willRetry":false,"error":{"message":"terminal marker"}}}"#;
    server.send(marker.to_owned()).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while merge(&cache, "marker", vec![]).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        merge(&cache, "thread", vec![]).is_empty(),
        "recoverable notifications must stay out of visible history"
    );
    let failure = r#"{"method":"error","params":{"threadId":"thread","turnId":"turn","willRetry":false,"error":{"message":"request failed","additionalDetails":"private diagnostics"}}}"#;
    for _ in 0..2 {
        server.send(failure.to_owned()).await.unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while merge(&cache, "thread", vec![]).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let merged = merge(&cache, "thread", vec![]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].phase.as_deref(), Some("error"));
    assert_eq!(merged[0].text, "request failed");
    assert_eq!(merge(&cache, "thread", merged).len(), 1);
    assert!(merge(&cache, "other-thread", vec![]).is_empty());
    drop(client);
    assert!(
        server.next().await.is_none(),
        "observation must never emit a lifecycle or subscription write"
    );
}
