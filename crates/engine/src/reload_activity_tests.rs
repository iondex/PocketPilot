use super::*;

#[test]
fn workspace_activity_publishes_without_expiring_authorized_menus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, "[[hosts]]\nid='h'\nname='host'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n").unwrap();
    let file = ConfigFile::open(path, |path, text| {
        let mut config: Config = toml::from_str(text).unwrap();
        config.prepare(path, text)?;
        config.providers.push(crate::config::Provider {
            id: "p".into(),
            host_id: "h".into(),
            kind: "codex".into(),
            connection_mode: "existing_proxy".into(),
            compatibility_profile: "fixture".into(),
            control_verified: false,
            endpoint: None,
        });
        config.workspaces.push(crate::config::Workspace {
            id: "w".into(),
            provider_id: "p".into(),
            name: "project".into(),
            short_name: "project".into(),
            cwd: "/projects/test".into(),
            additional_cwds: vec![],
            updated_at: None,
        });
        Ok(ParsedConfig {
            config,
            restart_key: String::new(),
        })
    })
    .unwrap();
    file.publish_catalog(file.snapshot().config.clone())
        .unwrap();
    let before = file.snapshot();
    let mut next = before.config.clone();
    next.workspaces[0].updated_at = Some(100);
    assert!(file.publish_catalog(next).unwrap());
    assert_eq!(file.snapshot().hash, before.hash);
    assert_eq!(file.snapshot().config.workspaces[0].updated_at, Some(100));
    let mut next = file.snapshot().config.clone();
    next.workspaces[0].cwd = "/projects/changed".into();
    assert!(file.publish_catalog(next).unwrap());
    assert_ne!(file.snapshot().hash, before.hash);
}
