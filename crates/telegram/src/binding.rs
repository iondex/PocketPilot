use super::{Error, Service, item_hashes, topic_title};
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
        self.open_named(input, snapshot, id, thread, None).await
    }

    pub(super) async fn open_named(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        thread: &str,
        name: Option<&str>,
    ) -> Result<(), Error> {
        if !self.topics_enabled {
            return Err(Error::TopicsDisabled);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(thread).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        let existing_topic = self
            .store
            .topic(self.bot, input.chat, &workspace.provider_id, thread)
            .await?;
        let saved_name = if current.native_title.is_none() {
            match existing_topic {
                Some(topic) => self
                    .store
                    .binding(self.bot, input.chat, topic)
                    .await?
                    .and_then(|b| b.display_name),
                None => None,
            }
        } else {
            None
        };
        let name = name
            .filter(|s| !s.trim().is_empty())
            .or(saved_name.as_deref())
            .unwrap_or(&current.title);
        if current.native_title.is_none()
            && name != "未命名会话"
            && provider.capabilities().control_verified
            && !self.delayed(input)
        {
            self.mutate(
                input,
                snapshot,
                workspace,
                "set-title",
                thread,
                session::provider::Mutation::SetTitleIfMissing {
                    thread: thread.into(),
                    title: name.into(),
                },
            )
            .await?;
        }
        self.authorize(input, &snapshot.hash).await?;
        let topic = match existing_topic {
            Some(topic) => topic,
            None => {
                if !self
                    .store
                    .begin_topic(self.bot, input.chat, &workspace.provider_id, thread)
                    .await?
                {
                    return Err(Error::Unknown);
                }
                let title = topic_title(workspace, name, thread);
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
        self.store.rename(self.bot, input.chat, topic, name).await?;
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
        let active = matches!(current.status, RuntimeStatus::Active { .. })
            || matches!(after.status, RuntimeStatus::Active { .. });
        let text = self.initial_messages(&binding, &history, active).await?;
        let mut body = Outbound::agent(text);
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
        self.refresh_status(&binding, workspace, &after, snapshot)
            .await?;
        self.opened(input, snapshot, workspace, topic).await
    }
}
