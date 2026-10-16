//! Shared receipt and uncertainty rules, after a platform authorizes dispatch.
use crate::storage::{Store, StoreError};
use session::{
    AgentProvider, ProviderError,
    operation::OperationState as State,
    provider::{Mutation, Receipt},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("request outcome is unknown; inspect the session before sending again")]
    Unknown,
}

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Unknown => ::i18n::Message::new("error.operation.unknown"),
            Self::Store(error) => ::i18n::LocalizedError::message(error),
            Self::Provider(error) => ::i18n::LocalizedError::message(error),
        }
    }
}

/// Executes once, with no retry or cancellation. Platforms retain responsibility
/// for freshness, authorization, binding checks and the atomic dispatch claim.
pub async fn execute_dispatched(
    store: &Store,
    id: &str,
    provider: &dyn AgentProvider,
    mutation: Mutation,
) -> Result<Receipt, Error> {
    if store.operation_state(id).await? != State::Dispatching {
        return Err(StoreError::Transition.into());
    }
    tracing::info!(operation_id = id, "dispatching authorized agent operation");
    match provider.execute(mutation).await {
        Ok(receipt) => {
            store.accept_receipt(id, &receipt).await?;
            tracing::info!(operation_id=id,thread_id=%receipt.thread,turn_id=?receipt.turn,"agent operation accepted");
            Ok(receipt)
        }
        Err(error) => {
            let rejected = matches!(
                error,
                ProviderError::Rejected(_) | ProviderError::Conflict | ProviderError::Unsupported
            );
            store
                .transition(
                    id,
                    State::Dispatching,
                    if rejected {
                        State::Rejected
                    } else {
                        State::OutcomeUnknown
                    },
                )
                .await?;
            tracing::warn!(operation_id=id,definitely_rejected=rejected,%error,"agent operation failed; no replay");
            Err(if rejected {
                Error::Provider(error)
            } else {
                Error::Unknown
            })
        }
    }
}
