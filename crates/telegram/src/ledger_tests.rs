use super::*;

#[tokio::test]
async fn terminal_pairing_delivery_drops_the_plaintext_code() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let mut body = Outbound::text("fixture-once-only-code");
    body.require_pairing = false;
    body.expires_at = Some(600);
    store.enqueue(1, 10, 0, "pairing", &body).await.unwrap();
    let rows = store.pending_deliveries(1, 1).await.unwrap();
    let id = &rows[0].id;
    store
        .delivery_state(id, "pending", "dispatching", None, 0)
        .await
        .unwrap();
    store
        .delivery_state(id, "dispatching", "delivered", Some(1), 0)
        .await
        .unwrap();
    let payload: String = sqlx::query_scalar("SELECT payload FROM deliveries WHERE id=?")
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert!(!payload.contains("fixture-once-only-code"));
}

async fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    let code = store
        .pairing()
        .request_pairing(1, 10, 10, true, 100)
        .await
        .unwrap();
    store.pairing().approve_pairing(&code, 101).await.unwrap();
    (dir, store)
}

fn input(id: u32) -> Input {
    Input {
        update_id: id,
        locale: None,
        event_key: format!("message:10:{id}"),
        user: 10,
        chat: 10,
        topic: 11,
        date: 100,
        text: "hello".into(),
        attachments: Vec::new(),
        callback: None,
        callback_id: None,
        message_id: id as i32,
        config_hash: None,
        requires_confirmation: false,
        reply_to_message_id: None,
    }
}

#[tokio::test]
async fn a_busy_topic_preserves_its_input_order_without_blocking_other_topics() {
    let (_dir, store) = setup().await;
    store.cursor(1, 100).await.unwrap();
    let mut other = input(3);
    other.topic = 12;
    store
        .ingest(
            1,
            1,
            &[(1, Some(input(1))), (2, Some(input(2))), (3, Some(other))],
            100,
        )
        .await
        .unwrap();
    let first = store.claim_available_input(1, &[]).await.unwrap().unwrap();
    assert_eq!(first.id, 1);
    let independent = store
        .claim_available_input(1, &[(10, 11)])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(independent.id, 3);
    assert!(
        store
            .claim_available_input(1, &[(10, 11)])
            .await
            .unwrap()
            .is_none()
    );
    store.finish_input(1, first.epoch, first.id).await.unwrap();
    assert_eq!(
        store
            .claim_available_input(1, &[])
            .await
            .unwrap()
            .unwrap()
            .id,
        2
    );
}

#[tokio::test]
async fn inbox_confirmation_is_durable_and_message_identity_survives_polling_epochs() {
    let (_dir, store) = setup().await;
    let (epoch, offset) = store.cursor(1, 100).await.unwrap();
    assert_eq!(offset, None);
    let first = input(100);
    store
        .ingest(1, epoch, &[(100, Some(first.clone())), (101, None)], 100)
        .await
        .unwrap();
    assert_eq!(store.cursor(1, 101).await.unwrap(), (epoch, Some(102)));
    let received = store.claim_input(1).await.unwrap().unwrap();
    assert_eq!(received.input.event_key, first.event_key);
    store
        .finish_input(1, received.epoch, received.id)
        .await
        .unwrap();
    let (next, offset) = store.cursor(1, 100 + 8 * 24 * 3600).await.unwrap();
    assert_eq!(next, epoch + 1);
    assert_eq!(offset, None);
    store
        .ingest(1, next, &[(100, Some(first))], 100 + 8 * 24 * 3600)
        .await
        .unwrap();
    assert!(store.claim_input(1).await.unwrap().is_none());
    assert_eq!(
        store.cursor(1, 100 + 8 * 24 * 3600 + 1).await.unwrap().0,
        next
    );
}

#[tokio::test]
async fn interrupted_input_is_reported_without_replaying_it_and_unknown_topic_is_not_recreated() {
    let (_dir, store) = setup().await;
    store.cursor(1, 100).await.unwrap();
    store
        .ingest(1, 1, &[(1, Some(input(1)))], 100)
        .await
        .unwrap();
    store.claim_input(1).await.unwrap().unwrap();
    assert!(store.begin_topic(1, 10, "host", "thread").await.unwrap());
    store.recover_inbox(1).await.unwrap();
    assert!(store.claim_input(1).await.unwrap().is_none());
    assert_eq!(store.pending_deliveries(1, 101).await.unwrap().len(), 1);
    assert!(!store.begin_topic(1, 10, "host", "thread").await.unwrap());
    store
        .end_topic(1, 10, "host", "thread", "rejected", None)
        .await
        .unwrap();
    assert!(store.begin_topic(1, 10, "host", "thread").await.unwrap());
}

