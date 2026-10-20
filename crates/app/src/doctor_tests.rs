use super::*;

#[tokio::test]
async fn offline_report_sorts_hosts_and_skips_network_and_credential_checks() {
    let config = config::parse(
        std::path::Path::new("/tmp/pocketpilot.toml"),
        "[[hosts]]\nid='z'\nname='Zulu'\ntransport='local_proxy'\n\
         [[hosts]]\nid='a'\nname='Alpha'\ntransport='local_proxy'\n\
         [telegram]\ntoken='invalid-fixture-secret'",
    )
    .unwrap()
    .config;
    let options = Options {
        credentials: false,
        connect: false,
        telegram: false,
        discord: false,
    };

    let report = inspect(&config, &options).await.unwrap();

    assert_eq!(
        report
            .hosts
            .iter()
            .map(|host| host.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert!(report.hosts.iter().all(|host| {
        host.connected.is_none() && host.sample_sessions.is_none() && host.reason.is_none()
    }));
    assert!(report.telegram.is_none() && report.discord.is_none());
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("invalid-fixture-secret"));
    assert!(!report.configuration.credentials_checked);
}

#[test]
fn offline_credentials_check_supports_discord_only_and_redacts_tokens() {
    let path = std::path::Path::new("/tmp/pocketpilot.toml");
    let discord = config::parse(
        path,
        "[discord]\ntoken='fixture-discord-secret'\nguild_id=2",
    )
    .unwrap()
    .config;
    let summary = configuration_summary(&discord, true).unwrap();
    let json = serde_json::to_value(summary).unwrap();
    assert_eq!(json["discord_configured"], true);
    assert_eq!(json["telegram_configured"], false);
    assert_eq!(json["credentials_checked"], true);
    assert!(!json.to_string().contains("fixture-discord-secret"));
    let telegram = config::parse(path, "[telegram]\ntoken='invalid-fixture-secret'")
        .unwrap()
        .config;
    assert!(configuration_summary(&telegram, false).is_ok());
    assert!(matches!(
        configuration_summary(&telegram, true),
        Err(Error::Config(config::ConfigError::Credentials))
    ));
}
