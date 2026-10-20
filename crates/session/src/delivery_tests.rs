use super::*;

#[test]
fn recoverable_errors_are_hidden_from_history_but_final_results_remain_visible() {
    let mut history = History {
        all_pages_read: true,
        items: vec![
            item("t", Role::User, None, "question"),
            item("t", Role::Agent, Some("commentary"), "working"),
            item("t", Role::Agent, Some("error_retry"), "Reconnecting... 1/5"),
            item("t", Role::Agent, Some("final_answer"), "done"),
        ],
    };
    for text in [agent_history(&history), latest_interaction(&history)] {
        assert!(!text.contains("Reconnecting"));
        assert!(text.contains("done"));
    }
    history
        .items
        .push(item("t", Role::Agent, Some("error"), "terminal failure"));
    assert_eq!(last_interaction(&history).unwrap().errors.len(), 1);
    assert!(agent_history(&history).contains("terminal failure"));
    assert_eq!(history.without_transient_errors().items.len(), 4);
}

fn item(turn: &str, role: Role, phase: Option<&str>, text: &str) -> TextItem {
    TextItem {
        turn_id: turn.into(),
        item_id: text.into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: Vec::new(),
    }
}

#[test]
fn last_interaction_excludes_incomplete_finals_and_does_not_guess_from_truncated_history() {
    let mut partial = item("t", Role::Agent, Some("final_answer"), "still incomplete");
    partial.complete = false;
    let mut history = History {
        all_pages_read: true,
        items: vec![item("t", Role::User, None, "latest question"), partial],
    };
    let interaction = last_interaction(&history).unwrap();
    assert_eq!(interaction.user.text, "latest question");
    assert!(interaction.answers.is_empty());
    history.all_pages_read = false;
    assert!(last_interaction(&history).is_none());
}

#[test]
fn never_pairs_a_new_user_input_with_an_old_answer() {
    let history = History {
        all_pages_read: true,
        items: vec![
            item("old", Role::User, None, "old question"),
            item("old", Role::Agent, Some("final_answer"), "old answer"),
            item("new", Role::User, None, "new question"),
            item("new", Role::Agent, Some("commentary"), "working"),
        ],
    };
    let text = latest_interaction(&history);
    assert!(text.contains("new question"));
    assert!(!text.contains("old answer"));
    assert!(!text.contains("working"));
}

#[test]
fn sustained_messages_have_a_fixed_flush_deadline() {
    let now = Instant::now();
    let mut batch = Batcher::new(Duration::from_secs(2), Duration::from_secs(5), 3);
    batch.push(now, "one".into());
    batch.push(now + Duration::from_secs(1), "two".into());
    batch.push(now + Duration::from_secs(2), "three".into());
    assert!(
        batch
            .take_ready(now + Duration::from_secs(4), false)
            .is_none()
    );
    batch.push(now + Duration::from_secs(4), "four".into());
    assert!(
        batch
            .take_ready(now + Duration::from_secs(5), false)
            .unwrap()
            .contains("four")
    );
}

#[test]
fn a_late_steer_does_not_inherit_an_earlier_final_in_the_same_turn() {
    let history = History {
        all_pages_read: true,
        items: vec![
            item("t", Role::User, None, "question"),
            item("t", Role::Agent, Some("final_answer"), "earlier answer"),
            item("t", Role::User, None, "late steer"),
        ],
    };
    let output = latest_interaction(&history);
    assert!(output.contains("late steer"));
    assert!(!output.contains("earlier answer"));
}
