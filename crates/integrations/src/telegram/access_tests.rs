use super::*;

#[tokio::test]
async fn designated_user_enters_without_pairing_and_other_messages_or_callbacks_are_ignored() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(7000, 0, "/start"))
        .await
        .unwrap();
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|body| body.navigation_id.is_some())
    );
    let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('pairings','pairing_requests')").fetch_one(fixture.service.store.pool()).await.unwrap();
    assert_eq!(
        tables, 0,
        "authorization no longer depends on pairing storage"
    );
    let outputs = fixture.outputs().await.len();
    let native_requests = fixture.telegram.received_requests().await.unwrap().len();
    let mut other = input(7001, 0, "/start");
    other.user = 20;
    other.chat = 20;
    fixture.service.handle(&other).await.unwrap();
    let mut forged = input(7002, 9, "");
    forged.user = 20;
    forged.callback = Some("nav:forged:1".into());
    fixture.service.handle(&forged).await.unwrap();
    assert_eq!(fixture.outputs().await.len(), outputs);
    assert_eq!(
        fixture.telegram.received_requests().await.unwrap().len(),
        native_requests
    );
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn unauthorized_queued_output_is_rejected_before_contacting_telegram() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .store
        .enqueue(
            1,
            20,
            11,
            "unauthorized-output",
            &Outbound::text("must not leave the bridge"),
        )
        .await
        .unwrap();
    fixture.deliver_pending().await;
    let state: String =
        sqlx::query_scalar("SELECT state FROM deliveries WHERE source_key='unauthorized-output'")
            .fetch_one(fixture.service.store.pool())
            .await
            .unwrap();
    assert_eq!(state, "rejected");
    assert!(
        fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        fixture
            .service
            .store
            .bind(1, 20, 11, "instance", "thread")
            .await,
        Err(StoreError::Unauthorized)
    ));
}

#[tokio::test]
async fn replacing_the_owner_rejects_previous_bindings_and_queued_outputs() {
    let mut fixture = Fixture::new().await;
    fixture
        .service
        .store
        .enqueue(
            1,
            10,
            11,
            "previous-owner",
            &Outbound::text("private output"),
        )
        .await
        .unwrap();
    let service = Arc::get_mut(&mut fixture.service).unwrap();
    service.store = Store::new(
        service.store.core().clone(),
        crate::telegram::Owner::new(20).unwrap(),
    );
    fixture.deliver_pending().await;
    let state: String =
        sqlx::query_scalar("SELECT state FROM deliveries WHERE source_key='previous-owner'")
            .fetch_one(fixture.service.store.pool())
            .await
            .unwrap();
    assert_eq!(state, "rejected");
    assert!(
        fixture
            .telegram
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        fixture
            .service
            .store
            .bind(1, 10, 12, "instance", "thread")
            .await,
        Err(StoreError::Unauthorized)
    ));
    fixture
        .service
        .store
        .bind(1, 20, 12, "instance", "thread")
        .await
        .unwrap();
}
