use super::*;

#[test]
fn guild_owner_is_required_and_stop_has_no_route() {
    assert!(authorized(1, Some(1), 2, 2));
    assert!(!authorized(1, None, 2, 2));
    assert!(!authorized(1, Some(3), 2, 2));
    assert!(!authorized(1, Some(1), 2, 3));
    assert!(parse("pp-stop", &BTreeMap::new(), true).is_err());
    assert!(parse("pp-ask", &BTreeMap::new(), true).is_err());
}

#[test]
fn pp_ask_requires_nonempty_text() {
    assert!(matches!(
        parse("pp-ask", &BTreeMap::new(), false),
        Err(Error::MissingInput)
    ));
    assert!(
        parse(
            "pp-ask",
            &BTreeMap::from([("text".into(), "  ".into())]),
            false
        )
        .is_err()
    );
    assert!(
        matches!(parse("pp-ask", &BTreeMap::from([("text".into(), "continue".into())]), false).unwrap(), Command::Ask { text } if text == "continue")
    );
}

#[test]
fn channel_names_are_short_and_cannot_ping_members() {
    let name = channel_name("Hello @everyone / Test");
    assert!(!name.contains('@'));
    assert_eq!(name, "hello-everyone-test");
    assert!(channel_name(&"好".repeat(500)).chars().count() < 100);
}

#[test]
fn navigation_commands_fill_missing_selections_through_the_menu() {
    assert!(matches!(
        parse("pp", &BTreeMap::new(), false).unwrap(),
        Command::Workspaces
    ));
    assert!(matches!(
        parse("pp-open", &BTreeMap::new(), false).unwrap(),
        Command::Workspaces
    ));
    let mut fields = BTreeMap::from([("workspace".into(), "host-a".into())]);
    assert!(
        matches!(parse("pp-projects", &fields, false).unwrap(), Command::Projects { workspace } if workspace == "host-a")
    );
    assert!(matches!(
        parse("pp-sessions", &fields, false).unwrap(),
        Command::Projects { .. }
    ));
    fields.insert("project".into(), "project-a".into());
    assert!(
        matches!(parse("pp-sessions", &fields, false).unwrap(), Command::Sessions { workspace, project, .. } if workspace == "host-a" && project == "project-a")
    );
    fields.remove("workspace");
    assert!(matches!(
        parse("pp-sessions", &fields, false).unwrap(),
        Command::Workspaces
    ));
}
