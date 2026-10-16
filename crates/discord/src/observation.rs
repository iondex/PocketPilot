use super::{Result, Service, error, hashes, item_key};
use crate::Api;
use crate::Command;
use crate::Input;
use crate::storage::Binding;
use engine::connections;
use engine::digest;
use engine::now;
use session::RuntimeStatus;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;

impl Service {
    pub(super) async fn refresh(&self, api: &Api, b: &Binding) -> Result<()> {
        let snapshot = self.config.snapshot();
        let ws = self.validate_binding(&snapshot.config, b)?;
        let channel = b.channel.parse::<u64>().map_err(error)?;
        let input = Input {
            id: 0,
            bot: api.bot,
            guild: api.guild,
            channel,
            user: b.owner.parse().map_err(error)?,
            date: now(),
            command: Command::Status,
        };
        self.fresh(api, &input, &snapshot, false).await?;
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        let current = provider.read(&b.thread).await.map_err(error)?;
        connections::require_workspace(&current.cwd, ws).map_err(error)?;
        let history = self.history(provider.as_ref(), ws, &b.thread).await?;
        self.fresh(api, &input, &snapshot, false).await?;
        let current_binding = self.bound(&input).await?;
        if current_binding.revision != b.revision {
            return Ok(());
        }
        let old: BTreeMap<String, String> =
            serde_json::from_str(&b.observed_items).map_err(error)?;
        let fresh = hashes(&history);
        let text = if !b.history_ready {
            if matches!(current.status, RuntimeStatus::Active { .. }) {
                session::delivery::full_history(&history)
            } else {
                session::delivery::latest_interaction(&history)
            }
        } else {
            history
                .items
                .iter()
                .filter(|i| old.get(&item_key(i)) != Some(&digest(&i.text)))
                .map(|i| {
                    format!(
                        "{}\n{}",
                        if i.role == session::Role::User {
                            "用户"
                        } else {
                            "Agent"
                        },
                        i.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n────────\n\n")
        };
        if !b.history_ready || !text.is_empty() {
            self.store
                .discord_publish(b, &format!("items:{}", digest(&fresh)), &text, &fresh)
                .await
                .map_err(error)?;
        }
        let card = format!(
            "{} · {}\nthread：{}",
            current.status.label(),
            session::delivery::short_title(&current.title, 80),
            b.thread
        );
        self.update_card(api, b, &card).await
    }

    pub(super) async fn update_card(&self, api: &Api, b: &Binding, text: &str) -> Result<()> {
        if b.card_text.as_deref() == Some(text) {
            return Ok(());
        }
        let channel = b.channel.parse().map_err(error)?;
        if let Some(id) = &b.card_message {
            if let Ok(id) = id.parse::<u64>()
                && id != 0
            {
                api.card(channel, id, text).await.map_err(error)?;
                self.store
                    .discord_card(b, Some(id), text)
                    .await
                    .map_err(error)?;
            }
        } else {
            // Zero is an unknown-send tombstone, never a valid Discord message ID.
            self.store
                .discord_card(b, Some(0), "状态卡发送中；若结果未知请使用 /pp-status")
                .await
                .map_err(error)?;
            let id = api.send(channel, text).await.map_err(error)?;
            self.store
                .discord_card(b, Some(id), text)
                .await
                .map_err(error)?;
        }
        Ok(())
    }

    pub(super) async fn observe(
        self: Arc<Self>,
        api: Api,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), crate::Error> {
        let mut jobs = JoinSet::new();
        let mut busy = HashSet::new();
        loop {
            for b in self
                .store
                .discord_bindings(api.bot, api.guild)
                .await
                .map_err(|_| crate::Error::Backend)?
            {
                if jobs.len() >= 8 || !busy.insert(b.channel.clone()) {
                    continue;
                }
                let service = self.clone();
                let api = api.clone();
                jobs.spawn(async move {
                    if service.refresh(&api, &b).await.is_err() {
                        tracing::warn!("Discord observation unavailable; session unchanged");
                        // Only a validated, still-owned binding may receive a stale marker.
                        let snapshot = service.config.snapshot();
                        let input = Input {
                            id: 0,
                            bot: api.bot,
                            guild: api.guild,
                            channel: b.channel.parse().unwrap_or(0),
                            user: b.owner.parse().unwrap_or(0),
                            date: now(),
                            command: Command::Status,
                        };
                        if service.validate_binding(&snapshot.config, &b).is_ok()
                            && service.bound(&input).await.is_ok()
                            && service.fresh(&api, &input, &snapshot, false).await.is_ok()
                        {
                            let _ = service
                                .update_card(&api, &b, "连接未知 · 请刷新 /pp-status")
                                .await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    b.channel
                });
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next(),if !jobs.is_empty()=>{match result {Some(Ok(channel))=>{busy.remove(&channel);},_=>return Err(crate::Error::Backend)}},
                _=tokio::time::sleep(Duration::from_secs(1))=>{},
            }
        }
        jobs.abort_all();
        Ok(())
    }
}
