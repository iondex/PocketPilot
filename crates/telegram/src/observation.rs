use super::{Error, Service, item_hashes, item_key, status_text, topic_title};
use crate::ledger::Binding;
use crate::ledger::Outbound;
use engine::config::Workspace;
use engine::connections;
use engine::reload::Snapshot;
use engine::storage::StoreError;
use session::AgentProvider;
use session::History;
use session::ProviderError;
use session::RuntimeStatus;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;

impl Service {
    pub(super) async fn observe(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        type Key = (i64, i32);
        let mut workers = HashMap::<Key, tokio::task::AbortHandle>::new();
        let mut jobs = JoinSet::new();
        loop {
            let bindings = match self.store.bindings(self.bot).await {
                Ok(rows) => rows,
                Err(_) => break,
            };
            let active = bindings
                .iter()
                .map(|b| (b.chat_id, b.topic_id))
                .collect::<std::collections::HashSet<_>>();
            workers.retain(|key, handle| {
                if active.contains(key) {
                    true
                } else {
                    handle.abort();
                    false
                }
            });
            for binding in bindings {
                let key = (binding.chat_id, binding.topic_id);
                if workers.contains_key(&key) {
                    continue;
                }
                let service = self.clone();
                let handle=jobs.spawn(async move {
                    loop {
                        if service.store.binding(service.bot,key.0,key.1).await.ok().flatten().is_none() {break;}
                        if let Err(error)=service.refresh_binding(&binding).await {tracing::debug!(%error,"session observation unavailable; no mutation requested");let _=service.offline_status(&binding,&error).await;}
                        let delay=service.config.snapshot().config.delivery.burst_window_ms;
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    key
                });
                workers.insert(key, handle);
            }
            tokio::select! {
                _=stop.changed()=>break,
                result=jobs.join_next_with_id(), if !jobs.is_empty()=>{
                    match result {
                        Some(Ok((id,key)))=>{if workers.get(&key).is_some_and(|handle|handle.id()==id) {workers.remove(&key);}},
                        Some(Err(error))=>{workers.retain(|_,handle|handle.id()!=error.id());},
                        None=>{},
                    }
                },
                _=tokio::time::sleep(Duration::from_secs(2))=>{},
            }
        }
        jobs.abort_all();
    }

    pub(super) async fn history(
        &self,
        provider: &dyn AgentProvider,
        workspace: &Workspace,
        thread: &str,
        snapshot: &Snapshot,
    ) -> Result<History, Error> {
        let local = self.connections.history_limit(&workspace.provider_id).await;
        let work = async {
            let _local = local
                .acquire()
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            let _global = self
                .history_slots
                .acquire()
                .await
                .map_err(|_| ProviderError::Disconnected)?;
            provider.history(thread).await
        };
        Ok(tokio::time::timeout(
            Duration::from_secs(snapshot.config.delivery.history_barrier_timeout_seconds),
            work,
        )
        .await
        .map_err(|_| ProviderError::Timeout)??)
    }

    pub(super) async fn offline_status(
        &self,
        binding: &Binding,
        error: &Error,
    ) -> Result<(), Error> {
        let binding = self
            .store
            .binding(self.bot, binding.chat_id, binding.topic_id)
            .await?
            .ok_or(Error::Unbound)?;
        let snapshot = self.config.snapshot();
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        tracing::debug!(thread_id=%binding.thread_id,%error,"session status unavailable");
        let status = "⚫ 暂时离线，正在重连。".to_owned();
        if binding.status_text.as_deref() == Some(&status) {
            return Ok(());
        }
        let mut body = Outbound::text(status);
        body.edit_message = binding.status_message_id;
        body.binding_revision = Some(binding.revision);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?);
        self.store.enqueue_status(&binding, &body).await?;
        Ok(())
    }