#[tokio::test]
async fn callbacks_are_single_use_private_peer_and_snapshot_bound() {
    let (_dir, store) = setup().await;
    let mut event = input(1);
    let token = store
        .callback(1, &event, "version1", "action", 100)
        .await
        .unwrap();
    event.callback = Some(token);
    assert!(
        store
            .consume_callback(2, &event, "version1", 101)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .consume_callback(1, &event, "version2", 101)
            .await
            .unwrap()
            .is_none()
    );
    event.topic = 12;
    assert!(
        store
            .consume_callback(1, &event, "version1", 101)
            .await
            .unwrap()
            .is_none()
    );
    event.topic = 11;
    assert_eq!(
        store
            .consume_callback(1, &event, "version1", 101)
            .await
            .unwrap()
            .as_deref(),
        Some("action")
    );
    assert!(
        store
            .consume_callback(1, &event, "version1", 101)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn history_and_observation_cursor_publish_atomically_and_never_after_unbind() {
    let (_dir, store) = setup().await;
    store.bind(1, 10, 11, "host", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let items = BTreeMap::from([("item".into(), "hash".into())]);
    store
        .publish_observation(&binding, &items, "history", &Outbound::text("full history"))
        .await
        .unwrap();
    let current = store.binding(1, 10, 11).await.unwrap().unwrap();
    assert!(current.history_ready);
    assert_eq!(
        serde_json::from_str::<BTreeMap<String, String>>(&current.observed_items).unwrap(),
        items
    );
    store.unbind(1, 10, 11).await.unwrap();
    assert!(
        store
            .publish_observation(
                &binding,
                &items,
                "later",
                &Outbound::text("should not leak")
            )
            .await
            .is_err()
    );
    assert_eq!(store.pending_deliveries(1, 100).await.unwrap().len(), 1);
}

#[tokio::test]
async fn burst_members_share_one_durable_batch_with_a_fixed_deadline() {
    let (dir, store) = setup().await;
    store.bind(1, 10, 11, "host", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let mut items = BTreeMap::new();
    let config = engine::config::Delivery::default();
    for (at, text) in [
        (100000, "one"),
        (101000, "two"),
        (101500, "three"),
        (104000, "four"),
    ] {
        items.insert(text.into(), text.into());
        store
            .publish_batch(&binding, &items, Outbound::text(text), 1, &config, at)
            .await
            .unwrap();
    }
    assert!(store.pending_deliveries(1, 104).await.unwrap().is_empty());
    store.pool.close().await;
    drop(store);
    let restarted = Store::open(dir.path()).await.unwrap();
    let rows = restarted.pending_deliveries(1, 105).await.unwrap();
    assert_eq!(rows.len(), 1);
    let body: Outbound = serde_json::from_str(&rows[0].payload).unwrap();
    for text in ["one", "two", "three", "four"] {
        assert!(body.text.contains(text));
    }
}

#[tokio::test]
async fn revoked_pairing_fails_at_the_dispatch_transaction() {
    let (_dir, store) = setup().await;
    store.bind(1, 10, 11, "host", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let id = store
        .reserve_operation("pending", "host", "thread")
        .await
        .unwrap();
    store
        .transition(
            &id,
            session::operation::OperationState::Received,
            session::operation::OperationState::Prepared,
        )
        .await
        .unwrap();
    store.pairing().revoke_pairing(1, 10).await.unwrap();
    assert!(matches!(
        store
            .dispatch_authorized(&id, 1, &input(1), Some(&binding))
            .await,
        Err(Error::NotPaired)
    ));
    assert_eq!(
        store.operation_state(&id).await.unwrap(),
        session::operation::OperationState::Prepared
    );
}

#[tokio::test]
async fn an_uncertain_delivery_is_not_automatically_sent_again() {
    let (_dir, store) = setup().await;
    store
        .enqueue(1, 10, 11, "once", &Outbound::text("body"))
        .await
        .unwrap();
    let row = store
        .pending_deliveries(1, 100)
        .await
        .unwrap()
        .pop()
        .unwrap();
    store
        .delivery_state(&row.id, "pending", "dispatching", None, 0)
        .await
        .unwrap();
    store
        .delivery_state(&row.id, "dispatching", "outcome_unknown", None, 0)
        .await
        .unwrap();
    store
        .enqueue(1, 10, 11, "once", &Outbound::text("body"))
        .await
        .unwrap();
    assert!(store.pending_deliveries(1, 10000).await.unwrap().is_empty());
}

#[tokio::test]
async fn deferred_icon_reads_allow_text_but_rate_limited_metadata_retains_queue_order() {
    let (_dir, store) = setup().await;
    store.bind(1, 10, 11, "host", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let mut icon = Outbound::text("");
    icon.appearance = Some(crate::presentation::Appearance {
        title: "Synthetic title".into(),
        emoji: "💬".into(),
    });
    store.enqueue_appearance(&binding, &icon).await.unwrap();
    let row = store
        .pending_deliveries(1, 100)
        .await
        .unwrap()
        .pop()
        .unwrap();
    store
        .delivery_state(&row.id, "pending", "dispatching", None, 0)
        .await
        .unwrap();
    store
        .delivery_state(&row.id, "dispatching", "pending", None, 200)
        .await
        .unwrap();
    store
        .enqueue(1, 10, 11, "text", &Outbound::agent("Synthetic reply"))
        .await
        .unwrap();
    assert!(
        store.pending_deliveries(1, 100).await.unwrap().is_empty(),
        "a real rate limit must not be bypassed"
    );
    store
        .delivery_state(&row.id, "pending", "dispatching", None, 0)
        .await
        .unwrap();
    store.defer_icon_lookup(&row.id, &icon, 200).await.unwrap();
    let text = store.pending_deliveries(1, 100).await.unwrap();
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].source_key, "text");
    icon.appearance.as_mut().unwrap().emoji = "⚡️".into();
    store.enqueue_appearance(&binding, &icon).await.unwrap();
    assert_eq!(
        store.pending_deliveries(1, 100).await.unwrap()[0].source_key,
        "text",
        "coalescing a new state must preserve the read backoff bypass"
    );
}
