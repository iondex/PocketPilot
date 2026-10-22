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
    let (cursor, sections) =
        capture(&binding, &history, "namespace", Region::Feishu, "en").unwrap();
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
        capture(&binding, &history, "namespace", Region::Feishu, "en")
            .unwrap()
            .1
            .is_empty()
    );
    let mut changed = history;
    changed.items.last_mut().unwrap().text = "changed answer".into();
    let (_, sections) = capture(&binding, &changed, "namespace", Region::Feishu, "en").unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].body, "changed answer");
}

#[test]
fn observation_suppresses_only_the_current_application_input_echo() {
    for (region, other) in [
        (Region::Feishu, Region::Lark),
        (Region::Lark, Region::Feishu),
    ] {
        let settings = super::super::super::tests::settings();
        let namespace = settings.namespace(region);
        let mut binding = binding();
        binding.ready = true;
        let mut own = item("own", session::Role::User, "own input");
        own.client_id = Some(format!(
            "pp:{}:{}:input",
            region.as_str(),
            engine::digest(&namespace)
        ));
        let mut cross_platform = item(
            "other-platform",
            session::Role::User,
            "other platform input",
        );
        cross_platform.client_id = Some(format!(
            "pp:{}:{}:input",
            other.as_str(),
            engine::digest(&settings.namespace(other))
        ));
        let mut cross_application =
            item("other-app", session::Role::User, "other application input");
        cross_application.client_id = Some(format!(
            "pp:{}:{}:input",
            region.as_str(),
            engine::digest("other-application")
        ));
        let mut answer = item("own", session::Role::Agent, "agent answer");
        answer.client_id = own.client_id.clone();
        let history = session::History {
            items: vec![own, cross_platform, cross_application, answer],
            all_pages_read: true,
        };
        let (_, sections) = capture(&binding, &history, &namespace, region, "en").unwrap();
        let bodies: Vec<_> = sections
            .iter()
            .map(|section| section.body.as_str())
            .collect();
        assert_eq!(
            bodies,
            [
                "other platform input",
                "other application input",
                "agent answer"
            ]
        );
    }
}
