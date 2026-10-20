use super::*;

#[test]
fn rfc6238_vector_is_generated_without_reimplementing_totp() {
    let config = TOTP {
        secret_base32: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into(),
        algorithm: "SHA1".into(),
        digits: 8,
        period_seconds: 30,
        minimum_remaining_seconds: 5,
        prompt_patterns: vec!["(?i)^otp: *$".into()],
        password_prompt_patterns: vec![],
    };
    assert_eq!(
        generator(&config).unwrap().generate(59).to_string(),
        "94287082"
    );
    assert!(matches(&config.prompt_patterns, "OTP: ").unwrap());
    assert!(!matches(&config.prompt_patterns, "Approve your login (yes/no)?").unwrap());
}

#[test]
fn duplicate_challenge_in_the_same_window_is_not_answered_again() {
    let dir = tempfile::tempdir().unwrap();
    let config = AuthContext {
        data_dir: dir.path().join("data"),
        source: dir.path().join("pocketpilot.toml"),
        source_hash: "fixture".into(),
    };
    std::fs::create_dir_all(config.data_dir.join("ssh-auth")).unwrap();
    let attempt = uuid::Uuid::new_v4();
    record_window(&config, &attempt, 1, 2).unwrap();
    assert!(record_window(&config, &attempt, 1, 2).is_err());
    record_window(&config, &attempt, 2, 2).unwrap();
    assert!(record_window(&config, &attempt, 3, 2).is_err());
}

#[test]
fn totp_challenge_patterns_match_context_format_without_account_or_address_literals() {
    let patterns = vec![r"(?i)^(?:\([^\r\n()]{1,256}\)\s*)?verification code:\s*$".into()];
    for prompt in [
        "Verification code: ",
        "(operator@host.example) Verification code: ",
        "(other@192.0.2.10) verification code:",
        "(test@[2001:db8::1]) Verification code: ",
    ] {
        assert!(matches(&patterns, prompt).unwrap());
    }
    for prompt in [
        "Password: ",
        "Approve login (yes/no)?",
        "Unrecognized challenge: ",
        "Send verification code: ",
        "Verification code: do something else",
        "(context\nextra) Verification code: ",
    ] {
        assert!(!matches(&patterns, prompt).unwrap());
    }
}
