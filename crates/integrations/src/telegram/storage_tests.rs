use super::*;
use session::operation::OperationState;

#[tokio::test]
async fn a_second_writer_is_rejected_and_an_input_cannot_change_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    assert!(matches!(
        Store::open(dir.path()).await,
        Err(StoreError::Locked)
    ));
    store
        .reserve_operation("same-message", "host", "original")
        .await
        .unwrap();
    assert!(matches!(
        store
            .reserve_operation("same-message", "host", "other")
            .await,
        Err(StoreError::SourceConflict)
    ));
}

#[tokio::test]
async fn duplicate_inputs_reuse_operations_and_restart_preserves_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let id = store
        .reserve_operation("message:1", "host", "thread")
        .await
        .unwrap();
    assert_eq!(
        store
            .reserve_operation("message:1", "host", "thread")
            .await
            .unwrap(),
        id
    );
    store
        .transition(&id, OperationState::Received, OperationState::Prepared)
        .await
        .unwrap();
    store
        .transition(&id, OperationState::Prepared, OperationState::Dispatching)
        .await
        .unwrap();
    store.pool.close().await;
    drop(store);
    let recovered = Store::open(dir.path()).await.unwrap();
    assert_eq!(
        recovered.operation_state(&id).await.unwrap(),
        OperationState::OutcomeUnknown
    );
    assert!(
        recovered
            .transition(
                &id,
                OperationState::OutcomeUnknown,
                OperationState::Dispatching
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn private_containers_are_bot_scoped_and_never_retarget_within_an_account() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    store.bind(1, 10, 1, "host", "original").await.unwrap();
    store.bind(2, 10, 1, "host", "different").await.unwrap();
    assert!(matches!(
        store.bind(2, 10, 1, "host", "retargeted").await,
        Err(StoreError::ContainerConflict)
    ));
    assert!(matches!(
        store.bind(1, -10, 1, "host", "original").await,
        Err(StoreError::Unauthorized)
    ));
}
