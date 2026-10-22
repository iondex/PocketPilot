//! Durable authorization states, independent of a native agent's turn status.
//!
//! Once dispatch starts, no transition makes an operation sendable again. A
//! positive native receipt may resolve uncertainty; time passing cannot.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Input identity is reserved; no provider write has been claimed.
    Received,
    /// Input is prepared and may be claimed after rechecking current authority.
    Prepared,
    /// One caller owns the durable claim; the remote write may already have happened.
    Dispatching,
    /// A native acceptance receipt is known; the turn itself may still be running.
    Accepted,
    /// Validation or the provider definitely rejected this operation.
    Rejected,
    /// A write may have happened, but no definite acceptance or rejection is known.
    OutcomeUnknown,
}

impl OperationState {
    /// Resolve interrupted dispatch to uncertainty without granting a new send.
    pub fn recover(self) -> Self {
        if self == Self::Dispatching {
            Self::OutcomeUnknown
        } else {
            self
        }
    }

    /// Test a legal edge; persistence must also atomically compare the saved state.
    ///
    /// Unknown outcomes can only become accepted when a receipt is recovered.
    /// They cannot return to prepared/dispatching or become rejected by inference.
    pub fn can_transition(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Received, Self::Prepared | Self::Rejected)
                | (Self::Prepared, Self::Dispatching | Self::Rejected)
                | (
                    Self::Dispatching,
                    Self::Accepted | Self::Rejected | Self::OutcomeUnknown
                )
                | (Self::OutcomeUnknown, Self::Accepted)
        )
    }
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
