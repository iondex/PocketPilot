use super::{Error, Service, item_hashes, item_key, topic_title};
use crate::telegram::ledger::Binding;
use crate::telegram::ledger::Outbound;
use engine::config::Workspace;
use engine::reload::Snapshot;
use engine::runtimes;
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
        let local = self.runtimes.history_limit(&workspace.provider_id).await;
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
        let history = tokio::time::timeout(
            Duration::from_secs(snapshot.config.delivery.history_barrier_timeout_seconds),
            work,
        )
        .await
        .map_err(|_| ProviderError::Timeout)??;
        Ok(self
            .store
            .retain_errors(&workspace.provider_id, thread, history)
            .await?)
    }

    pub(super) async fn offline_status(
        &self,
        binding: &Binding,
        error: &Error,
    ) -> Result<(), Error> {
        let actor = self
            .runtimes
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
        let snapshot = self.config.snapshot();
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        tracing::debug!(thread_id=%binding.thread_id,%error,"session status unavailable");
        self.presence.clear(&binding).await;
        let appearance = crate::telegram::presentation::Appearance {
            title: binding.topic_title.clone().unwrap_or_else(|| {
                crate::telegram::presentation::title(
                    binding.display_name.as_deref().unwrap_or("nav.sessions"),
                )
            }),
            emoji: crate::telegram::presentation::emoji(&RuntimeStatus::Unknown).into(),
        };
        self.queue_appearance(&binding, workspace, &workspace.cwd, appearance, &snapshot)
            .await
    }

    pub(super) async fn history_unavailable(
        &self,
        binding: &Binding,
        error: &Error,
    ) -> Result<(), Error> {
        tracing::warn!(thread_id=%binding.thread_id,%error,"session history unavailable; keeping independently observed runtime");
        Ok(())
    }

    pub(super) async fn refresh_status(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        current: &session::Session,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        if let Err(error) = engine::catalog::Catalog::new(self.store.pool().clone())
            .touch_workspace(&workspace.id, current.updated_at)
            .await
        {
            tracing::debug!(%error,"workspace activity metadata unavailable");
        }
        let ttl = Duration::from_secs(
            snapshot
                .config
                .delivery
                .history_barrier_timeout_seconds
                .saturating_add(90),
        );
        self.presence.observe(binding, &current.status, ttl).await;
        let appearance = crate::telegram::presentation::Appearance {
            title: topic_title(
                workspace,
                binding.display_name.as_deref().unwrap_or(&current.title),
                &binding.thread_id,
            ),
            emoji: crate::telegram::presentation::emoji(&current.status).into(),
        };
        self.queue_appearance(binding, workspace, &current.cwd, appearance, snapshot)
            .await
    }

    async fn queue_appearance(
        &self,
        binding: &Binding,
        workspace: &Workspace,
        cwd: &str,
        appearance: crate::telegram::presentation::Appearance,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        if binding.topic_title.as_deref() == Some(appearance.title.as_str())
            && binding.status_text.as_deref() == Some(appearance.receipt().as_str())
            && binding.status_message_id.is_none()
        {
            return Ok(());
        }
        let mut body = Outbound::text("");
        body.appearance = Some(appearance);
        body.remove_status = binding.status_message_id;
        body.scope = Some(self.output_scope(&snapshot.config, workspace, cwd)?);
        body.binding_revision = Some(binding.revision);
        self.store.enqueue_appearance(binding, &body).await?;
        Ok(())
    }

    pub(super) async fn refresh_binding(&self, binding: &Binding) -> Result<(), Error> {
        let actor = self
            .runtimes
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
        let provider = self.runtimes.get(&snapshot.config, workspace).await?;
        let current = provider.read(&binding.thread_id).await?;
        runtimes::require_workspace(&current.cwd, workspace)?;
        self.refresh_status(binding, workspace, &current, &snapshot)
            .await?;
        let history = self
            .history(provider.as_ref(), workspace, &binding.thread_id, &snapshot)
            .await;
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                return self.history_unavailable(binding, &error).await;
            }
        };
        if !binding.history_ready {
            let text = self
                .initial_messages(
                    binding,
                    &history,
                    matches!(current.status, RuntimeStatus::Active { .. }),
                )
                .await?;
            let mut body = Outbound::agent(text);
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
        new.remove("@connection_error");
        new.extend(item_hashes(&history));
        let upgrading = old
            .values()
            .all(|hash| !hash.starts_with("v2:") && !hash.starts_with("v3:"));
        let latest_turn = history
            .items
            .iter()
            .rfind(|i| i.role == session::Role::User)
            .map(|i| i.turn_id.as_str());
        let own = self.store.own_client_ids(binding).await?;
        let changed = history
            .items
            .iter()
            .filter(|item| {
                if item.is_transient_error() {
                    return false;
                }
                let key = item_key(item);
                let previous = old.get(&key);
                if super::unchanged(item, previous) {
                    return false;
                }
                if item.role == session::Role::User
                    && item.client_id.as_ref().is_some_and(|id| own.contains(id))
                {
                    return false;
                }
                if upgrading && item.is_error() && Some(item.turn_id.as_str()) != latest_turn {
                    return false;
                }
                true
            })
            .collect::<Vec<_>>();
        let locale = self.ui_locale(binding.chat_id).await;
        let text = changed
            .iter()
            .map(|item| super::messages::render(item, &locale))
            .collect::<Vec<_>>()
            .join("\n\n");
        let visible_members = changed.len();
        if visible_members > 0 {
            tracing::info!(
                topic_id = binding.topic_id,
                binding_revision = binding.revision,
                observed_items = history.items.len(),
                changed_items = visible_members,
                intermediate_items = changed
                    .iter()
                    .filter(|i| matches!(
                        i.phase.as_deref(),
                        Some("commentary" | "partial_answer" | "plan")
                    ))
                    .count(),
                final_items = changed
                    .iter()
                    .filter(|i| i.phase.as_deref() == Some("final_answer"))
                    .count(),
                "session visible changes evaluated"
            );
        }
        if text.is_empty() {
            if new != old {
                self.store.observe(binding, &new).await?;
            }
            return Ok(());
        }
        let mut body = Outbound::agent(text);
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
