use super::*;

#[test]
fn bad_reload_keeps_the_authoritative_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, "[delivery]\nburst_threshold=3").unwrap();
    let config = ConfigFile::open(path.clone(), |path, text| {
        let mut config: Config = toml::from_str(text).map_err(|_| ConfigError::Parse)?;
        config.prepare(path, text)?;
        Ok(ParsedConfig {
            config,
            restart_key: String::new(),
        })
    })
    .unwrap();
    let original = config.snapshot();
    std::fs::write(&path, "include=['other.toml']").unwrap();
    assert!(config.reload().is_err());
    assert_eq!(config.snapshot().hash, original.hash);
    std::fs::write(&path, "[delivery]\nburst_threshold=5").unwrap();
    assert!(config.reload().unwrap());
    assert_eq!(config.snapshot().config.delivery.burst_threshold, 5);
}
