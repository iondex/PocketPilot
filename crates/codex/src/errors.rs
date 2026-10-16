//! Local error notification collection; never subscribes/resumes a remote thread.
use crate::{Client, protocol as p};
use session::{Role, TextItem};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub(crate) type Errors = Arc<Mutex<VecDeque<(String, TextItem)>>>;

pub(crate) fn listen(client: &Client) -> Errors {
    let cache: Errors = Arc::new(Mutex::new(VecDeque::new()));
    let errors = cache.clone();
    let mut events = client.subscribe();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) if event.method == "error" && event.request_id.is_none() => {
                        let Some(raw) = event.params else {
                            continue;
                        };
                        if let Ok(error) = serde_json::from_str::<p::ErrorNotification>(raw.get()) {
                            let mut cache = errors.lock().expect("error notification cache");
                            let text = error.error.message.chars().take(8192).collect::<String>();
                            let key = format!(
                                "{:x}",
                                Sha256::digest(format!("{}:{}", error.will_retry, text).as_bytes())
                            );
                            let item = TextItem {
                                turn_id: error.turn_id,
                                item_id: if error.will_retry {
                                    format!("pp:notification-error:{key}")
                                } else {
                                    "pp:turn-error".into()
                                },
                                role: Role::Agent,
                                phase: Some(
                                    if error.will_retry {
                                        "error_retry"
                                    } else {
                                        "error"
                                    }
                                    .into(),
                                ),
                                text,
                                complete: true,
                                client_id: None,
                                attachments: vec![],
                            };
                            if let Some((_, old)) = cache.iter_mut().find(|(thread, old)| {
                                *thread == error.thread_id
                                    && old.turn_id == item.turn_id
                                    && old.item_id == item.item_id
                            }) {
                                *old = item;
                            } else {
                                cache.push_back((error.thread_id, item));
                                while cache.len() > 256 {
                                    cache.pop_front();
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => tracing::warn!(
                        "error notification receiver lagged; native history remains available"
                    ),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    cache
}

pub(crate) fn merge(cache: &Errors, thread: &str, mut items: Vec<TextItem>) -> Vec<TextItem> {
    for (_, error) in cache
        .lock()
        .expect("error notification cache")
        .iter()
        .filter(|(id, _)| id == thread)
    {
        if items.iter().any(|i| {
            i.is_error()
                && i.turn_id == error.turn_id
                && i.text == error.text
                && i.phase == error.phase
        }) {
            continue;
        }
        if let Some(native) = items
            .iter_mut()
            .find(|i| i.turn_id == error.turn_id && i.item_id == error.item_id)
        {
            if native.text.is_empty() {
                *native = error.clone();
            }
            continue;
        }
        let index = items
            .iter()
            .rposition(|i| i.turn_id == error.turn_id)
            .map_or(items.len(), |i| i + 1);
        items.insert(index, error.clone());
    }
    items
}

#[cfg(test)]
mod tests {
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
            Box::new(crate::transport::JsonLines::new(read, write, None)),
            std::time::Duration::from_secs(1),
        );
        let cache = listen(&client);
        let mut server = Framed::new(remote, LinesCodec::new());
        let notification = r#"{"method":"error","params":{"threadId":"thread","turnId":"turn","willRetry":true,"error":{"message":"network problem","additionalDetails":"private diagnostics"}}}"#;
        for _ in 0..2 {
            server.send(notification.to_owned()).await.unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while cache.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let merged = merge(&cache, "thread", vec![]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].phase.as_deref(), Some("error_retry"));
        assert_eq!(merged[0].text, "network problem");
        assert_eq!(merge(&cache, "thread", merged).len(), 1);
        assert!(merge(&cache, "other-thread", vec![]).is_empty());
        drop(client);
        assert!(
            server.next().await.is_none(),
            "observation must never emit a lifecycle or subscription write"
        );
    }
}
