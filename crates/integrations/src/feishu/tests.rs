use super::*;
use crate::rendering::{RenderRequest, Renderer};

pub(super) fn settings() -> config::Config {
    config::Config {
        app_id: "cli_example".into(),
        app_secret: "synthetic-secret".into(),
        tenant_key: "tenant_example".into(),
        owner_open_id: "ou_owner".into(),
        groups: vec![config::Group {
            chat_id: "oc_group".into(),
            name: "Test group".into(),
        }],
    }
}

#[test]
fn configuration_is_application_scoped_and_redacts_secrets() {
    let mut config = settings();
    assert!(config.valid());
    assert!(!format!("{config:?}").contains("synthetic-secret"));
    assert!(!config.permits("other", "ou_owner"));
    config.groups.push(config.groups[0].clone());
    assert!(!config.valid());
}

#[test]
fn cards_preserve_tables_code_and_unicode_without_native_tag_injection() {
    let source = "# Heading\n\n| Name | Value |\n| --- | --- |\n| 示例 | **value** |\n\n```rust\nlet x = 1 < 2;\n```\n\n<at id=all>everyone</at>\n\n[bad](javascript:alert)";
    let rendered = render::CardRenderer
        .render(RenderRequest {
            source,
            format: session::markdown::Format::Markdown,
            heading: Some("🤖 **Agent**"),
        })
        .unwrap();
    assert!(!rendered.parts.is_empty());
    let wire = serde_json::to_string(&rendered.parts).unwrap();
    assert!(wire.contains("| Name | Value |"));
    assert!(wire.contains("let x = 1 < 2;"));
    assert!(wire.contains("示例"));
    assert!(wire.contains("🤖 Agent"));
    assert!(!wire.contains("**Agent**"));
    assert!(!wire.contains("<at"));
    assert!(!wire.contains("javascript:"));
    assert!(rendered.parts.iter().all(render::Card::bounded));
}

#[test]
fn rich_text_splits_within_final_encoded_payload_limits() {
    let text = "你好 👩🏽‍💻 **rich** `code`\n\n".repeat(1000);
    let rendered = render::CardRenderer
        .render(RenderRequest {
            source: &text,
            format: session::markdown::Format::Markdown,
            heading: Some("Agent"),
        })
        .unwrap();
    assert!(rendered.parts.len() > 1);
    assert!(rendered.parts.iter().all(render::Card::bounded));
}

#[tokio::test]
async fn callbacks_are_one_shot_and_require_the_exact_chat_message_and_revision() {
    let dir = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let config = settings();
    let store = storage::Store::new(core.clone(), config::Region::Feishu, &config);
    let input = model::Input {
        id: "event".into(),
        chat: "oc_private".into(),
        message: "om_menu".into(),
        root: None,
        thread: None,
        date: engine::now(),
        command: model::Command::Menu(model::Action::Home { offset: 0 }),
        files: vec![],
    };
    let nonce = store
        .action(&input.chat, model::Action::Home { offset: 0 }, "hash")
        .await
        .unwrap();
    sqlx::query("UPDATE feishu_actions SET message='om_menu' WHERE nonce=?")
        .bind(&nonce)
        .execute(core.pool())
        .await
        .unwrap();
    assert!(
        !store
            .callback(&nonce, "om_other", &input, "hash", None)
            .await
            .unwrap()
    );
    assert!(
        !store
            .callback(&nonce, "om_menu", &input, "different-hash", None)
            .await
            .unwrap()
    );
    assert!(
        store
            .callback(&nonce, "om_menu", &input, "hash", None)
            .await
            .unwrap()
    );
    assert!(
        !store
            .callback(&nonce, "om_menu", &input, "hash", None)
            .await
            .unwrap()
    );
    let row = store.next().await.unwrap().unwrap();
    assert_eq!(row.id, "event");
    assert!(store.next().await.unwrap().is_none());
    store.recover().await.unwrap();
    let state: String =
        sqlx::query_scalar("SELECT state FROM feishu_inbox WHERE namespace=? AND id='event'")
            .bind(&store.namespace)
            .fetch_one(core.pool())
            .await
            .unwrap();
    assert_eq!(state, "unknown");
}

