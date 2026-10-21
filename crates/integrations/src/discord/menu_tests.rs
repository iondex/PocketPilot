use super::*;
use crate::discord::{storage::Store, ui::Action};
use std::sync::Arc;
use tokio::sync::watch;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

fn parse(
    path: &std::path::Path,
    text: &str,
) -> std::result::Result<engine::reload::ParsedConfig, engine::config::ConfigError> {
    let mut config: engine::config::Config = toml::from_str(text).unwrap();
    config.prepare(path, text)?;
    for host in ["environment-alpha", "environment-beta"] {
        let id = format!("provider-{host}");
        config.providers.push(engine::model::Provider {
            id: id.clone(),
            host_id: host.into(),
            kind: "codex".into(),
            connection_mode: "existing_proxy".into(),
            compatibility_profile: "fixture".into(),
            control_verified: false,
            endpoint: None,
        });
        let count = if host.ends_with("alpha") { 23 } else { 1 };
        for index in 0..count {
            config.workspaces.push(engine::model::Workspace {
                id: format!("raw-project-{host}-{index}"),
                provider_id: id.clone(),
                name: format!("Project {index:02}"),
                short_name: format!("project-{host}-{index}"),
                cwd: format!("/projects/{host}/{index}"),
                additional_cwds: vec![],
                updated_at: None,
            });
        }
    }
    config.validate()?;
    Ok(engine::reload::ParsedConfig {
        config,
        restart_key: String::new(),
    })
}

fn input() -> Input {
    Input {
        uploads: vec![],
        locale: Some("zh-CN".into()),
        id: 100,
        bot: 1,
        guild: 2,
        channel: 7,
        user: 10,
        date: engine::now(),
        command: Command::Workspaces,
    }
}

async fn fixture() -> (tempfile::TempDir, MockServer, Service, Api) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pocketpilot.toml");
    std::fs::write(&file, "[[hosts]]\nid='environment-alpha'\nname='Alpha'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n[[hosts]]\nid='environment-beta'\nname='Beta'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']").unwrap();
    let config = Arc::new(engine::reload::ConfigFile::open(file, parse).unwrap());
    let core = engine::storage::Store::open(dir.path()).await.unwrap();
    let (_stop, stop) = watch::channel(false);
    let service = Service {
        config,
        store: Store::new(core.clone()),
        runtimes: Arc::new(engine::runtimes::RuntimeRegistry::new(core)),
        stop,
    };
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path_regex(".*/guilds/2$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"2","name":"Fixture","owner_id":"10","verification_level":0,"default_message_notifications":0,"explicit_content_filter":0,"roles":[],"emojis":[],"features":[],"mfa_level":0,"system_channel_flags":0,"premium_tier":0,"preferred_locale":"en-US","nsfw_level":0,"stickers":[],"premium_progress_bar_enabled":false}"#))
        .mount(&server).await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    (dir, server, service, api)
}

fn selector(screen: &Screen) -> (String, serde_json::Value) {
    let payload = serde_json::to_value(&screen.components).unwrap();
    let menu = &payload[0]["components"][0];
    (
        menu["custom_id"]
            .as_str()
            .unwrap()
            .strip_prefix("pp:")
            .unwrap()
            .into(),
        menu["options"].clone(),
    )
}

fn button_token(screen: &Screen, label: &str) -> String {
    let payload = serde_json::to_value(&screen.components).unwrap();
    payload
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["components"].as_array().unwrap())
        .find(|button| button["label"] == label)
        .unwrap()["custom_id"]
        .as_str()
        .unwrap()
        .strip_prefix("pp:")
        .unwrap()
        .into()
}

