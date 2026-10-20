use super::*;

#[tokio::test]
async fn provisioning_and_unknown_sends_are_not_replayed_and_containers_cannot_change_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();
    assert!(
        store
            .discord_reserve(1, 2, "ws", None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.discord_reserve(1, 2, "ws", None).await.is_err());
    store.discord_category_receipt(1, 2, "ws", 3).await.unwrap();
    assert_eq!(
        store.discord_reserve(1, 2, "ws", None).await.unwrap(),
        Some(3)
    );
    let mut b = Binding {
        bot: "1".into(),
        guild: "2".into(),
        channel: "4".into(),
        owner: "5".into(),
        workspace: "ws".into(),
        instance: "instance".into(),
        thread: "thread".into(),
        cwd: "/ws".into(),
        endpoint: "endpoint".into(),
        active: true,
        revision: 1,
        observed_items: "{}".into(),
        history_ready: false,
        card_message: None,
        card_text: None,
    };
    store.discord_bind(&b).await.unwrap();
    store
        .discord_publish(
            &b,
            "history",
            &crate::discord::render::Conversation {
                text: "visible history".into(),
                parts: vec![
                    crate::discord::native::Message::text("first".into()),
                    crate::discord::native::Message::text("second".into()),
                ],
            },
            "{\"item\":\"hash\"}",
        )
        .await
        .unwrap();
    let out = store.discord_pending(1, 2, 0).await.unwrap();
    assert_eq!(out.len(), 1);
    store
        .discord_delivery_state(&out[0].id, "pending", "dispatching")
        .await
        .unwrap();
    let parts = ["first", "second"]
        .into_iter()
        .map(|text| {
            crate::discord::native::Part::V2(crate::discord::native::Message::text(text.into()))
        })
        .collect::<Vec<_>>();
    // Role headers and chunk layout are frozen atomically with publication.
    assert_eq!(
        out[0].rendered_parts.as_deref(),
        Some(serde_json::to_string(&parts).unwrap().as_str()),
    );
    store.acknowledge_part(&out[0], true).await.unwrap();
    store.discord_recover().await.unwrap();
    let remaining = store
        .discord_pending(1, 2, engine::now() + 10)
        .await
        .unwrap();
    assert_eq!(remaining[0].next_part, 1);
    assert_eq!(
        remaining[0].rendered_parts.as_deref(),
        Some(serde_json::to_string(&parts).unwrap().as_str())
    );
    store
        .discord_delivery_state(&remaining[0].id, "pending", "dispatching")
        .await
        .unwrap();
    store.discord_recover().await.unwrap();
    assert!(store.discord_pending(1, 2, 0).await.unwrap().is_empty());
    b.thread = "other".into();
    assert!(matches!(
        store.discord_bind(&b).await,
        Err(Error::ContainerConflict)
    ));
}
