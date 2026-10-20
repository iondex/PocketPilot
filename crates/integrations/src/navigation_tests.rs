use super::*;

fn catalog() -> Config {
    let mut config: Config = toml::from_str("[[hosts]]\nid='a'\nname='Alpha'\ntransport='local_proxy'\n[[hosts]]\nid='b'\nname='Beta'\ntransport='local_proxy'").unwrap();
    for (id, host) in [("pa", "a"), ("pb", "b")] {
        config.providers.push(engine::model::Provider {
            id: id.into(),
            host_id: host.into(),
            kind: "codex".into(),
            connection_mode: "existing_proxy".into(),
            compatibility_profile: "fixture".into(),
            control_verified: false,
            endpoint: None,
        });
    }
    for (id, provider, name) in [
        ("a2", "pa", "Zeta"),
        ("b1", "pb", "Remote"),
        ("a1", "pa", "Alpha"),
    ] {
        config.workspaces.push(Workspace {
            id: id.into(),
            provider_id: provider.into(),
            name: name.into(),
            short_name: id.into(),
            cwd: format!("/projects/{id}"),
            additional_cwds: Vec::new(),
            updated_at: None,
        });
    }
    config
}

#[test]
fn every_integration_uses_the_same_order_and_cannot_cross_workspaces() {
    let config = catalog();
    let navigation = Navigator::new(&config);
    assert_eq!(
        navigation
            .workspaces()
            .iter()
            .map(|v| v.host.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        navigation
            .workspace("Alpha")
            .unwrap()
            .projects
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["a1", "a2"]
    );
    assert!(navigation.project("a", "b1").is_err());
    assert_eq!(
        navigation
            .project("Beta", "/projects/b1")
            .unwrap()
            .project
            .id,
        "b1"
    );
    assert_eq!(navigation.parent("b1").unwrap().host.id, "b");
}

#[test]
fn removed_or_ambiguous_selections_do_not_fall_through_to_another_host() {
    let mut config = catalog();
    config.hosts[1].name = "Alpha".into();
    assert!(Navigator::new(&config).workspace("Alpha").is_err());
    assert!(Navigator::new(&config).workspace("a").is_ok());
    config.hosts.retain(|host| host.id != "b");
    assert!(Navigator::new(&config).parent("b1").is_err());
}

#[test]
fn paging_is_bounded_and_shrinking_catalogs_return_to_the_first_page() {
    let middle = page((0..25).collect(), 8, 8);
    assert_eq!(middle.items, (8..16).collect::<Vec<_>>());
    assert_eq!((middle.previous, middle.next), (Some(0), Some(16)));
    let shrunk = page(vec![1, 2], 80, usize::MAX);
    assert_eq!(shrunk.offset, 0);
    assert_eq!(shrunk.items, [1, 2]);
    assert_eq!(page(Vec::<u8>::new(), usize::MAX, 0).total, 0);
}

#[test]
fn project_pages_follow_activity_then_case_insensitive_name_path_and_id() {
    let mut config = catalog();
    config.workspaces[0].updated_at = Some(200);
    config.workspaces[2].updated_at = Some(100);
    let navigation = Navigator::new(&config);
    let first = navigation.projects_page("a", 0, 1).unwrap();
    assert_eq!(first.host.id, "a");
    assert_eq!(first.projects.items[0].id, "a2");
    assert_eq!(first.projects.next, Some(1));
    let second = navigation.projects_page("a", 1, 1).unwrap();
    assert_eq!(second.projects.items[0].id, "a1");
    assert_eq!(second.projects.previous, Some(0));
    assert_eq!(second.projects.next, None);
    assert_eq!(navigation.project_offset("a1", 1).unwrap(), 1);
    assert_eq!(navigation.project_offset("a1", 100).unwrap(), 0);
    assert_eq!(navigation.workspaces_page(1, 1).items[0].host.id, "b");

    let mut same = config.workspaces[0].clone();
    same.id = "a3".into();
    same.name = "zeta".into();
    same.cwd = "/projects/a0".into();
    config.workspaces.push(same);
    let ordered = Navigator::new(&config).workspace("a").unwrap();
    assert_eq!(
        ordered
            .projects
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["a3", "a2", "a1"]
    );
}

#[test]
fn names_paths_and_host_aliases_share_resolution_and_stable_ids_take_precedence() {
    let mut config = catalog();
    config.hosts[0].ssh_alias = Some("local-alias".into());
    config.workspaces[0].short_name = "a1".into();
    let navigation = Navigator::new(&config);
    assert_eq!(navigation.parent("a1").unwrap().project.id, "a1");
    assert_eq!(
        navigation.project("local-alias", "a1").unwrap().project.id,
        "a1"
    );
    assert_eq!(
        navigation.parent("local-alias/Zeta").unwrap().project.id,
        "a2"
    );
    assert_eq!(navigation.parent("/projects/a2").unwrap().host.id, "a");
    config.workspaces[1].name = "Zeta".into();
    assert!(matches!(
        Navigator::new(&config).parent("Zeta"),
        Err(Error::Ambiguous)
    ));
    assert_eq!(
        Navigator::new(&config).parent("a/Zeta").unwrap().project.id,
        "a2"
    );
    assert!(Navigator::new(&config).parent("b/a2").is_err());
    config.workspaces[0].short_name = "b1".into();
    assert!(matches!(
        Navigator::new(&config).project("a", "b1"),
        Err(Error::Selection)
    ));
}

#[tokio::test]
async fn session_navigation_rejects_a_removed_host_before_connecting() {
    let mut config = catalog();
    config.hosts.retain(|host| host.id != "b");
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    let runtimes = RuntimeRegistry::new(store.clone());
    assert!(matches!(
        Navigator::new(&config)
            .sessions_page(&store, &runtimes, "b1", None, 8)
            .await,
        Err(runtimes::Error::Selection)
    ));
}
