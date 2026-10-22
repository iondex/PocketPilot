use super::*;

#[test]
fn doctor_combines_offline_checks_with_explicit_network_flags() {
    let args = Arguments::try_parse_from([
        "pocketpilot",
        "doctor",
        "--credentials",
        "--connect",
        "--telegram",
        "--discord",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Command::Doctor(crate::doctor::Options {
            credentials: true,
            connect: true,
            telegram: true,
            discord: true,
            feishu: false,
            lark: false,
        })
    ));
    let args = Arguments::try_parse_from(["pocketpilot", "doctor"]).unwrap();
    assert!(matches!(
        args.command,
        Command::Doctor(crate::doctor::Options {
            credentials: false,
            connect: false,
            telegram: false,
            discord: false,
            feishu: false,
            lark: false,
        })
    ));
    assert!(Arguments::try_parse_from(["pocketpilot", "config-check"]).is_err());
}

#[test]
fn cli_errors_keep_their_type_and_render_the_selected_language() {
    let english = Failure {
        error: Error::Provider(session::ProviderError::Rejected(-32600)),
        locale: "en".into(),
    };
    let chinese = Failure {
        error: Error::Provider(session::ProviderError::Rejected(-32600)),
        locale: "zh-CN".into(),
    };
    assert!(english.to_string().contains("provider rejected"));
    assert!(chinese.to_string().contains("执行服务拒绝"));
    assert!(english.to_string().contains("-32600") && chinese.to_string().contains("-32600"));
}

#[test]
fn retry_is_explicit_and_can_run_without_chat_integrations() {
    assert!(matches!(
        Arguments::try_parse_from(["pocketpilot", "retry"])
            .unwrap()
            .command,
        Command::Retry
    ));
    assert!(matches!(
        Arguments::try_parse_from(["pocketpilot", "serve"])
            .unwrap()
            .command,
        Command::Serve
    ));
    assert!(Arguments::try_parse_from(["pocketpilot", "serve", "--retry"]).is_err());
}

#[test]
fn lark_probe_is_explicit_and_can_be_combined_with_feishu() {
    let args = Arguments::try_parse_from(["pocketpilot", "doctor", "--lark", "--feishu"]).unwrap();
    assert!(matches!(
        args.command,
        Command::Doctor(crate::doctor::Options {
            feishu: true,
            lark: true,
            connect: false,
            ..
        })
    ));
}
