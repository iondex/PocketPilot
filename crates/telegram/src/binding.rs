use super::{Error, Service, item_hashes, status_text, topic_title};
use crate::api::Input;
use crate::ledger::Outbound;
use engine::connections;
use engine::reload::Snapshot;
use session::RuntimeStatus;

impl Service {
    pub(super) async fn open(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        thread: &str,
    ) -> Result<(), Error> {
        if !self.topics_enabled {
            return Err(Error::TopicsDisabled);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(thread).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        self.authorize(input, &snapshot.hash).await?;
        let topic = match self
            .store
            .topic(self.bot, input.chat, &workspace.provider_id, thread)
            .await?
        {
            Some(topic) => topic,
            None => {
                if !self
                    .store
                    .begin_topic(self.bot, input.chat, &workspace.provider_id, thread)
                    .await?
                {
                    return Err(Error::Unknown);
                }
                let title = topic_title(workspace, &current.title, thread);
                match self.api.create_topic(input.chat, &title).await {
                    Ok(topic) => {
                        self.store
                            .end_topic(
                                self.bot,
                                input.chat,
                                &workspace.provider_id,
                                thread,
                                "accepted",
                                Some(topic),
                            )
                            .await?;
                        topic
                    }
                    Err(error) => {
                        let state = if matches!(
                            error,
                            crate::Error::Rejected
                                | crate::Error::RetryAfter(_)
                                | crate::Error::NoRecipient
                                | crate::Error::Authentication
                        ) {
                            "rejected"
                        } else {
                            "outcome_unknown"
                        };
                        self.store
                            .end_topic(
                                self.bot,
                                input.chat,
                                &workspace.provider_id,
                                thread,
                                state,
                                None,
                            )
                            .await?;
                        return Err(error.into());
                    }
                }
            }
        };
        self.authorize(input, &snapshot.hash).await?;
        self.store
            .bind(self.bot, input.chat, topic, &workspace.provider_id, thread)
            .await?;
        self.store
            .set_workspace(self.bot, input.chat, topic, &workspace.id)
            .await?;
        let binding = self
            .store
            .binding(self.bot, input.chat, topic)
            .await?
            .ok_or(Error::Unbound)?;
        let binding_actor = self
            .connections
            .actor(
                &workspace.provider_id,
                &format!("__binding:{}:{topic}", input.chat),
            )
            .await;
        let _binding_guard = binding_actor.lock().await;
        let history = self
            .history(provider.as_ref(), workspace, thread, snapshot)
            .await;
        let history = match history {
            Ok(history) => history,
            Err(error) => {
                self.history_unavailable(&binding, workspace, &current.cwd, &error, snapshot)
                    .await?;
                self.refresh_status(&binding, workspace, &current, snapshot)
                    .await?;
                return self.opened(input, snapshot, workspace, topic).await;
            }
        };
        let after = provider.read(thread).await?;
        connections::require_workspace(&after.cwd, workspace)?;
        let text = if matches!(current.status, RuntimeStatus::Active { .. })
            || matches!(after.status, RuntimeStatus::Active { .. })
        {
            session::delivery::agent_history(&history)
        } else {
            session::delivery::latest_agent_answer(&history)
        };
        let mut body = Outbound::text(text);
        body.scope = Some(self.output_scope(&snapshot.config, workspace, &current.cwd)?);
        body.binding_revision = Some(binding.revision);
        self.store
            .publish_observation(
                &binding,
                &item_hashes(&history),
                &format!("history:{}", input.event_key),
                &body,
            )
            .await?;
        let mut card = Outbound::text(status_text(
            workspace,
            binding.display_name.as_deref().unwrap_or(&after.title),
            after.status.label(),
            thread,
        ));
        card.scope = Some(self.output_scope(&snapshot.config, workspace, &after.cwd)?);
        card.binding_revision = Some(binding.revision);
        self.store.enqueue_status(&binding, &card).await?;
        self.opened(input, snapshot, workspace, topic).await
    }
}
