//! Renewable Telegram typing leases. Dropping these never controls the agent.
use super::Service;
use crate::telegram::{ledger::Binding, presentation};
use session::RuntimeStatus;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Notify, watch},
    task::JoinSet,
    time::Instant,
};

#[derive(Clone)]
struct Lease {
    chat: i64,
    topic: i32,
    revision: i64,
    until: Instant,
    retry_at: Instant,
}

#[derive(Default)]
pub struct Presence {
    changed: Notify,
    leases: Mutex<HashMap<(i64, i32), Lease>>,
}

impl Presence {
    pub async fn observe(&self, binding: &Binding, status: &RuntimeStatus, ttl: Duration) {
        let mut leases = self.leases.lock().await;
        let key = (binding.chat_id, binding.topic_id);
        if presentation::typing(status) {
            let newly_active = !leases.contains_key(&key);
            let retry_at = leases.get(&key).map_or_else(Instant::now, |l| l.retry_at);
            leases.insert(
                key,
                Lease {
                    chat: binding.chat_id,
                    topic: binding.topic_id,
                    revision: binding.revision,
                    until: Instant::now() + ttl,
                    retry_at,
                },
            );
            if newly_active {
                self.changed.notify_one();
            }
        } else {
            leases.remove(&key);
        }
    }

    pub async fn clear(&self, binding: &Binding) {
        self.leases
            .lock()
            .await
            .remove(&(binding.chat_id, binding.topic_id));
    }
}

impl Service {
    pub(super) async fn typing(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {_=stop.changed()=>break,_=tick.tick()=>{},_=self.presence.changed.notified()=>{}}
            let leases = {
                let mut leases = self.presence.leases.lock().await;
                leases.retain(|_, l| l.until > Instant::now());
                leases
                    .values()
                    .filter(|l| l.retry_at <= Instant::now())
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let mut jobs = JoinSet::new();
            for lease in leases {
                let service = self.clone();
                jobs.spawn(async move {
                    service.send_typing(lease).await;
                });
                if jobs.len() >= 4 {
                    let _ = jobs.join_next().await;
                }
            }
            tokio::select! {_=stop.changed()=>{jobs.abort_all();break;},_=async {while jobs.join_next().await.is_some() {}}=>{}}
        }
    }

    async fn send_typing(&self, lease: Lease) {
        let Some(binding) = self
            .store
            .binding(self.bot, lease.chat, lease.topic)
            .await
            .ok()
            .flatten()
        else {
            self.presence
                .leases
                .lock()
                .await
                .remove(&(lease.chat, lease.topic));
            return;
        };
        let snapshot = self.config.snapshot();
        let valid = binding.revision == lease.revision
            && self.binding_workspace(&snapshot.config, &binding).is_ok();
        let paired = self
            .store
            .pairing()
            .is_paired(self.bot, lease.chat as u64, lease.chat)
            .await
            .unwrap_or(false);
        if !valid || !paired {
            self.presence.clear(&binding).await;
            return;
        }
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            self.api.typing(lease.chat, lease.topic),
        )
        .await;
        match result {
            Ok(Ok(())) => {
                if let Some(item) = self
                    .presence
                    .leases
                    .lock()
                    .await
                    .get_mut(&(lease.chat, lease.topic))
                {
                    item.retry_at = Instant::now() + Duration::from_secs(3);
                }
            }
            Ok(Err(crate::telegram::Error::RetryAfter(seconds))) => {
                let mut leases = self.presence.leases.lock().await;
                for item in leases.values_mut().filter(|l| l.chat == lease.chat) {
                    item.retry_at = Instant::now() + Duration::from_secs(seconds);
                }
            }
            Ok(Err(error)) => {
                tracing::debug!(%error,topic_id=lease.topic,"typing hint unavailable")
            }
            _ => {}
        }
    }
}
