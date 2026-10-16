use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Received,
    Prepared,
    Dispatching,
    Accepted,
    Rejected,
    OutcomeUnknown,
}

impl OperationState {
    pub fn recover(self) -> Self {
        if self == Self::Dispatching {
            Self::OutcomeUnknown
        } else {
            self
        }
    }

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
mod tests {
    use super::*;

    #[test]
    fn restart_never_makes_a_sent_request_retryable() {
        assert_eq!(
            OperationState::Dispatching.recover(),
            OperationState::OutcomeUnknown
        );
        assert!(!OperationState::OutcomeUnknown.can_transition(OperationState::Prepared));
        assert!(!OperationState::OutcomeUnknown.can_transition(OperationState::Dispatching));
    }
}
