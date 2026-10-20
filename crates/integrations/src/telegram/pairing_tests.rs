use super::*;

#[tokio::test]
async fn cli_approval_works_while_service_owns_the_ledger_without_recovering_operations() {
    let dir = tempfile::tempdir().unwrap();
    let service = Store::open(dir.path()).await.unwrap();
    let id = service
        .reserve_operation("live-operation", "host", "thread")
        .await
        .unwrap();
    use session::operation::OperationState as State;
    service
        .transition(&id, State::Received, State::Prepared)
        .await
        .unwrap();
    service
        .transition(&id, State::Prepared, State::Dispatching)
        .await
        .unwrap();
    let code = service
        .pairing()
        .request_pairing(1, 10, 10, true, 100)
        .await
        .unwrap();
    let cli = Registry::open(dir.path()).await.unwrap();
    cli.approve_pairing(&code, 101).await.unwrap();
    assert!(service.pairing().is_paired(1, 10, 10).await.unwrap());
    assert_eq!(
        service.operation_state(&id).await.unwrap(),
        State::Dispatching
    );
}

#[tokio::test]
async fn only_cli_approval_grants_access_and_codes_are_one_time_and_bot_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    assert!(matches!(
        store
            .pairing()
            .request_pairing(1, 10, -10, false, 100)
            .await,
        Err(Error::PrivateOnly)
    ));
    let code = store
        .pairing()
        .request_pairing(1, 10, 10, true, 100)
        .await
        .unwrap();
    assert!(!store.pairing().is_paired(1, 10, 10).await.unwrap());
    store.pairing().approve_pairing(&code, 101).await.unwrap();
    assert!(store.pairing().is_paired(1, 10, 10).await.unwrap());
    assert!(!store.pairing().is_paired(2, 10, 10).await.unwrap());
    assert!(matches!(
        store.pairing().approve_pairing(&code, 102).await,
        Err(Error::InvalidCode)
    ));
    store.pairing().revoke_pairing(1, 10).await.unwrap();
    assert!(!store.pairing().is_paired(1, 10, 10).await.unwrap());
}

#[tokio::test]
async fn expired_codes_cannot_be_approved() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let code = store
        .pairing()
        .request_pairing(1, 10, 10, true, 100)
        .await
        .unwrap();
    assert!(matches!(
        store.pairing().approve_pairing(&code, 700).await,
        Err(Error::InvalidCode)
    ));
}
