use super::Service;
use crate::ledger::Outbound;
use engine::now;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;

impl Service {
    pub(super) async fn poll(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut failures = 0_u32;
        loop {
            let cursor = match self.store.cursor(self.bot, now()).await {
                Ok(cursor) => cursor,
                Err(_) => {
                    tracing::error!("poll cursor unavailable");
                    break;
                }
            };
            let result = tokio::select! { _=stop.changed()=>break, result=self.api.updates(cursor.1)=>result };
            match result {
                Ok(updates) => {
                    let updates = updates
                        .iter()
                        .map(|u| {
                            let mut input = crate::api::input(u);
                            if failures > 0
                                && let Some(input) = input.as_mut()
                            {
                                input.requires_confirmation = true;
                            }
                            (u.id.0, input)
                        })
                        .collect::<Vec<_>>();
                    if self
                        .store
                        .ingest(self.bot, cursor.0, &updates, now())
                        .await
                        .is_err()
                    {
                        tracing::error!(
                            "inbox persistence failed; fetched updates not acknowledged"
                        );
                        break;
                    }
                    failures = 0;
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let delay = match error {
                        crate::Error::RetryAfter(s) => s,
                        _ => 2_u64.pow(failures.min(6)),
                    };
                    tracing::warn!(%error,"Telegram poll failed; retrying reads only");
                    tokio::select! { _=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(delay))=>{} }
                }
            }
        }
    }

    pub(super) async fn work(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut jobs = JoinSet::new();
        let mut busy = std::collections::HashSet::new();
        loop {
            if jobs.len() < 16 {
                let peers = busy.iter().copied().collect::<Vec<_>>();
                match self.store.claim_available_input(self.bot, &peers).await {
                    Ok(Some(inbox)) => {
                        let key = (inbox.input.chat, inbox.input.topic);
                        busy.insert(key);
                        let service = self.clone();
                        jobs.spawn(async move {
                            let input = &inbox.input;
                            tracing::info!(
                                bot_id = service.bot,
                                update_id = input.update_id,
                                origin_topic = input.topic,
                                "Telegram input received"
                            );
                            if let Some(id) = &input.callback_id {
                                let _ = service.api.answer_callback(id).await;
                            }
                            let result = service.handle(input).await;
                            if let Err(error) = result {
                                tracing::warn!(%error,"input failed without replay");
                                let mut response_input = input.clone();
                                if service
                                    .store
                                    .binding(service.bot, input.chat, input.topic)
                                    .await
                                    .ok()
                                    .flatten()
                                    .is_none()
                                    && let Ok(Some(topic)) = service
                                        .store
                                        .management_topic(service.bot, input.chat)
                                        .await
                                {
                                    response_input.topic = topic;
                                }
                                let _ = service
                                    .respond(
                                        &response_input,
                                        &format!("error:{}", input.event_key),
                                        Outbound::text(error.user_message()),
                                    )
                                    .await;
                            }
                            let _ = service
                                .store
                                .finish_input(service.bot, inbox.epoch, inbox.id)
                                .await;
                            key
                        });
                        continue;
                    }
                    Ok(None) => {}
                    Err(_) => {
                        tracing::error!("inbox unavailable");
                        break;
                    }
                }
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next(), if !jobs.is_empty()=>{
                    match result {
                        Some(Ok(key))=>{busy.remove(&key);},
                        Some(Err(_))=>{tracing::error!("input worker failed; pending request will be marked uncertain on restart");break;},
                        None=>{},
                    }
                },
                _=tokio::time::sleep(Duration::from_millis(200))=>{},
            }
        }
        jobs.abort_all();
    }
}
