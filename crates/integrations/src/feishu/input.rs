//! Explicit user input uses the common operation ledger and per-session actor.
use super::*;
use session::{
    RuntimeStatus,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};

impl Service {
    #[allow(clippy::too_many_arguments)]
    async fn mutate(
        &self,
        input: &Input,
        hash: &str,
        project: &engine::config::Workspace,
        binding: Option<&Binding>,
        thread: &str,
        key: &str,
        mutation: Mutation,
    ) -> Result<Receipt> {
        let snapshot = self.fresh(input, hash)?;
        let provider = self.context.runtimes.get(&snapshot.config, project).await?;
        if !provider.capabilities().control_verified {
            return Err(Error::Ui("feishu.error.read_only"));
        }
        let operation = self
            .store
            .core
            .reserve_operation(
                &format!("{}:{}:{key}", self.store.namespace, input.id),
                &project.provider_id,
                thread,
            )
            .await?;
        match self.store.core.operation_state(&operation).await? {
            State::Accepted => {
                return self
                    .store
                    .core
                    .receipt(&operation)
                    .await?
                    .ok_or(Error::Ui("feishu.error.unknown"));
            }
            State::Received => {}
            _ => return Err(Error::Ui("feishu.error.unknown")),
        }
        self.store
            .core
            .transition(&operation, State::Received, State::Prepared)
            .await?;
        self.fresh(input, hash)?;
        if let Some(binding) = binding {
            self.current_binding(binding).await?;
        }
        self.store
            .core
            .transition(&operation, State::Prepared, State::Dispatching)
            .await?;
        Ok(engine::operations::execute_dispatched(
            &self.store.core,
            &operation,
            provider.as_ref(),
            mutation,
        )
        .await?)
    }

    pub(super) async fn create(
        &self,
        input: &Input,
        hash: &str,
        project: &str,
        title: &str,
    ) -> Result<()> {
        let snapshot = self.fresh(input, hash)?;
        let selected = Navigator::new(&snapshot.config)
            .parent(project)
            .map_err(|_| Error::Ui("feishu.error.scope"))?;
        let actor = self
            .context
            .runtimes
            .actor(&selected.project.provider_id, &format!("create:{project}"))
            .await;
        let _guard = actor.lock().await;
        let receipt = self
            .mutate(
                input,
                hash,
                selected.project,
                None,
                "__create",
                "create",
                Mutation::Create {
                    cwd: selected.project.cwd.clone(),
                },
            )
            .await?;
        self.mutate(
            input,
            hash,
            selected.project,
            None,
            &receipt.thread,
            "title",
            Mutation::SetTitleIfMissing {
                thread: receipt.thread.clone(),
                title: title.into(),
            },
        )
        .await?;
        self.menu(
            input,
            hash,
            Action::Groups {
                project: project.into(),
                session: receipt.thread,
            },
        )
        .await
    }

    pub(super) async fn input(&self, input: &Input, hash: &str, text: &str) -> Result<()> {
        let snapshot = self.fresh(input, hash)?;
        let b = self
            .store
            .bound(
                &input.chat,
                input.thread.as_deref().unwrap_or_default(),
                input.root.as_deref().unwrap_or_default(),
            )
            .await?;
        let project = self.validate(&snapshot, &b)?;
        let actor = self.context.runtimes.actor(&b.instance, &b.session).await;
        let _guard = actor.lock().await;
        self.current_binding(&b).await?;
        let provider = self.context.runtimes.get(&snapshot.config, project).await?;
        let mut session = provider.read(&b.session).await?;
        engine::runtimes::require_workspace(&session.cwd, project)?;
        if input.files.is_empty() && matches!(text, "/status" | "/history") {
            return if text == "/status" {
                self.store
                    .output(
                        &Destination {
                            chat: b.chat.clone(),
                            root: Some(b.root.clone()),
                        },
                        &[self.status_card(&snapshot, &b, &session)?],
                        hash,
                        Some(&b),
                        None,
                        &[],
                    )
                    .await
            } else {
                let history =
                    crate::questions::history(provider.as_ref(), &b.session, &self.locale())
                        .await?;
                self.conversation(
                    &Destination {
                        chat: b.chat.clone(),
                        root: Some(b.root.clone()),
                    },
                    session::presentation::latest(&history, &self.locale()),
                    hash,
                    Some(&b),
                )
                .await
            };
        }
        if text.is_empty() && input.files.is_empty() {
            return Err(Error::Ui("feishu.error.empty"));
        }
        if !provider.capabilities().control_verified {
            return Err(Error::Ui("feishu.error.read_only"));
        }
        if let Some(mutation) =
            crate::questions::prepare(provider.as_ref(), &b.session, text, !input.files.is_empty())
                .await?
        {
            self.mutate(
                input,
                hash,
                project,
                Some(&b),
                &b.session,
                "answer",
                mutation,
            )
            .await?;
            return Ok(());
        }
        if session.status == RuntimeStatus::NotLoaded {
            self.mutate(
                input,
                hash,
                project,
                Some(&b),
                &b.session,
                "resume",
                Mutation::Resume {
                    thread: b.session.clone(),
                },
            )
            .await?;
            session = provider.read(&b.session).await?;
            engine::runtimes::require_workspace(&session.cwd, project)?;
        }
        let client_id = format!(
            "pp:feishu:{}:{}",
            engine::digest(&self.store.namespace),
            engine::digest(&input.id)
        );
        let mutation = match session.status {
            RuntimeStatus::Idle => Mutation::Start {
                thread: b.session.clone(),
                text: text.into(),
                client_id,
            },
            RuntimeStatus::Active { .. } => Mutation::Steer {
                thread: b.session.clone(),
                text: text.into(),
                client_id,
                expected_turn: match session.active_turn {
                    Some(id) => id,
                    None => provider
                        .active_turn(&b.session)
                        .await?
                        .ok_or(Error::Ui("feishu.error.state"))?,
                },
            },
            _ => return Err(Error::Ui("feishu.error.state")),
        };
        if input.files.len() > session::files::MAX_FILES {
            return Err(Error::Ui("common.file.failed"));
        }
        let mut downloaded = Vec::new();
        for file in &input.files {
            downloaded.push(self.api.download(file).await?);
        }
        let instance = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == b.instance)
            .ok_or(Error::Ui("feishu.error.scope"))?;
        let (uploads, files) = engine::uploads::prepare(
            instance,
            &format!("{}:{}", self.store.namespace, input.id),
            &b.session,
            downloaded,
        )?;
        for (index, mutation) in uploads.into_iter().enumerate() {
            self.mutate(
                input,
                hash,
                project,
                Some(&b),
                &b.session,
                &format!("upload:{index}"),
                mutation,
            )
            .await?;
        }
        self.mutate(
            input,
            hash,
            project,
            Some(&b),
            &b.session,
            "input",
            engine::uploads::attach(mutation, files),
        )
        .await?;
        Ok(())
    }
}
