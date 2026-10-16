use super::Service;
use crate::ledger::Outbound;
use engine::now;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

impl Service {
    pub(super) async fn deliver(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut next_chat = HashMap::<i64, Instant>::new();
        let mut global = Instant::now();
        loop {
            let deliveries = match self.store.pending_deliveries(self.bot, now()).await {
                Ok(rows) => rows,
                Err(_) => break,
            };
            for row in deliveries {
                if *stop.borrow() {
                    return;
                }
                let snapshot = self.config.snapshot();
                let body: Outbound = match serde_json::from_str(&row.payload) {
                    Ok(body) => body,
                    Err(_) => break,
                };
                let paired = self
                    .store
                    .pairing()
                    .is_paired(self.bot, row.chat_id as u64, row.chat_id)
                    .await
                    .unwrap_or(false);
                let binding_valid = if let Some(revision) = body.binding_revision {
                    self.store
                        .binding(self.bot, row.chat_id, row.topic_id)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|b| b.revision == revision)
                } else {
                    true
                };
                if (body.require_pairing && !paired)
                    || row.chat_id <= 0
                    || !binding_valid
                    || body.expires_at.is_some_and(|expiry| expiry <= now())
                    || body
                        .scope
                        .as_ref()
                        .is_some_and(|scope| !self.scope_allowed(&snapshot.config, scope))
                    || body
                        .config_hash
                        .as_ref()
                        .is_some_and(|hash| *hash != snapshot.hash)
                {
                    let _ = self
                        .store
                        .delivery_state(&row.id, "pending", "rejected", None, 0)
                        .await;
                    continue;
                }
                if global > Instant::now()
                    || next_chat
                        .get(&row.chat_id)
                        .is_some_and(|at| *at > Instant::now())
                {
                    continue;
                }
                if self
                    .store
                    .delivery_state(&row.id, "pending", "dispatching", None, 0)
                    .await
                    .is_err()
                {
                    continue;
                }
                let result = async {
                    if let Some(title) = &body.topic_title {
                        self.api
                            .rename_topic(row.chat_id, row.topic_id, title)
                            .await?;
                        if let Some(binding) = self
                            .store
                            .binding(self.bot, row.chat_id, row.topic_id)
                            .await
                            .ok()
                            .flatten()
                        {
                            let _ = self.store.topic_title_receipt(&binding, title).await;
                        }
                    }
                    if let Some(message) = body.edit_message {
                        match self
                            .api
                            .edit_formatted(
                                row.chat_id,
                                message,
                                &body.text,
                                body.buttons.clone(),
                                body.markdown,
                            )
                            .await
                        {
                            Ok(()) => Ok(message),
                            Err(crate::Error::MessageUnavailable)
                                if body.navigation_id.is_some()
                                    || row.source_key.starts_with("status:") =>
                            {
                                self.api
                                    .send_formatted(
                                        row.chat_id,
                                        row.topic_id,
                                        &body.text,
                                        body.buttons.clone(),
                                        body.markdown,
                                    )
                                    .await
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        self.api
                            .send_formatted(
                                row.chat_id,
                                row.topic_id,
                                &body.text,
                                body.buttons.clone(),
                                body.markdown,
                            )
                            .await
                    }
                }
                .await;
                global = Instant::now() + Duration::from_millis(40);
                next_chat.insert(
                    row.chat_id,
                    Instant::now()
                        + Duration::from_millis(
                            60000 / u64::from(snapshot.config.delivery.messages_per_minute),
                        ),
                );
                match result {
                    Ok(message) => {
                        tracing::info!(
                            bot_id = self.bot,
                            topic_id = row.topic_id,
                            message_id = message,
                            edited = body.edit_message.is_some(),
                            "Telegram message delivered"
                        );
                        if let Some(id) = &body.navigation_id {
                            let _ = self.store.navigation_receipt(id, message, now()).await;
                        }
                        let _ = self
                            .store
                            .delivery_state(&row.id, "dispatching", "delivered", Some(message), 0)
                            .await;
                        if row.source_key.starts_with("status:")
                            && let Ok(Some(binding)) = self
                                .store
                                .binding(self.bot, row.chat_id, row.topic_id)
                                .await
                        {
                            let _ = self.store.status_card(&binding, message, &body.text).await;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error,"Telegram delivery failed");
                        if matches!(error, crate::Error::TopicUnavailable) {
                            let management = self
                                .store
                                .management_topic(self.bot, row.chat_id)
                                .await
                                .ok()
                                .flatten()
                                == Some(row.topic_id);
                            let _ = self
                                .store
                                .unavailable_topic(self.bot, row.chat_id, row.topic_id)
                                .await;
                            let mut notice = Outbound::text(if management {
                                "管理 topic 已关闭或删除。下次发送 /start 将重新创建管理入口；Codex 会话不受影响。"
                            } else {
                                "会话 topic 已关闭或删除；Codex 会话继续运行。请从 /sessions 重新添加，会创建一个新 topic。"
                            });
                            notice.scope = body.scope.clone();
                            let _ = self
                                .store
                                .enqueue(
                                    self.bot,
                                    row.chat_id,
                                    0,
                                    &format!("topic-unavailable:{}", row.id),
                                    &notice,
                                )
                                .await;
                        }
                        if matches!(error, crate::Error::NoRecipient) {
                            let _ = self.store.disable_chat(self.bot, row.chat_id).await;
                        }
                        if matches!(error, crate::Error::TooLarge) {
                            let mut fallback = Outbound::text(
                                "历史超过 Telegram 单个附件的大小限制，未发送不完整的历史。请在本机使用 sessions history --output 导出完整集合。",
                            );
                            fallback.scope = body.scope.clone();
                            fallback.binding_revision = body.binding_revision;
                            let _ = self
                                .store
                                .enqueue(
                                    self.bot,
                                    row.chat_id,
                                    row.topic_id,
                                    &format!("oversize:{}", row.id),
                                    &fallback,
                                )
                                .await;
                        }
                        if let crate::Error::RetryAfter(seconds) = &error {
                            global = Instant::now() + Duration::from_secs(*seconds);
                        }
                        let (state, retry) = match error {
                            crate::Error::RetryAfter(s) => (
                                "pending",
                                now().saturating_add(s.min(i64::MAX as u64) as i64),
                            ),
                            crate::Error::Rejected
                            | crate::Error::Authentication
                            | crate::Error::NoRecipient
                            | crate::Error::TopicUnavailable
                            | crate::Error::TooLarge => ("rejected", 0),
                            _ => ("outcome_unknown", 0),
                        };
                        let _ = self
                            .store
                            .delivery_state(&row.id, "dispatching", state, None, retry)
                            .await;
                    }
                }
            }
            tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
    }
}