#[tokio::test]
async fn native_menus_page_in_place_without_displaying_ids_or_long_project_lists() {
    let (_dir, server, service, api) = fixture().await;
    let first = service
        .menu_screen(&api, input(), None, None)
        .await
        .unwrap();
    assert!(first.text.contains("🌐") && !first.text.contains("environment-alpha"));
    let (token, options) = selector(&first);
    assert_eq!(options.as_array().unwrap().len(), 2);
    assert_eq!(options[0]["label"], "Alpha");
    let projects = service
        .menu_screen(&api, input(), Some(token.clone()), Some("0".into()))
        .await
        .unwrap();
    assert!(projects.text.contains("1–20") && projects.text.contains("23"));
    assert!(!projects.text.contains("raw-project") && !projects.text.contains("Project 00"));
    assert!(projects.text.encode_utf16().count() < 300);
    let (_, options) = selector(&projects);
    assert_eq!(options.as_array().unwrap().len(), 20);
    assert_eq!(options[0]["label"], "Project 00");
    assert_eq!(options[0]["value"], "0");
    let next = service
        .menu_screen(&api, input(), Some(button_token(&projects, "下一页")), None)
        .await
        .unwrap();
    let (_, options) = selector(&next);
    assert_eq!(options.as_array().unwrap().len(), 3);
    assert_eq!(options[0]["label"], "Project 20");
    let previous = service
        .menu_screen(&api, input(), Some(button_token(&next, "上一页")), None)
        .await
        .unwrap();
    assert_eq!(selector(&previous).1.as_array().unwrap().len(), 20);
    let mut other = input();
    other.user = 99;
    assert!(
        service
            .menu_screen(&api, other, Some(token.clone()), Some("0".into()))
            .await
            .is_err()
    );
    let mut other = input();
    other.channel = 8;
    assert!(
        service
            .menu_screen(&api, other, Some(token.clone()), Some("0".into()))
            .await
            .is_err()
    );
    assert!(
        service
            .menu_screen(&api, input(), Some(token.clone()), Some("2".into()))
            .await
            .is_err()
    );
    let file = &service.config.path;
    let text = std::fs::read_to_string(file).unwrap();
    std::fs::write(file, format!("{text}\n# configuration changed\n")).unwrap();
    service.config.reload().unwrap();
    assert!(
        service
            .menu_screen(&api, input(), Some(token), Some("0".into()))
            .await
            .is_err()
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn open_choices_are_atomic_once_and_scoped_to_the_saved_menu() {
    let (_dir, _server, service, _api) = fixture().await;
    let snapshot = service.config.snapshot();
    let choice = View::Open {
        project: "project".into(),
        thread: "private-thread".into(),
        cursors: vec![None],
    };
    let token = service
        .store
        .ui_action(&input(), &snapshot.hash, Action::Select(vec![choice]))
        .await
        .unwrap();
    assert!(
        service
            .store
            .ui_resolve(&input(), &snapshot.hash, &token, Some("not-an-index"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .store
            .ui_resolve(&input(), &snapshot.hash, &token, None)
            .await
            .unwrap()
            .is_none()
    );
    let request = input();
    let results = tokio::join!(
        service
            .store
            .ui_resolve(&request, &snapshot.hash, &token, Some("0")),
        service
            .store
            .ui_resolve(&request, &snapshot.hash, &token, Some("0"))
    );
    assert_eq!(
        usize::from(results.0.unwrap().is_some()) + usize::from(results.1.unwrap().is_some()),
        1
    );
    let token = service
        .store
        .ui_action(
            &input(),
            &snapshot.hash,
            Action::Navigate(View::Workspaces { offset: 0 }),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE discord_ui_actions SET expires=0 WHERE nonce=?")
        .bind(&token)
        .execute(service.store.pool())
        .await
        .unwrap();
    assert!(
        service
            .store
            .ui_resolve(&input(), &snapshot.hash, &token, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn labels_and_paths_fit_native_mobile_controls_with_emoji_and_literal_titles() {
    let name = format!("**Title** {}", "👩‍💻".repeat(100));
    let option = serde_json::to_value(
        ui::option(
            &name,
            &directory(&format!("/{}end", "long/path/".repeat(100))),
            0,
        )
        .build(),
    )
    .unwrap();
    assert!(session::markdown::fits(
        option["label"].as_str().unwrap(),
        100
    ));
    assert!(session::markdown::fits(
        option["description"].as_str().unwrap(),
        100
    ));
    assert!(option["description"].as_str().unwrap().ends_with("end"));
    assert!(ui::quoted("**name** @everyone").starts_with("\\*\\*"));
    assert!(
        ui::status(
            &session::RuntimeStatus::Active {
                waiting_on_approval: true,
                waiting_on_user_input: false
            },
            "en"
        )
        .contains("🟠")
    );
}

#[tokio::test]
async fn project_controls_follow_the_shared_activity_order() {
    let (_dir, _server, service, api) = fixture().await;
    let mut config = service.config.snapshot().config.clone();
    let recent = config
        .workspaces
        .iter_mut()
        .find(|p| p.name == "Project 22")
        .unwrap();
    recent.updated_at = Some(500);
    service.config.publish_catalog(config).unwrap();
    let mut request = input();
    request.command = Command::Projects {
        workspace: "environment-alpha".into(),
    };
    let screen = service
        .menu_screen(&api, request, None, None)
        .await
        .unwrap();
    assert_eq!(selector(&screen).1[0]["label"], "Project 22");
}

#[path = "binding_menu_tests.rs"]
mod bindings;
