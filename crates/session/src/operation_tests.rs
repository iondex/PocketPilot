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
