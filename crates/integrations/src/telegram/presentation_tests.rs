use super::*;

#[test]
fn titles_have_no_workspace_or_identity_prefix_and_waiting_is_not_typing() {
    assert_eq!(title("原始标题"), "原始标题");
    assert_eq!(
        emoji(&RuntimeStatus::NotLoaded),
        emoji(&RuntimeStatus::Idle)
    );
    assert!(title(&"长".repeat(100)).chars().count() <= 40);
    assert!(!typing(&RuntimeStatus::Active {
        waiting_on_approval: true,
        waiting_on_user_input: false
    }));
    assert!(typing(&RuntimeStatus::Active {
        waiting_on_approval: false,
        waiting_on_user_input: false
    }));
}
