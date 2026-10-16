use super::{Action, Error, Service};
use crate::api::Input;
use crate::ledger::Outbound;
use engine::config::Workspace;
use engine::connections;
use engine::now;
use engine::reload::Snapshot;
use session::operation::OperationState as State;
use session::provider::Mutation;
use session::provider::Receipt;
use session::{ProviderError, RuntimeStatus};

impl Service {
    pub(super) fn delayed(&self, input: &Input) -> bool {
        input.requires_confirmation
            || input.date < self.started_at
            || !(0..=300).contains(&now().saturating_sub(input.date))
    }

    pub(super) async fn draft(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        text: &str,
        steer_only: bool,
    ) -> Result<(), Error> {
        if text.trim().is_empty() {
            return Err(Error::Stale);
        }
        let binding = self.bound(input).await?;
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        let button = self
            .button(
                input,
                snapshot,
                "确认发送草稿",
                Action::SubmitDraft {
                    text: text.into(),
                    steer_only,
                    instance: binding.instance_id,
                    thread: binding.thread_id.clone(),
                    revision: binding.revision,
                },
            )
            .await?;
        tracing::info!(workspace_id=%workspace.id,thread_id=%binding.thread_id,"session input requires renewed confirmation");
        let mut body = Outbound::text(format!(
            "⚠️ 这条消息尚未发送\n\n{}\n\n核对会话后确认发送。",
            session::delivery::short_title(text, 80)
        ));
        body.buttons = vec![vec![button]];
        self.respond(input, &format!("draft:{}", input.event_key), body)
            .await
    }

    pub(super) async fn mutate(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
        key: &str,
        thread: &str,
        mutation: Mutation,
    ) -> Result<Receipt, Error> {
        self.authorize(input, &snapshot.hash).await?;
        if input.callback.is_none() && self.delayed(input) {
            return Err(Error::Stale);
        }
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        if !provider.capabilities().control_verified {
            return Err(Error::ReadOnly);
        }
        let id = self
            .store
            .reserve_operation(
                &format!("telegram:{}:{}:{}", self.bot, input.event_key, key),
                &workspace.provider_id,
                thread,
            )
            .await?;
        let state = self.store.operation_state(&id).await?;
        if state == State::Accepted {
            return self.store.receipt(&id).await?.ok_or(Error::Unknown);
        }
        if state != State::Received {
            return Err(Error::Unknown);
        }
        self.store
            .transition(&id, State::Received, State::Prepared)
            .await?;
        // Recheck after a potentially slow SSH connect and immediately before dispatch.
        self.authorize(input, &snapshot.hash).await?;
        if input.callback.is_none() && self.delayed(input) {
            return Err(Error::Stale);
        }
        let binding = if matches!(key, "resume" | "input") {
            Some(self.bound(input).await?)
        } else {
            None
        };
        if binding
            .as_ref()
            .is_some_and(|b| b.thread_id != thread || b.instance_id != workspace.provider_id)
        {
            return Err(Error::Expired);
        }
        self.store
            .dispatch_authorized(&id, self.bot, input, binding.as_ref())
            .await?;
        engine::operations::execute_dispatched(&self.store, &id, provider.as_ref(), mutation)
            .await
            .map_err(|error| match error {
                engine::operations::Error::Store(error) => Error::Store(error),
                engine::operations::Error::Provider(error) => Error::Provider(error),
                engine::operations::Error::Unknown => Error::Unknown,
            })
    }

    pub(super) async fn create(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        text: &str,
    ) -> Result<(), Error> {
        if !self.topics_enabled {
            return Err(Error::TopicsDisabled);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let actor = self
            .connections
            .actor(&workspace.provider_id, "__create")
            .await;
        let _guard = actor.lock().await;
        let receipt = self
            .mutate(
                input,
                snapshot,
                workspace,
                "create",
                "__new",
                Mutation::Create {
                    cwd: workspace.cwd.clone(),
                },
            )
            .await?;
        let thread = &receipt.thread;
        // Each step has its own receipt. A crash never replays create/resume/start.
        self.mutate(
            input,
            snapshot,
            workspace,
            "first-turn",
            thread,
            Mutation::Start {
                thread: thread.clone(),
                text: text.into(),
                client_id: format!("pp:{}:{}", self.bot, input.event_key),
            },
        )
        .await?;
        self.open(input, snapshot, id, thread).await
    }

    pub(super) async fn submit(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        text: &str,
        steer_only: bool,
    ) -> Result<(), Error> {
        if text.trim().is_empty() {
            return Err(Error::Stale);
        }
        let binding = self.bound(input).await?;
        let workspace = self.binding_workspace(&snapshot.config, &binding)?;
        let actor = self
            .connections
            .actor(&binding.instance_id, &binding.thread_id)
            .await;
        let _guard = actor.lock().await;
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let current = provider.read(&binding.thread_id).await?;
        connections::require_workspace(&current.cwd, workspace)?;
        let fresh = self.bound(input).await?;
        if fresh.revision != binding.revision {
            return Err(Error::Expired);
        }
        let client_id = format!("pp:{}:{}", self.bot, input.event_key);
        let (mutation, label) = match current.status {
            RuntimeStatus::Active {
                waiting_on_approval,
                waiting_on_user_input,
            } => {
                let expected = match current.active_turn {
                    Some(turn) => turn,
                    None => match provider.active_turn(&binding.thread_id).await? {
                        Some(turn) => turn,
                        None => return self.draft(input, snapshot, text, steer_only).await,
                    },
                };
                (
                    Mutation::Steer {
                        thread: binding.thread_id.clone(),
                        expected_turn: expected,
                        text: text.into(),
                        client_id,
                    },
                    if waiting_on_approval || waiting_on_user_input {
                        "已追加；原端的审批/用户问答仍需处理"
                    } else {
                        "已追加到运行中的轮次"
                    },
                )
            }
            RuntimeStatus::Idle if !steer_only => (
                Mutation::Start {
                    thread: binding.thread_id.clone(),
                    text: text.into(),
                    client_id,
                },
                "已开始新一轮",
            ),
            RuntimeStatus::NotLoaded if !steer_only => {
                self.mutate(
                    input,
                    snapshot,
                    workspace,
                    "resume",
                    &binding.thread_id,
                    Mutation::Resume {
                        thread: binding.thread_id.clone(),
                    },
                )
                .await?;
                (
                    Mutation::Start {
                        thread: binding.thread_id.clone(),
                        text: text.into(),
                        client_id,
                    },
                    "已继续会话",
                )
            }
            _ => return self.draft(input, snapshot, text, steer_only).await,
        };
        let result = self
            .mutate(
                input,
                snapshot,
                workspace,
                "input",
                &binding.thread_id,
                mutation,
            )
            .await;
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(Error::Provider(ProviderError::Conflict)) => {
                return self.draft(input, snapshot, text, steer_only).await;
            }
            Err(error) => return Err(error),
        };
        tracing::info!(thread_id=%binding.thread_id,turn_id=?receipt.turn,action=label,"session input accepted");
        Ok(())
    }
}
