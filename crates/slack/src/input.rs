use super::*;
use session::{
    RuntimeStatus,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};

impl Service {
    pub(super) async fn mutate(
        &self,
        r: &Request,
        hash: &str,
        ws: &engine::config::Workspace,
        thread: &str,
        key: &str,
        mutation: Mutation,
    ) -> Result<Receipt> {
        let snapshot = self.fresh(r, hash)?;
        if r.date < self.cutoff.load(Ordering::SeqCst)
            || !(0..=300).contains(&engine::now().saturating_sub(r.date))
        {
            return Err(Error::Ui("slack.error.stale"));
        }
        let provider = self.context.connections.get(&snapshot.config, ws).await?;
        if !provider.capabilities().control_verified {
            return Err(Error::Ui("slack.error.read_only"));
        }
        let id = self
            .store
            .core
            .reserve_operation(
                &format!("slack:{}:{}:{key}", self.store.namespace, r.id),
                &ws.provider_id,
                thread,
            )
            .await?;
        let state = self.store.core.operation_state(&id).await?;
        if state == State::Accepted {
            return self
                .store
                .core
                .receipt(&id)
                .await?
                .ok_or(Error::Ui("slack.error.unknown"));
        }
        if state != State::Received {
            return Err(Error::Ui("slack.error.unknown"));
        }
        self.store
            .core
            .transition(&id, State::Received, State::Prepared)
            .await?;
        self.api.channel(&r.channel).await?;
        self.fresh(r, hash)?;
        if r.binding.is_some() {
            self.bound(r).await?;
        }
        self.store
            .core
            .transition(&id, State::Prepared, State::Dispatching)
            .await?;
        Ok(engine::operations::execute_dispatched(
            &self.store.core,
            &id,
            provider.as_ref(),
            mutation,
        )
        .await?)
    }

    pub(super) async fn input(&self, r: &Request, hash: &str, text: &str) -> Result<()> {
        let snapshot = self.fresh(r, hash)?;
        let b = self.bound(r).await?;
        let ws = self.validate(&snapshot, &b)?;
        let actor = self.context.connections.actor(&b.instance, &b.thread).await;
        let _guard = actor.lock().await;
        let provider = self.context.connections.get(&snapshot.config, ws).await?;
        let mut current = provider.read(&b.thread).await?;
        engine::connections::require_workspace(&current.cwd, ws)?;
        if current.status == RuntimeStatus::NotLoaded {
            self.mutate(
                r,
                hash,
                ws,
                &b.thread,
                "resume",
                Mutation::Resume {
                    thread: b.thread.clone(),
                },
            )
            .await?;
            current = provider.read(&b.thread).await?;
            engine::connections::require_workspace(&current.cwd, ws)?;
        }
        let client_id = format!("pp:slack:{}:{}", self.store.namespace, r.id);
        let mutation = match current.status {
            RuntimeStatus::Idle => Mutation::Start {
                thread: b.thread.clone(),
                text: text.into(),
                client_id,
            },
            RuntimeStatus::Active { .. } => Mutation::Steer {
                thread: b.thread.clone(),
                text: text.into(),
                client_id,
                expected_turn: match current.active_turn {
                    Some(id) => id,
                    None => provider
                        .active_turn(&b.thread)
                        .await?
                        .ok_or(Error::Ui("slack.error.state"))?,
                },
            },
            _ => return Err(Error::Ui("slack.error.state")),
        };
        if !provider.capabilities().control_verified {
            return Err(Error::Ui("slack.error.read_only"));
        }
        if r.uploads.len() > session::files::MAX_FILES {
            return Err(Error::Ui("common.file.failed"));
        }
        let mut downloaded = Vec::new();
        for file in &r.uploads {
            downloaded.push(self.api.download(file).await?);
        }
        let instance = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == b.instance)
            .ok_or(Error::Ui("slack.error.scope"))?;
        let (uploads, files) = engine::uploads::prepare(
            instance,
            &format!("slack:{}:{}", self.store.namespace, r.id),
            &b.thread,
            downloaded,
        )?;
        for (index, mutation) in uploads.into_iter().enumerate() {
            self.mutate(r, hash, ws, &b.thread, &format!("upload:{index}"), mutation)
                .await?;
        }
        self.mutate(
            r,
            hash,
            ws,
            &b.thread,
            "input",
            engine::uploads::attach(mutation, files),
        )
        .await?;
        Ok(())
    }
}
