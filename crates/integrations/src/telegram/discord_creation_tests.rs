use super::*;
use discord::{Backend, Command, Input};

fn request(id: u64, command: Command) -> Input {
    Input {
        uploads: vec![],
        locale: Some("en".into()),
        id,
        bot: 1,
        guild: 2,
        channel: 7,
        user: 10,
        date: now(),
        command,
    }
}

#[derive(Deserialize)]
struct Row {
    components: Vec<Control>,
}

#[derive(Deserialize)]
struct Control {
    custom_id: Option<String>,
    label: Option<String>,
}

fn controls(screen: &discord::ui::Screen) -> Vec<Control> {
    let rows: Vec<Row> =
        serde_json::from_str(&serde_json::to_string(&screen.components).unwrap()).unwrap();
    rows.into_iter().flat_map(|row| row.components).collect()
}

#[tokio::test]
async fn discord_creation_form_requires_explicit_confirmation_and_never_replays_a_creation() {
    let (fixture, _http, api, service) = discord_fixture().await;
    let list = service
        .menu(
            &api,
            request(
                8000,
                Command::Sessions {
                    workspace: "host".into(),
                    project: "workspace".into(),
                    cursor: None,
                },
            ),
            None,
            None,
        )
        .await
        .unwrap();
    let buttons = controls(&list);
    assert!(
        !buttons
            .iter()
            .any(|button| matches!(button.label.as_deref(), Some("Next page" | "Previous page")))
    );
    let form_button = buttons
        .iter()
        .find_map(|control| {
            control
                .custom_id
                .as_deref()
                .and_then(|id| id.strip_prefix("pp-form:"))
        })
        .unwrap();
    let form = service
        .new_session_form(&api, request(8001, Command::Workspaces), form_button)
        .await
        .unwrap();
    let confirm = service
        .menu(
            &api,
            request(8002, Command::Workspaces),
            Some(form.token),
            Some("Only reply with the fixture response".into()),
        )
        .await
        .unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|method| matches!(*method, "thread/start" | "turn/start"))
    );
    let token = controls(&confirm)
        .into_iter()
        .find(|control| control.label.as_deref() == Some("Create and send task"))
        .unwrap()
        .custom_id
        .unwrap();
    let token = token.strip_prefix("pp:").unwrap().to_owned();
    let opened = service
        .menu(
            &api,
            request(8003, Command::Workspaces),
            Some(token.clone()),
            None,
        )
        .await
        .unwrap();
    assert!(opened.text.contains("<#11>"));
    assert!(
        service
            .menu(&api, request(8004, Command::Workspaces), Some(token), None)
            .await
            .is_err()
    );
    let state = fixture.state.lock().await;
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|method| **method == "thread/start")
            .count(),
        1
    );
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|method| **method == "turn/start")
            .count(),
        1
    );
}
