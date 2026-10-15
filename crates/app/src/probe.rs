//! Explicit operator probe on a NEW thread. Never writes to an existing thread.
use crate::{config::Config, connections, storage::Store};
use codex::Codex;
use serde::Serialize;
use session::{
    AgentProvider, RuntimeStatus, SessionQuery,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Connection(#[from] connections::Error),
    #[error(transparent)]
    Rpc(#[from] codex::Error),
    #[error(transparent)]
    Provider(#[from] session::ProviderError),
    #[error(transparent)]
    Store(#[from] crate::storage::StoreError),
    #[error("probe write outcome is unknown; no automatic retry was attempted")]
    Unknown,
}

#[derive(Serialize)]
pub struct Report {
    pub sample_cli_version: Option<String>,
    pub history_mode: Option<String>,
    pub schema_cli: &'static str,
    pub workspace: String,
    pub write_probe: bool,
    pub thread: Option<String>,
    pub turn: Option<String>,
    pub active_observed: bool,
    pub steer_accepted: bool,
    pub status_before_disconnect: Option<RuntimeStatus>,
    pub status_after_disconnect: Option<RuntimeStatus>,
    pub completed: bool,
    pub final_marker_found: bool,
    pub resume_accepted: bool,
    pub note: &'static str,
}

async fn execute(
    store: &Store,
    provider: &dyn AgentProvider,
    source: &str,
    instance: &str,
    thread: &str,
    mutation: Mutation,
) -> Result<Receipt, Error> {
    let id = store.reserve_operation(source, instance, thread).await?;
    store
        .transition(&id, State::Received, State::Prepared)
        .await?;
    store
        .transition(&id, State::Prepared, State::Dispatching)
        .await?;
    match provider.execute(mutation).await {
        Ok(receipt) => {
            store.accept_receipt(&id, &receipt).await?;
            Ok(receipt)
        }
        Err(error) => {
            let rejected = matches!(
                error,
                session::ProviderError::Rejected(_)
                    | session::ProviderError::Conflict
                    | session::ProviderError::Unsupported
            );
            store
                .transition(
                    &id,
                    State::Dispatching,
                    if rejected {
                        State::Rejected
                    } else {
                        State::OutcomeUnknown
                    },
                )
                .await?;
            Err(if rejected {
                error.into()
            } else {
                Error::Unknown
            })
        }
    }
}

pub async fn run(
    config: &Config,
    workspace_id: &str,
    write: bool,
    inspect: Option<&str>,
) -> Result<Report, Error> {
    if write && inspect.is_some() {
        return Err(connections::Error::Selection.into());
    }
    let workspace = config
        .workspaces
        .iter()
        .find(|w| w.id == workspace_id || w.short_name == workspace_id)
        .ok_or(connections::Error::Selection)?;
    let provider = config
        .providers
        .iter()
        .find(|p| p.id == workspace.provider_id)
        .ok_or(connections::Error::Selection)?;
    let host = config
        .hosts
        .iter()
        .find(|h| h.id == provider.host_id)
        .ok_or(connections::Error::Selection)?;
    let first = connections::connect(host, config).await?;
    let writer = Codex::new(provider.id.clone(), first).with_control(write);
    let mut report = Report {
        sample_cli_version: None,
        history_mode: None,
        schema_cli: codex::protocol::SCHEMA_CLI_VERSION,
        workspace: workspace.id.clone(),
        write_probe: write,
        thread: None,
        turn: None,
        active_observed: false,
        steer_accepted: false,
        status_before_disconnect: None,
        status_after_disconnect: None,
        completed: false,
        final_marker_found: false,
        resume_accepted: false,
        note: "Read-only connectivity check; no session operation was sent.",
    };
    if !write {
        let page = writer
            .list(SessionQuery {
                cwd: Some(workspace.cwd.clone()),
                limit: 1,
                ..Default::default()
            })
            .await?;
        let selected = inspect
            .map(str::to_owned)
            .or_else(|| page.data.first().map(|s| s.reference.thread.clone()));
        if let Some(thread) = selected {
            let metadata = writer
                .client()
                .call(&codex::protocol::ThreadReadParams {
                    thread_id: thread.clone(),
                    include_turns: Some(false),
                })
                .await?;
            connections::require_workspace(&metadata.thread.cwd.to_string(), workspace)?;
            report.thread = Some(thread);
            report.sample_cli_version = Some(metadata.thread.cli_version);
            report.history_mode = Some(metadata.thread.history_mode.to_string());
            report.status_before_disconnect =
                Some(codex::provider::runtime(&metadata.thread.status));
        }
        return Ok(report);
    }
    // --write is an explicit local operator action, independently of the bot's
    // control flag. It exists to certify a previously read-only deployment.
    let store = Store::open(&config.service.data_dir).await?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let second = Codex::new(
        provider.id.clone(),
        connections::connect(host, config).await?,
    );
    let created = execute(
        &store,
        &writer,
        &format!("probe:{run_id}:create"),
        &provider.id,
        "__new",
        Mutation::Create {
            cwd: workspace.cwd.clone(),
        },
    )
    .await?;
    let thread = created.thread;
    report.thread = Some(thread.clone());
    let metadata = writer
        .client()
        .call(&codex::protocol::ThreadReadParams {
            thread_id: thread.clone(),
            include_turns: Some(false),
        })
        .await?;
    report.sample_cli_version = Some(metadata.thread.cli_version);
    report.history_mode = Some(metadata.thread.history_mode.to_string());
    tracing::info!(thread_id=%thread,"created dedicated operator probe thread");
    let prompt = "This is a PocketPilot integration probe. Do not use tools, read files, or change files. Output 120 numbered short lines about connection stability. The final line must be PP_PROBE_OK.";
    let started = execute(
        &store,
        &writer,
        &format!("probe:{run_id}:start"),
        &provider.id,
        &thread,
        Mutation::Start {
            thread: thread.clone(),
            text: prompt.into(),
            client_id: format!("probe:{run_id}:input"),
        },
    )
    .await?;
    report.turn = started.turn.clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let before = loop {
        let before = second.read(&thread).await?;
        connections::require_workspace(&before.cwd, workspace)?;
        if matches!(
            before.status,
            RuntimeStatus::Active { .. } | RuntimeStatus::SystemError
        ) || tokio::time::Instant::now() >= deadline
        {
            break before;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    report.active_observed = matches!(before.status, RuntimeStatus::Active { .. });
    let active_turn = match before.active_turn {
        Some(turn) => Some(turn),
        None => second.active_turn(&thread).await?,
    };
    if let Some(turn) = active_turn
        && started.turn.as_deref() == Some(&turn)
    {
        let steered = execute(
            &store,
            &writer,
            &format!("probe:{run_id}:steer"),
            &provider.id,
            &thread,
            Mutation::Steer {
                thread: thread.clone(),
                expected_turn: turn,
                text: "Continue without tools; keep PP_PROBE_OK as the final line.".into(),
                client_id: format!("probe:{run_id}:steer-input"),
            },
        )
        .await?;
        report.steer_accepted = steered.turn == started.turn;
    }
    drop(writer);
    report.status_before_disconnect = Some(second.read(&thread).await?.status);
    drop(second);
    // Fresh SSH/proxy connection. Reconnection performs initialize and reads only.
    let reader = Codex::new(
        provider.id.clone(),
        connections::connect(host, config).await?,
    );
    report.status_after_disconnect = Some(reader.read(&thread).await?.status);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let current = reader.read(&thread).await?;
        if current.status == RuntimeStatus::Idle {
            report.completed = true;
            break;
        }
        if matches!(
            current.status,
            RuntimeStatus::SystemError | RuntimeStatus::NotLoaded
        ) || tokio::time::Instant::now() >= deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let history = reader.history(&thread).await?;
    report.final_marker_found = history.all_pages_read
        && history.items.iter().any(|i| {
            i.role == session::Role::Agent
                && i.phase.as_deref() == Some("final_answer")
                && i.text.contains("PP_PROBE_OK")
        });
    let final_turn = last_turn(&reader, &thread).await?;
    report.completed = final_turn.as_ref().is_some_and(|t| {
        Some(&t.id) == started.turn.as_ref()
            && matches!(t.status, codex::protocol::TurnStatus::Completed)
    });
    if report.completed
        && matches!(
            reader.read(&thread).await?.status,
            RuntimeStatus::Idle | RuntimeStatus::NotLoaded
        )
    {
        execute(
            &store,
            &reader.clone().with_control(true),
            &format!("probe:{run_id}:resume"),
            &provider.id,
            &thread,
            Mutation::Resume {
                thread: thread.clone(),
            },
        )
        .await?;
        report.resume_accepted = true;
    }
    report.note = "Only the newly created probe thread received input. Existing threads were not resumed, steered, stopped or restarted. The probe thread remains available for inspection.";
    Ok(report)
}

async fn last_turn(provider: &Codex, thread: &str) -> Result<Option<codex::protocol::Turn>, Error> {
    use codex::protocol as p;
    let metadata = provider
        .client()
        .call(&p::ThreadReadParams {
            thread_id: thread.into(),
            include_turns: Some(false),
        })
        .await?;
    if matches!(metadata.thread.history_mode, p::ThreadHistoryMode::Legacy) {
        let full = provider
            .client()
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(true),
            })
            .await?;
        Ok(full.thread.turns.into_iter().last())
    } else {
        let page = provider
            .client()
            .call(&p::ThreadTurnsListParams {
                thread_id: thread.into(),
                cursor: None,
                limit: Some(1),
                sort_direction: Some(p::SortDirection::Desc),
                items_view: Some(p::TurnItemsView::NotLoaded),
            })
            .await?;
        Ok(page.data.into_iter().next())
    }
}
