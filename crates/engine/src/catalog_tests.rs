use super::*;

fn configuration(path: &Path, text: &str) -> Config {
    let mut config: Config = toml::from_str(text).unwrap();
    config.prepare(path, text).unwrap();
    config
}

const ROOT: &str = "[[hosts]]\nid='host'\nname='fixture'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n";

#[tokio::test]
async fn catalog_administration_coexists_with_the_bridge_and_keeps_stable_ids() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    let service = crate::storage::Store::open(dir.path()).await.unwrap();
    let endpoint = codex::discovery::Endpoint {
        binary: "/bin/codex".into(),
        socket: "/tmp/control.sock".into(),
    };
    let a = catalog.discover("host", endpoint.clone()).await.unwrap();
    let b = catalog.discover("host", endpoint).await.unwrap();
    assert_eq!(a.id, b.id);
    assert!(!a.control_verified);
    let mut config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    let w = catalog
        .add_workspace(&config, &a.id, "/projects/app", None)
        .await
        .unwrap();
    let same = catalog
        .add_workspace(&config, &a.id, "/projects/app", None)
        .await
        .unwrap();
    assert_eq!(w.id, same.id);
    assert!(
        catalog
            .add_workspace(&config, &a.id, "/outside", None)
            .await
            .is_err()
    );
    catalog.load(&mut config).await.unwrap();
    assert_eq!(config.workspaces.len(), 1);
    config.hosts.clear();
    catalog.load(&mut config).await.unwrap();
    assert!(config.providers.is_empty() && config.workspaces.is_empty());
    config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    assert_eq!(config.workspaces[0].id, w.id);
    catalog.remove_workspace(&w.id).await.unwrap();
    catalog.load(&mut config).await.unwrap();
    assert!(config.workspaces.is_empty());
    drop(service);
}

#[tokio::test]
async fn changed_endpoints_do_not_inherit_control_or_retarget_old_providers() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    let mut original = catalog
        .discover(
            "host",
            codex::discovery::Endpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/old.sock".into(),
            },
        )
        .await
        .unwrap();
    original.control_verified = true;
    catalog.save_provider(&original).await.unwrap();
    let new = catalog
        .discover(
            "host",
            codex::discovery::Endpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/new.sock".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(new.id, original.id);
    assert!(!new.control_verified);
    let mut config = configuration(&dir.path().join("pocketpilot.toml"), ROOT);
    catalog.load(&mut config).await.unwrap();
    assert!(
        config
            .providers
            .iter()
            .find(|p| p.id == original.id)
            .unwrap()
            .control_verified
    );
}

#[tokio::test]
async fn catalog_refresh_cannot_restore_a_host_removed_by_config_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pocketpilot.toml");
    std::fs::write(&path, ROOT).unwrap();
    let file = crate::reload::ConfigFile::open(path.clone(), |path, text| {
        let mut config: Config = toml::from_str(text).map_err(|_| ConfigError::Parse)?;
        config.prepare(path, text)?;
        Ok(crate::reload::ParsedConfig {
            config,
            restart_key: String::new(),
        })
    })
    .unwrap();
    let catalog = Catalog::open(dir.path()).await.unwrap();
    catalog
        .discover(
            "host",
            codex::discovery::Endpoint {
                binary: "/bin/codex".into(),
                socket: "/tmp/old.sock".into(),
            },
        )
        .await
        .unwrap();
    let mut config = file.snapshot().config.clone();
    catalog.load(&mut config).await.unwrap();
    file.publish_catalog(config.clone()).unwrap();
    let old_hash = file.snapshot().hash.clone();
    std::fs::write(&path, "").unwrap();
    file.reload().unwrap();
    assert!(file.snapshot().config.providers.is_empty());
    assert!(!file.publish_catalog(config).unwrap());
    assert_ne!(old_hash, file.snapshot().hash);
    assert!(file.snapshot().config.hosts.is_empty());
}
