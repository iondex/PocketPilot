//! Explicit operator-only verification using a dedicated, durably recorded probe.
use crate::{config::Config, operations, storage::Store};
use session::{
    AgentProvider, RuntimeStatus,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};
use std::time::Duration;

const MARKER: &str = "POCKETPILOT_SHARED_OK";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Rpc(#[from] adapters::codex::Error),
    #[error(transparent)]
    Store(#[from] crate::storage::StoreError),
    #[error(transparent)]
    Operation(#[from] operations::Error),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error("shared lifecycle verification failed; control remains unchanged")]
    Check,
}

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        match self {
            Self::Rpc(e) => e.message(),
            Self::Store(e) => e.message(),
            Self::Operation(e) => e.message(),
            Self::Provider(e) => e.message(),
            Self::Check => i18n::Message::new("cli.control.failed"),
        }
    }
}

#[derive(serde::Serialize)]
pub struct Verified {
    pub provider: String,
    pub thread: String,
    pub shared_active_turn: bool,
    pub steer_accepted: bool,
    pub disconnect_survived: bool,
    pub final_observed: bool,
    pub upload_ready: bool,
}

pub async fn verify(
    config: &Config,
    store: &Store,
    id: &str,
    project: &str,
) -> Result<Verified, Error> {
    let provider = config
        .providers
        .iter()
        .find(|p| p.id == id)
        .ok_or(Error::Check)?;
    let host = config
        .hosts
        .iter()
        .find(|h| h.id == provider.host_id)
        .ok_or(Error::Check)?;
    let mut projects = config
        .workspaces
        .iter()
        .filter(|p| p.provider_id == id && (p.id == project || p.short_name == project));
    let workspace = projects.next().ok_or(Error::Check)?;
    if projects.next().is_some() {
        return Err(Error::Check);
    }
    let writer = adapters::codex::Codex::new(
        id.into(),
        crate::runtimes::connect(host, provider, config).await?,
    )
    .with_control(true);
    let observer = adapters::codex::Codex::new(
        id.into(),
        crate::runtimes::connect(host, provider, config).await?,
    );
    let source = format!("probe:{}", uuid::Uuid::now_v7());
    let created = dispatch(
        store,
        &writer,
        &source,
        "create",
        id,
        "__probe",
        Mutation::Create {
            cwd: workspace.cwd.clone(),
        },
    )
    .await?;
    let thread = created.thread;
    if observer.read(&thread).await?.cwd != workspace.cwd {
        return Err(Error::Check);
    }
    let (uploads, _) = crate::uploads::prepare(
        provider,
        &source,
        &thread,
        vec![session::files::Downloaded {
            name: "probe.txt".into(),
            bytes: b"PocketPilot attachment probe".to_vec(),
            image: false,
        }],
    )?;
    for (index, mutation) in uploads.into_iter().enumerate() {
        dispatch(
            store,
            &writer,
            &source,
            &format!("upload:{index}"),
            id,
            &thread,
            mutation,
        )
        .await?;
    }
    let started = dispatch(store, &writer, &source, "start", id, &thread, Mutation::Start {
        thread: thread.clone(),
        text: "This is a PocketPilot connection verification. Do not use tools, access files or change anything. Print the integers 1 to 1000, one per line. A follow-up instruction will arrive during generation.".into(),
        client_id: format!("{source}:start"),
    }).await?;
    let turn = started.turn.ok_or(Error::Check)?;
    if observer.active_turn(&thread).await?.as_deref() != Some(&turn) {
        return Err(Error::Check);
    }
    let steered = dispatch(store, &writer, &source, "steer", id, &thread, Mutation::Steer {
        thread: thread.clone(), expected_turn: turn.clone(),
        text: format!("Verification complete. Stop listing integers. Do not use tools. Finish with exactly {MARKER}."),
        client_id: format!("{source}:steer"),
    }).await?;
    if steered.turn.as_deref() != Some(&turn) {
        return Err(Error::Check);
    }
    drop(writer);
    drop(observer);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let observer = adapters::codex::Codex::new(
        id.into(),
        crate::runtimes::connect(host, provider, config).await?,
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let state = observer.read(&thread).await?;
        match state.status {
            RuntimeStatus::Idle => break,
            RuntimeStatus::Active {
                waiting_on_approval: false,
                waiting_on_user_input: false,
            } => {
                if observer.active_turn(&thread).await?.as_deref() != Some(&turn) {
                    return Err(Error::Check);
                }
            }
            _ => return Err(Error::Check),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Check);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let history = observer.history(&thread).await?;
    if !history.all_pages_read
        || !history
            .items
            .iter()
            .any(|i| i.role == session::Role::Agent && i.text.contains(MARKER) && i.complete)
    {
        return Err(Error::Check);
    }
    Ok(Verified {
        provider: id.into(),
        thread,
        shared_active_turn: true,
        steer_accepted: true,
        disconnect_survived: true,
        final_observed: true,
        upload_ready: true,
    })
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    store: &Store,
    provider: &dyn AgentProvider,
    source: &str,
    step: &str,
    instance: &str,
    thread: &str,
    mutation: Mutation,
) -> Result<Receipt, Error> {
    let id = store
        .reserve_operation(&format!("{source}:{step}"), instance, thread)
        .await?;
    store
        .transition(&id, State::Received, State::Prepared)
        .await?;
    store
        .transition(&id, State::Prepared, State::Dispatching)
        .await?;
    Ok(operations::execute_dispatched(store, &id, provider, mutation).await?)
}
