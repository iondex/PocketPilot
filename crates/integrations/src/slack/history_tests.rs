use super::*;

fn item(turn: &str, id: &str, role: Role, phase: Option<&str>, text: &str) -> session::TextItem {
    session::TextItem {
        turn_id: turn.into(),
        item_id: id.into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    }
}

fn history() -> History {
    History {
        items: vec![
            item("old", "user", Role::User, None, "Earlier question"),
            item(
                "old",
                "answer",
                Role::Agent,
                Some("final_answer"),
                "Earlier answer",
            ),
            item("last", "user", Role::User, None, "Last question"),
            item(
                "last",
                "working",
                Role::Agent,
                Some("commentary"),
                "Working notes",
            ),
            item(
                "last",
                "answer",
                Role::Agent,
                Some("final_answer"),
                "Last answer",
            ),
        ],
        all_pages_read: true,
    }
}

fn binding() -> Binding {
    Binding {
        id: "binding".into(),
        app: "A".into(),
        team: "T".into(),
        bot: "B".into(),
        channel: "D".into(),
        root: "1.000001".into(),
        owner: "U".into(),
        workspace: "project".into(),
        instance: "runtime".into(),
        thread: "thread".into(),
        cwd: "/project".into(),
        endpoint: "endpoint".into(),
        active: true,
        revision: 1,
        observed: "{}".into(),
        card: String::new(),
        history_ready: false,
    }
}

#[tokio::test]
async fn entry_displays_only_last_interaction_and_first_poll_does_not_replay_old_history() {
    let history = history();
    let mut binding = binding();
    let first = capture(&binding, &history, "A:T:B", "en").unwrap();
    assert_eq!(first.content.sections.len(), 2);
    assert!(
        first.content.text.contains("Last question") && first.content.text.contains("Last answer")
    );
    assert!(
        !first.content.text.contains("Earlier") && !first.content.text.contains("Working notes")
    );
    assert_eq!(
        serde_json::from_str::<BTreeMap<String, String>>(&first.cursor)
            .unwrap()
            .len(),
        5
    );
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(
        engine::storage::Store::open(directory.path())
            .await
            .unwrap(),
        "A",
        "T",
        "B",
    );
    store.bind(&binding).await.unwrap();
    store
        .publish(&binding, &first.content, &first.cursor, "scope")
        .await
        .unwrap();
    binding = store.binding(&binding.id).await.unwrap().unwrap();
    let next = capture(&binding, &history, "A:T:B", "en").unwrap();
    assert!(next.content.sections.is_empty() && next.content.text.is_empty());
    store
        .publish(&binding, &next.content, &next.cursor, "scope")
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM slack_outbox")
        .fetch_one(store.core.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn live_poll_shows_new_items_and_preserves_the_old_baseline() {
    let mut history = history();
    let mut binding = binding();
    binding.observed = capture(&binding, &history, "A:T:B", "en").unwrap().cursor;
    binding.history_ready = true;
    let mut own = item("new", "user", Role::User, None, "Already visible in Slack");
    own.client_id = Some("pp:slack:A:T:B:fixture".into());
    history.items.push(own);
    history.items.push(item(
        "new",
        "answer",
        Role::Agent,
        Some("final_answer"),
        "New answer",
    ));
    let delta = capture(&binding, &history, "A:T:B", "en").unwrap();
    assert_eq!(delta.content.sections.len(), 1);
    assert!(delta.content.text.contains("New answer"));
    assert!(
        !delta.content.text.contains("Earlier")
            && !delta.content.text.contains("Last question")
            && !delta.content.text.contains("Already visible")
    );
    assert_eq!(
        serde_json::from_str::<BTreeMap<String, String>>(&delta.cursor)
            .unwrap()
            .len(),
        7
    );
}

#[test]
fn incomplete_history_cannot_be_published_as_an_entry_or_live_baseline() {
    let mut history = history();
    history.all_pages_read = false;
    let mut binding = binding();
    assert!(capture(&binding, &history, "A:T:B", "en").is_err());
    binding.history_ready = true;
    assert!(capture(&binding, &history, "A:T:B", "en").is_err());
}
