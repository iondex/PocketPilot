use super::*;

fn binding() -> Binding {
    Binding {
        id: "binding".into(),
        chat: "oc_group".into(),
        root: "om_root".into(),
        topic: "omt_thread".into(),
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

fn item(turn: &str, role: session::Role, text: &str) -> session::TextItem {
    session::TextItem {
        turn_id: turn.into(),
        item_id: format!("{role:?}"),
        role,
        phase: (role == session::Role::Agent).then(|| "final_answer".into()),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    }
}

#[test]
fn initial_binding_previews_only_the_latest_interaction_then_sends_deltas() {
    let history = session::History {
        items: vec![
            item("old", session::Role::User, "old input"),
            item("old", session::Role::Agent, "old answer"),
            item("new", session::Role::User, "latest input"),
            item("new", session::Role::Agent, "latest answer"),
        ],
        all_pages_read: true,
    };
    let mut binding = binding();
    let (cursor, sections) = capture(&binding, &history, "namespace", "en").unwrap();
    let text = sections
        .into_iter()
        .map(|s| s.body)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("latest input") && text.contains("latest answer"));
    assert!(!text.contains("old input") && !text.contains("old answer"));
    binding.ready = true;
    binding.cursor = cursor;
    assert!(
        capture(&binding, &history, "namespace", "en")
            .unwrap()
            .1
            .is_empty()
    );
    let mut changed = history;
    changed.items.last_mut().unwrap().text = "changed answer".into();
    let (_, sections) = capture(&binding, &changed, "namespace", "en").unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].body, "changed answer");
}
