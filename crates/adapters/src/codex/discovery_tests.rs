use super::*;

#[test]
fn discovers_only_existing_sockets_and_rejects_shell_noise() {
    assert!(parse("B\t/usr/bin/codex\n").unwrap().is_empty());
    let found = parse("B\t/native/codex\nB\t/wrapper/codex\nS\t/home/.codex/control.sock\nS\t/home/.codex/control.sock\n").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].binary, "/native/codex");
    assert!(parse("welcome to server\n").is_err());
    assert!(parse("B\trelative\n").is_err());
}
