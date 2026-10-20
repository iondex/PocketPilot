use super::*;

#[test]
fn discovery_does_not_expand_wildcards_or_negated_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(
        &path,
        "Host alpha beta !blocked *.example\n HostName example.invalid\nHost *\n User user\n",
    )
    .unwrap();
    assert_eq!(discover(&path).unwrap(), vec!["alpha", "beta"]);
}

#[test]
fn keyboard_interactive_is_permission_not_an_otp_requirement() {
    let host = parse_resolved(
        "host",
        "hostname example.invalid\nuser user\nport 22\nkbdinteractiveauthentication yes\n",
    )
    .unwrap();
    assert!(host.keyboard_interactive_allowed);
    assert!(!host.has_proxy);
}
