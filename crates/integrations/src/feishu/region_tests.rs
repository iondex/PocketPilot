//! Identical external IDs must not merge platform authorization or durable state.
use super::*;
use crate::bindings::{Repository, Scope, Target};
use crate::feishu::{binding_store, config::Region};

#[test]
fn official_endpoints_and_existing_feishu_namespaces_are_preserved() {
    let config = settings();
    assert_eq!(
        config.namespace(Region::Feishu),
        "feishu:cli_example:tenant_example:ou_owner"
    );
    assert_eq!(
        config.namespace(Region::Lark),
        "lark:cli_example:tenant_example:ou_owner"
    );
    for (region, base, platform) in [
        (
            Region::Feishu,
            "https://open.feishu.cn",
            crate::Platform::Feishu,
        ),
        (
            Region::Lark,
            "https://open.larksuite.com",
            crate::Platform::Lark,
        ),
    ] {
        let api = api::Api::new(region, &config).unwrap();
        assert_eq!(api.config.base_url(), base);
        assert!(!api.config.allow_custom_base_url());
        let integration = Feishu::for_region(region, config.clone());
        assert_eq!(crate::Integration::platform(&integration), platform);
        assert!(matches!(
            crate::Integration::capabilities(&integration).format,
            crate::rendering::MessageFormat::FeishuCard
        ));
    }
    // Existing persisted binding controls keep their original enum representation.
    let scope = binding_store::scope(Region::Feishu, &config);
    let serialized = serde_json::to_string(&scope).unwrap();
    assert_eq!(
        serialized,
        r#"{"Feishu":{"app":"cli_example","tenant":"tenant_example","owner":"ou_owner"}}"#
    );
}

#[tokio::test]
async fn shared_external_ids_keep_events_callbacks_operations_and_recovery_separate() {
    let directory = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(directory.path())
        .await
        .unwrap();
    let config = settings();
    let feishu = storage::Store::new(core.clone(), Region::Feishu, &config);
    let lark = storage::Store::new(core.clone(), Region::Lark, &config);
    let mut input = model::Input {
        id: "message:om_same".into(),
        chat: "oc_private".into(),
        message: "om_same".into(),
        root: None,
        thread: None,
        date: engine::now(),
        command: model::Command::Menu(model::Action::Home { offset: 0 }),
        files: vec![],
    };
    assert!(feishu.enqueue(&input, "hash").await.unwrap());
    assert!(lark.enqueue(&input, "hash").await.unwrap());
    assert!(!lark.enqueue(&input, "hash").await.unwrap());
    let first = core
        .reserve_operation(
            &format!("{}:{}:start", feishu.namespace, input.id),
            "runtime",
            "session",
        )
        .await
        .unwrap();
    let second = core
        .reserve_operation(
            &format!("{}:{}:start", lark.namespace, input.id),
            "runtime",
            "session",
        )
        .await
        .unwrap();
    assert_ne!(first, second);

    let nonce = feishu
        .action(&input.chat, model::Action::Home { offset: 0 }, "hash")
        .await
        .unwrap();
    sqlx::query("UPDATE feishu_actions SET message='om_same' WHERE nonce=?")
        .bind(&nonce)
        .execute(core.pool())
        .await
        .unwrap();
    input.id = "card:shared-event".into();
    assert!(
        !lark
            .callback(&nonce, "om_same", &input, "hash", None)
            .await
            .unwrap()
    );
    assert!(
        feishu
            .callback(&nonce, "om_same", &input, "hash", None)
            .await
            .unwrap()
    );

    let surviving_nonce = feishu
        .action(&input.chat, model::Action::Home { offset: 0 }, "hash")
        .await
        .unwrap();
    sqlx::query("UPDATE feishu_actions SET message='om_same' WHERE nonce=?")
        .bind(&surviving_nonce)
        .execute(core.pool())
        .await
        .unwrap();
    assert!(lark.next().await.unwrap().is_some());
    lark.recover().await.unwrap();
    let lark_state: String = sqlx::query_scalar(
        "SELECT state FROM feishu_inbox WHERE namespace=? AND id='message:om_same'",
    )
    .bind(&lark.namespace)
    .fetch_one(core.pool())
    .await
    .unwrap();
    assert_eq!(lark_state, "unknown");
    // Recovering Lark must neither stale Feishu input nor consume its actions.
    assert!(feishu.next().await.unwrap().is_some());
    input.id = "card:after-lark-recovery".into();
    assert!(
        feishu
            .callback(&surviving_nonce, "om_same", &input, "hash", None)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn binding_inventory_and_claims_reject_the_other_platform() {
    let directory = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(directory.path())
        .await
        .unwrap();
    let config = settings();
    let feishu = storage::Store::new(core.clone(), Region::Feishu, &config);
    let lark = storage::Store::new(core.clone(), Region::Lark, &config);
    for (store, id) in [(&feishu, "binding-feishu"), (&lark, "binding-lark")] {
        let b = binding(id);
        store.reserve(&b).await.unwrap();
        store
            .provision(id, Some(&b.root), Some(&b.topic), false)
            .await
            .unwrap();
    }
    let lark_scope = binding_store::scope(Region::Lark, &config);
    let feishu_scope = binding_store::scope(Region::Feishu, &config);
    assert!(matches!(lark_scope, Scope::Lark { .. }));
    let records = lark.records(&lark_scope).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].target,
        Target::Lark {
            id: "binding-lark".into()
        }
    );
    assert!(lark.records(&feishu_scope).await.is_err());
    assert!(feishu.records(&lark_scope).await.is_err());
    assert!(lark.binding("binding-feishu").await.is_err());
    let mut tx = core.pool().begin().await.unwrap();
    assert!(
        lark.claim(
            &mut tx,
            &lark_scope,
            &Target::Feishu {
                id: "binding-lark".into()
            },
            1,
            crate::bindings::Change::Detach
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    assert!(lark.binding("binding-lark").await.unwrap().active);
    crate::bindings::Manager::new(&lark, lark_scope)
        .change(
            &Target::Lark {
                id: "binding-lark".into(),
            },
            crate::bindings::Version {
                binding: 1,
                observation: 0,
            },
            crate::bindings::Change::Detach,
        )
        .await
        .unwrap();
    assert!(!lark.binding("binding-lark").await.unwrap().active);
    assert!(feishu.binding("binding-feishu").await.unwrap().active);
}
