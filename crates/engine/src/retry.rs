//! The opt-in retry policy. Ordinary background observers remain read-only.
use crate::{
    config::Config,
    operations,
    reload::ConfigFile,
    runtimes::RuntimeRegistry,
    storage::{Store, StoreError},
};
use session::{
    AgentProvider, RuntimeStatus, Session, operation::OperationState as State, provider::Mutation,
    retry::RetryableFailure,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path},
    time::Duration,
};
use tokio::time::Instant;

pub const DELAY: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Runtime(#[from] crate::runtimes::Error),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error(transparent)]
    Operation(#[from] operations::Error),
}

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        match self {
            Self::Store(error) => error.message(),
            Self::Runtime(error) => error.message(),
            Self::Provider(error) => error.message(),
            Self::Operation(error) => error.message(),
        }
    }
}

struct Pending {
    failure: RetryableFailure,
    operation: String,
    due: Instant,
}

pub struct Monitor {
    authorization: String,
    pending: HashMap<String, Pending>,
    statuses: HashMap<String, RuntimeStatus>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            authorization: uuid::Uuid::now_v7().to_string(),
            pending: HashMap::new(),
            statuses: HashMap::new(),
        }
    }
}

impl Monitor {
    /// Enumerate loaded and stored sessions on every pass. No chat binding is required.
    pub async fn scan(
        &mut self,
        config: &ConfigFile,
        runtimes: &RuntimeRegistry,
        store: &Store,
        instance: &str,
    ) -> Result<(), Error> {
        let snapshot = config.snapshot();
        let provider = runtimes.get_provider(&snapshot.config, instance).await?;
        self.scan_provider(config, runtimes, store, instance, provider.as_ref())
            .await
    }