#[tokio::test]
async fn event_redelivery_uses_message_identity_and_other_namespaces_cannot_claim_actions() {
    let dir = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let config = settings();
    let store = storage::Store::new(core.clone(), config::Region::Feishu, &config);
    let input = model::Input {
        id: "message:om_one".into(),
        chat: "oc_private".into(),
        message: "om_one".into(),
        root: None,
        thread: None,
        date: engine::now(),
        command: model::Command::Menu(model::Action::Home { offset: 0 }),
        files: vec![],
    };
    assert!(store.enqueue(&input, "hash").await.unwrap());
    assert!(!store.enqueue(&input, "hash").await.unwrap());
    let nonce = store
        .action(&input.chat, model::Action::Home { offset: 0 }, "hash")
        .await
        .unwrap();
    sqlx::query("UPDATE feishu_actions SET message='om_one' WHERE nonce=?")
        .bind(&nonce)
        .execute(core.pool())
        .await
        .unwrap();
    let mut other = config;
    other.tenant_key = "other_tenant".into();
    let other = storage::Store::new(core, config::Region::Feishu, &other);
    assert!(
        !other
            .callback(&nonce, "om_one", &input, "hash", None)
            .await
            .unwrap()
    );
}

fn binding(id: &str) -> model::Binding {
    model::Binding {
        id: id.into(),
        chat: "oc_group".into(),
        root: "om_root".into(),
        topic: "omt_topic".into(),
        project: "project".into(),
        instance: "runtime".into(),
        session: "session".into(),
        cwd: "/projects/example".into(),
        endpoint: "endpoint".into(),
        revision: 1,
        active: true,
        cursor: "{}".into(),
        ready: false,
        status: String::new(),
        state: "ready".into(),
    }
}

#[tokio::test]
async fn uncertain_provisioning_is_not_recreated_until_explicit_detach() {
    let directory = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(directory.path())
        .await
        .unwrap();
    let config = settings();
    let store = storage::Store::new(core.clone(), config::Region::Feishu, &config);
    let original = binding("original");
    store.reserve(&original).await.unwrap();
    store
        .provision(&original.id, Some(&original.root), None, false)
        .await
        .unwrap();
    store.recover().await.unwrap();
    let recovered = store.binding(&original.id).await.unwrap();
    assert_eq!(recovered.state, "unknown");
    assert_eq!(recovered.root, "om_root");
    let next = binding("next");
    assert!(store.reserve(&next).await.is_err());
    crate::bindings::Manager::new(
        &store,
        super::binding_store::scope(config::Region::Feishu, &config),
    )
    .change(
        &crate::bindings::Target::Feishu { id: original.id },
        crate::bindings::Version {
            binding: 1,
            observation: 0,
        },
        crate::bindings::Change::Detach,
    )
    .await
    .unwrap();
    store.reserve(&next).await.unwrap();
    assert_eq!(store.binding(&next.id).await.unwrap().state, "provisioning");
    let operations: i64 = sqlx::query_scalar("SELECT count(*) FROM operations")
        .fetch_one(core.pool())
        .await
        .unwrap();
    assert_eq!(operations, 0);
}

#[tokio::test]
async fn unknown_outbound_receipts_remain_terminal_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let core = engine::storage::Store::open(directory.path())
        .await
        .unwrap();
    let store = storage::Store::new(core.clone(), config::Region::Feishu, &settings());
    store
        .output(
            &model::Destination {
                chat: "oc_group".into(),
                root: Some("om_root".into()),
            },
            &[render::Card::new("fixture")],
            "hash",
            None,
            None,
            &[],
        )
        .await
        .unwrap();
    let claimed = store.outgoing().await.unwrap().unwrap();
    assert!(store.outgoing().await.unwrap().is_none());
    store.recover().await.unwrap();
    assert!(store.outgoing().await.unwrap().is_none());
    let state: String = sqlx::query_scalar("SELECT state FROM feishu_outbox WHERE id=?")
        .bind(&claimed.id)
        .fetch_one(core.pool())
        .await
        .unwrap();
    assert_eq!(state, "unknown");
}

#[test]
fn multiple_tables_use_separate_native_components_within_one_card() {
    let source = "| A | B |\n|---|---|\n|1|2|\n\n".repeat(5);
    let rendered = render::CardRenderer
        .render(RenderRequest {
            source: &source,
            format: session::markdown::Format::Markdown,
            heading: None,
        })
        .unwrap();
    assert_eq!(rendered.parts.len(), 1);
    let encoded = serde_json::to_string(&rendered.parts[0]).unwrap();
    assert_eq!(encoded.matches("\"tag\":\"markdown\"").count(), 5);
}

#[test]
fn ordinary_long_replies_do_not_split_when_the_native_payload_still_fits() {
    let source = "内容".repeat(2400);
    let rendered = render::CardRenderer
        .render(RenderRequest {
            source: &source,
            format: session::markdown::Format::Markdown,
            heading: Some("🤖 **Agent**"),
        })
        .unwrap();
    assert_eq!(rendered.parts.len(), 1);
    assert!(rendered.parts[0].bounded());
}

#[path = "region_tests.rs"]
mod regions;
