use super::*;
use std::fs;

#[test]
fn language_is_an_explicit_root_setting_with_a_chinese_default() {
    let path = std::path::Path::new("/tmp/pocketpilot.toml");
    assert_eq!(parse(path, "").unwrap().config.language, "zh-CN");
    assert_eq!(parse(path, "language='en'").unwrap().config.language, "en");
    assert!(parse(path, "language='fr'").is_err());
}

#[test]
fn paths_are_relative_to_the_single_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    fs::write(&path, "[service]\ndata_dir='state'\n").unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(
        loaded.config.service.data_dir,
        dir.path().canonicalize().unwrap().join("state")
    );
    assert_eq!(loaded.sources.len(), 1);
}

#[test]
fn parse_errors_and_debug_output_do_not_echo_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    fs::write(
        &path,
        "[telegram]\nowner_user_id=10\ntoken='private-value'\nunknown='private-value'",
    )
    .unwrap();
    assert!(
        !load(&path)
            .unwrap_err()
            .to_string()
            .contains("private-value")
    );
    assert!(
        !format!(
            "{:?}",
            Telegram {
                owner_user_id: 10,
                token: "private-value".into(),
                default_locale: "zh-CN".into(),
                proxy: Default::default()
            }
        )
        .contains("private-value")
    );
}

#[test]
fn includes_and_configured_user_chat_allowlists_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    for input in [
        "include=[]",
        "[[providers]]\nid='obsolete'",
        "[[workspaces]]\nid='obsolete'",
        "[[hosts]]\nid='h'\nname='h'\ntransport='ssh_proxy'\nssh_alias='h'\ncodex_bin='obsolete'",
        "[[hosts]]\nid='h'\nname='h'\ntransport='ssh_proxy'\nssh_alias='h'\ncodex_control_socket='/obsolete.sock'",
        "[telegram]\nowner_user_id=10\ntoken='t'\nallowed_user_ids=[]",
        "[telegram]\nowner_user_id=10\ntoken='t'\nallowed_chat_ids=[]",
    ] {
        fs::write(&path, input).unwrap();
        assert!(matches!(load(&path), Err(ConfigError::Parse)));
    }
}

#[test]
fn channel_proxies_default_to_direct_validate_and_require_restart() {
    let path = Path::new("/tmp/pocketpilot.toml");
    for text in [
        "[telegram]\nowner_user_id=10\ntoken='fixture'",
        "[discord]\ntoken='fixture'\nguild_id=1",
        "[slack]\napp_token='xapp-fixture'\nbot_token='xoxb-fixture'\napp_id='A1'\nteam_id='T1'\nowner_user_id='U1'",
    ] {
        let before = runtime_parser(path, text).unwrap();
        let empty = runtime_parser(path, &format!("{text}\nproxy='' ")).unwrap();
        assert_eq!(before.restart_key, empty.restart_key);
        let first =
            runtime_parser(path, &format!("{text}\nproxy='http://192.0.2.10:7890'")).unwrap();
        let second =
            runtime_parser(path, &format!("{text}\nproxy='http://192.0.2.11:7890'")).unwrap();
        assert_ne!(before.restart_key, first.restart_key);
        assert_ne!(first.restart_key, second.restart_key);
        assert!(parse(path, &format!("{text}\nproxy='invalid-secret'")).is_err());
    }
}

#[test]
fn public_example_is_a_valid_configuration() {
    load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pocketpilot.example.toml")).unwrap();
}

#[test]
fn telegram_requires_a_valid_owner_and_owner_changes_require_restart() {
    let path = Path::new("/tmp/pocketpilot.toml");
    assert!(parse(path, "[telegram]\ntoken='fixture'").is_err());
    for owner in [0, u64::MAX] {
        assert!(
            parse(
                path,
                &format!("[telegram]\ntoken='fixture'\nowner_user_id={owner}")
            )
            .is_err()
        );
    }
    let first = runtime_parser(path, "[telegram]\ntoken='fixture'\nowner_user_id=10").unwrap();
    let second = runtime_parser(path, "[telegram]\ntoken='fixture'\nowner_user_id=20").unwrap();
    assert_ne!(first.restart_key, second.restart_key);
}

#[test]
fn feishu_is_direct_only_and_rejects_even_an_empty_proxy_setting() {
    let path = Path::new("/tmp/pocketpilot.toml");
    let text = "[feishu]\napp_id='cli_fixture'\napp_secret='fixture'\ntenant_key='tenant'\nowner_open_id='ou_owner'\n[[feishu.groups]]\nchat_id='oc_group'\nname='Fixture'\n";
    assert!(parse(path, text).is_ok());
    for proxy in ["", "http://192.0.2.10:7890"] {
        let configured = text.replace("[feishu]\n", &format!("[feishu]\nproxy='{proxy}'\n"));
        assert!(parse(path, &configured).is_err());
    }
    assert!(parse(path, "[lark]\nproxy=''").is_err());
}

#[test]
fn slack_account_scope_is_validated_redacted_and_requires_restart() {
    let path = Path::new("/tmp/pocketpilot.toml");
    let text = "[slack]\napp_token='xapp-fixture-secret'\nbot_token='xoxb-fixture-secret'\napp_id='A1'\nteam_id='T1'\nowner_user_id='U1'\n";
    let loaded = parse(path, text).unwrap();
    let debug = format!("{:?}", loaded.config);
    assert!(!debug.contains("fixture-secret"));
    assert!(loaded.config.slack.unwrap().credentials_valid());
    assert!(parse(path, &format!("{text}\nallowed_channel_ids=['C1']")).is_err());
    assert!(
        parse(
            path,
            &text.replace("owner_user_id='U1'", "owner_user_id='C1'")
        )
        .is_err()
    );
    let before = runtime_parser(path, text).unwrap();
    let after = runtime_parser(
        path,
        &text.replace("owner_user_id='U1'", "owner_user_id='U3'"),
    )
    .unwrap();
    assert_ne!(before.restart_key, after.restart_key);
}