    async fn scan_provider(
        &mut self,
        config: &ConfigFile,
        runtimes: &RuntimeRegistry,
        store: &Store,
        instance: &str,
        provider: &dyn AgentProvider,
    ) -> Result<(), Error> {
        let snapshot = config.snapshot();
        if !provider.capabilities().control_verified {
            tracing::debug!(
                provider_id = instance,
                "retry writes disabled for unverified provider"
            );
        }
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let page = provider
                .list(session::SessionQuery {
                    cursor,
                    limit: 100,
                    ..Default::default()
                })
                .await?;
            for session in page.data {
                let thread = session.reference.thread.clone();
                if !seen.insert(thread.clone()) {
                    continue;
                }
                if config.snapshot().hash != snapshot.hash {
                    return Ok(());
                }
                if self.statuses.get(&thread) != Some(&session.status) {
                    tracing::info!(provider_id=instance,thread_id=%thread,status=?session.status,"retry monitor observed session state");
                    self.statuses.insert(thread.clone(), session.status.clone());
                }
                if !allowed(&snapshot.config, instance, &session)
                    || session.status != RuntimeStatus::Idle
                    || !provider.capabilities().control_verified
                {
                    self.cancel(store, &thread).await?;
                    continue;
                }
                let actor = runtimes.actor(instance, &thread).await;
                let _guard = actor.lock().await;
                if let Err(error) = self
                    .inspect(config, store, provider, &session, &snapshot.hash)
                    .await
                {
                    tracing::warn!(provider_id=instance,thread_id=%thread,%error,"retry inspection failed; no replay");
                }
            }
            match page.next_cursor {
                None => {
                    self.statuses.retain(|thread, _| {
                        let present = seen.contains(thread);
                        if !present {
                            tracing::info!(provider_id=instance,thread_id=%thread,"retry monitor no longer sees session");
                        }
                        present
                    });
                    let missing: Vec<_> = self
                        .pending
                        .keys()
                        .filter(|id| !seen.contains(*id))
                        .cloned()
                        .collect();
                    for thread in missing {
                        self.cancel(store, &thread).await?;
                    }
                    return Ok(());
                }
                Some(next) if cursors.insert(next.clone()) => cursor = Some(next),
                _ => return Err(session::ProviderError::Incompatible.into()),
            }
        }
        Err(session::ProviderError::Incompatible.into())
    }

    async fn inspect(
        &mut self,
        config: &ConfigFile,
        store: &Store,
        provider: &dyn AgentProvider,
        session: &Session,
        hash: &str,
    ) -> Result<(), Error> {
        let thread = &session.reference.thread;
        let Some(failure) = provider.retryable_failure(thread).await? else {
            self.cancel(store, thread).await?;
            return Ok(());
        };
        self.observe(store, session, failure, hash, Instant::now())
            .await?;
        let Some(pending) = self.pending.get(thread) else {
            return Ok(());
        };
        if pending.due > Instant::now() {
            return Ok(());
        }
        let snapshot = config.snapshot();
        if !allowed(&snapshot.config, &session.reference.instance, session) {
            self.cancel(store, thread).await?;
            return Ok(());
        }
        // Refresh cwd, control and latest failed turn immediately before claiming a write.
        let current = provider.read(thread).await?;
        let failure = provider.retryable_failure(thread).await?;
        if config.snapshot().hash != snapshot.hash {
            // Recheck against the new snapshot on the next pass. An unrelated catalog
            // update must not reject the operation or restart its minute-long delay.
            return Ok(());
        }
        if current.status != RuntimeStatus::Idle
            || !allowed(&snapshot.config, &session.reference.instance, &current)
            || !provider.capabilities().control_verified
            || failure.as_ref() != Some(&pending.failure)
        {
            self.cancel(store, thread).await?;
            return Ok(());
        }
        self.dispatch(store, provider, thread).await
    }

    async fn observe(
        &mut self,
        store: &Store,
        session: &Session,
        failure: RetryableFailure,
        hash: &str,
        now: Instant,
    ) -> Result<(), Error> {
        let thread = &session.reference.thread;
        if self
            .pending
            .get(thread)
            .is_some_and(|p| p.failure == failure)
        {
            return Ok(());
        }
        self.cancel(store, thread).await?;
        let source = format!(
            "retry:{}",
            crate::digest(
                &serde_json::to_string(&(&session.reference, &failure.turn_id,))
                    .map_err(|_| StoreError::Database)?
            )
        );
        let operation = store
            .reserve_operation(&source, &session.reference.instance, thread)
            .await?;
        let state = store.operation_state(&operation).await?;
        if !matches!(state, State::Received | State::Prepared) {
            return Ok(());
        }
        #[derive(serde::Serialize)]
        struct Authorization<'a> {
            kind: &'static str,
            monitor: &'a str,
            config_hash: &'a str,
            failed_turn: &'a str,
            reason: session::retry::RetryReason,
            delay_seconds: u64,
            text: &'static str,
        }
        let payload = serde_json::to_string(&Authorization {
            kind: "retry_monitor",
            monitor: &self.authorization,
            config_hash: hash,
            failed_turn: &failure.turn_id,
            reason: failure.reason,
            delay_seconds: DELAY.as_secs(),
            text: "继续",
        })
        .map_err(|_| StoreError::Database)?;
        sqlx::query(
            "UPDATE operations SET payload=? WHERE id=? AND state IN ('received','prepared')",
        )
        .bind(payload)
        .bind(&operation)
        .execute(store.pool())
        .await
        .map_err(|_| StoreError::Database)?;
        if state == State::Received {
            store
                .transition(&operation, State::Received, State::Prepared)
                .await?;
        }
        tracing::info!(provider_id=%session.reference.instance,thread_id=%thread,turn_id=%failure.turn_id,reason=?failure.reason,delay_seconds=60,"retry scheduled");
        self.pending.insert(
            thread.clone(),
            Pending {
                failure,
                operation,
                due: now + DELAY,
            },
        );
        Ok(())
    }

    async fn cancel(&mut self, store: &Store, thread: &str) -> Result<(), Error> {
        if let Some(pending) = self.pending.remove(thread) {
            store
                .transition(&pending.operation, State::Prepared, State::Rejected)
                .await?;
            tracing::info!(thread_id=thread,turn_id=%pending.failure.turn_id,"retry cancelled after session or authorization changed");
        }
        Ok(())
    }

    async fn dispatch(
        &mut self,
        store: &Store,
        provider: &dyn AgentProvider,
        thread: &str,
    ) -> Result<(), Error> {
        let Some(pending) = self.pending.remove(thread) else {
            return Ok(());
        };
        store
            .transition(&pending.operation, State::Prepared, State::Dispatching)
            .await?;
        operations::execute_dispatched(
            store,
            &pending.operation,
            provider,
            Mutation::Start {
                thread: thread.into(),
                text: "继续".into(),
                client_id: pending.operation.clone(),
            },
        )
        .await?;
        Ok(())
    }
}

fn allowed(config: &Config, instance: &str, session: &Session) -> bool {
    let Some(provider) = config
        .providers
        .iter()
        .find(|p| p.id == instance && p.control_verified)
    else {
        return false;
    };
    let Some(host) = config.hosts.iter().find(|h| h.id == provider.host_id) else {
        return false;
    };
    let cwd = Path::new(&session.cwd);
    session.reference.instance == instance
        && cwd.is_absolute()
        && !cwd.components().any(|c| matches!(c, Component::ParentDir))
        && host
            .allowed_workspace_roots
            .iter()
            .any(|root| cwd.starts_with(root))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