    pub(super) async fn history_unavailable(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        cwd: &str,
        error: &Error,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        tracing::warn!(%error,"session history unavailable; keeping independently observed runtime");
        let mut body = Outbound::text("⚠️ 历史暂不可用，请稍后 /history。");
        body.scope = Some(self.output_scope(&snapshot.config, workspace, cwd)?);
        body.binding_revision = Some(binding.revision);
        self.store
            .enqueue(
                self.bot,
                binding.chat_id,
                binding.topic_id,
                &format!(
                    "history-unavailable:{}:{}",
                    binding.thread_id, binding.revision
                ),
                &body,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn refresh_status(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        current: &session::Session,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let status = status_text(
            workspace,
            binding.display_name.as_deref().unwrap_or(&current.title),
            current.status.label(),
            &binding.thread_id,
        );
        let title = topic_title(
            workspace,
            binding.display_name.as_deref().unwrap_or(&current.title),
            &binding.thread_id,
        );
        let title_changed = binding.topic_title.as_deref() != Some(&title);
        if binding.status_text.as_deref() != Some(&status) || title_changed {
            let mut body = Outbound::text(status);
            body.edit_message = binding.status_message_id;
            body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
            body.binding_revision = Some(binding.revision);
            if title_changed {
                body.topic_title = Some(title);
            }
            // Frequent runtime changes edit one card; Telegram emits a service
            // message for every topic rename, so status never belongs in its name.
            self.store.enqueue_status(binding, &body).await?;
        }
        Ok(())
    }

    pub(super) async fn refresh_binding(&self, binding: &Binding) -> Result<(), Error> {
        let actor = self
            .connections
            .actor(
                &binding.instance_id,
                &format!("__binding:{}:{}", binding.chat_id, binding.topic_id),
            )
            .await;
        let _guard = actor.lock().await;
        let binding = self
            .store
            .binding(self.bot, binding.chat_id, binding.topic_id)
            .await?
            .ok_or(Error::Unbound)?;
        let binding = &binding;
        if !self
            .store
            .pairing()
            .is_paired(self.bot, binding.chat_id as u64, binding.chat_id)
            .await
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::NotPaired.into());
        }
        let snapshot = self.config.snapshot();
        let workspace = self.binding_workspace(&snapshot.config, binding)?;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(&binding.thread_id).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        self.refresh_status(binding, workspace, &current, &snapshot)
            .await?;
        let history = self
            .history(provider.as_ref(), workspace, &binding.thread_id, &snapshot)
            .await;
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                return self
                    .history_unavailable(binding, workspace, &current.cwd, &error, &snapshot)
                    .await;
            }
        };
        if !binding.history_ready {
            let text = if matches!(current.status, RuntimeStatus::Active { .. }) {
                session::delivery::agent_history(&history)
            } else {
                session::delivery::latest_agent_answer(&history)
            };
            let mut body = Outbound::text(text);
            body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
            body.binding_revision = Some(binding.revision);
            self.store
                .publish_observation(
                    binding,
                    &item_hashes(&history),
                    &format!("initial-history:{}", binding.revision),
                    &body,
                )
                .await?;
            return Ok(());
        }
        let old: BTreeMap<String, String> =
            serde_json::from_str(&binding.observed_items).map_err(|_| StoreError::Database)?;
        let mut new = old.clone();
        new.extend(item_hashes(&history));
        let changed = history
            .items
            .iter()
            .filter(|i| old.get(&item_key(i)) != new.get(&item_key(i)))
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return Ok(());
        }
        let text = changed
            .iter()
            .filter(|i| i.role == session::Role::Agent)
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let visible_members = changed
            .iter()
            .filter(|i| i.role == session::Role::Agent)
            .count();
        if text.is_empty() {
            self.store.observe(binding, &new).await?;
            return Ok(());
        }
        let mut body = Outbound::text(text);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
        body.binding_revision = Some(binding.revision);
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Stale)?
            .as_millis()
            .min(i64::MAX as u128) as i64;
        self.store
            .publish_batch(
                binding,
                &new,
                body,
                visible_members,
                &snapshot.config.delivery,
                time_ms,
            )
            .await?;
        Ok(())
    }
}
