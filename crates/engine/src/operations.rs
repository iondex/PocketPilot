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

/// Execute one claimed mutation and persist its receipt before reporting success.
///
/// The platform must first validate freshness, authority and the session binding,
/// then atomically claim `Dispatching` while holding the session actor. Checking
/// the saved state here does not itself acquire that claim.
///
/// # Errors
///
/// Definite rejections finish as `Rejected`. Transport/protocol failures finish
/// as `OutcomeUnknown`; storage failure may leave the dispatch claim unresolved.
/// None of these errors authorizes resending the mutation.
///
/// # Cancellation
///
/// This function adds no retry or timeout wrapper. If its caller drops the future
/// after dispatch, recovery must treat the durable claim as uncertain. A native
/// write can outlive its awaiting caller.
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
            // Acceptance and its native identifiers must survive a process crash
            // before the platform is allowed to acknowledge success to the user.
            store.accept_receipt(id, &receipt).await?;
            tracing::info!(operation_id=id,thread_id=%receipt.thread,turn_id=?receipt.turn,"agent operation accepted");
            Ok(receipt)
        }
        Err(error) => {
            // Only definite failures close the claim as rejected. A failed read
            // of a response is not evidence that the provider skipped the write.
            let rejected = matches!(
                error,
                ProviderError::Rejected(_)
                    | ProviderError::Conflict
                    | ProviderError::Unsupported
                    | ProviderError::InvalidAnswer
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
