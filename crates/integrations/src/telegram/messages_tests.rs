use super::*;

#[test]
fn entry_preview_is_bounded_for_unicode_and_escaped_markdown_at_the_configured_limit() {
    let history = History {
        all_pages_read: true,
        items: vec![TextItem {
            turn_id: "t".into(),
            item_id: "a".into(),
            role: Role::Agent,
            phase: Some("final_answer".into()),
            text: "👩🏽‍💻 **Synthetic result** <value> ".repeat(1000),
            complete: true,
            client_id: None,
            attachments: vec![],
        }],
    };
    for limit in [256, 1200, 2800] {
        let preview = initial(&history, true, &HashSet::new(), "en", limit);
        assert!(preview.encode_utf16().count() < 3800);
        assert!(
            preview.contains('…')
                && preview.contains("/history")
                && !preview.contains("/history export")
        );
    }
}

#[test]
fn entry_preview_uses_the_latest_turn_without_pairing_an_old_answer() {
    let item = |turn: &str, role, phase: Option<&str>, text: &str| TextItem {
        turn_id: turn.into(),
        item_id: text.into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let history = History {
        all_pages_read: true,
        items: vec![
            item("old", Role::User, None, "Earlier request"),
            item("old", Role::Agent, Some("final_answer"), "Earlier final"),
            item("new", Role::User, None, "Current request"),
            item("new", Role::Agent, Some("commentary"), "Current progress"),
        ],
    };
    let preview = initial(&history, true, &HashSet::new(), "en", 1200);
    assert!(
        preview.contains("Current request")
            && preview.contains("🤖 **Agent** · 💭\n\nCurrent progress")
    );
    assert!(!preview.contains("Earlier"));
}
