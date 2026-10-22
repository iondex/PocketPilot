use super::Owner;

#[test]
fn owner_is_valid_and_scoped_to_their_own_private_chat() {
    assert!(Owner::new(0).is_none());
    assert!(Owner::new(u64::MAX).is_none());
    let owner = Owner::new(10).unwrap();
    assert!(owner.authorizes(10, 10, true));
    assert!(!owner.authorizes(10, -100, false));
    assert!(!owner.authorizes(10, 20, true));
    assert!(!owner.authorizes(20, 10, true));
    assert!(!owner.authorizes(20, 20, true));
    let other = Owner::new(20).unwrap();
    assert!(other.authorizes(20, 20, true));
    assert!(!other.chat(10));
    assert!(!other.authorizes(10, 10, true));
}
