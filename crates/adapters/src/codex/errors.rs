//! Local error notification collection; never subscribes/resumes a remote thread.
use crate::codex::{Client, protocol as p};
use session::{Role, TextItem};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub(crate) type Errors = Arc<Mutex<VecDeque<(String, TextItem)>>>;

pub(crate) fn listen(client: &Client, changes: tokio::sync::watch::Sender<()>) -> Errors {
    let cache: Errors = Arc::new(Mutex::new(VecDeque::new()));
    let errors = cache.clone();
    let mut events = client.subscribe();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            loop {
                let event = events.recv().await;
                if let Ok(event) = &event
                    && matches!(
                        event.method.as_str(),
                        "thread/started"
                            | "thread/closed"
                            | "thread/archived"
                            | "thread/unarchived"
                            | "thread/status/changed"
                            | "turn/started"
                            | "turn/completed"
                            | "error"
                    )
                {
                    changes.send_replace(());
                }
                match event {
                    Ok(event) if event.method == "error" && event.request_id.is_none() => {
                        let Some(raw) = event.params else {
                            continue;
                        };
                        if let Ok(error) = serde_json::from_str::<p::ErrorNotification>(raw.get()) {
                            if error.will_retry {
                                continue;
                            }
                            let mut cache = errors.lock().expect("error notification cache");
                            let text = error.error.message.chars().take(8192).collect::<String>();
                            let item = TextItem {
                                turn_id: error.turn_id,
                                item_id: "pp:turn-error".into(),
                                role: Role::Agent,
                                phase: Some("error".into()),
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
#[path = "errors_tests.rs"]
mod tests;
