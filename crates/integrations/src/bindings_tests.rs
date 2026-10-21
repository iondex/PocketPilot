use super::*;

fn config() -> Config {
    let mut config: Config = toml::from_str("[[hosts]]\nid='host'\nname='Development'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']").unwrap();
    config.providers.push(engine::model::Provider {
        id: "runtime".into(),
        host_id: "host".into(),
        kind: "codex".into(),
        connection_mode: "existing_proxy".into(),
        compatibility_profile: "fixture".into(),
        control_verified: false,
        endpoint: None,
    });
    config.workspaces.push(engine::config::Workspace {
        id: "project".into(),
        provider_id: "runtime".into(),
        name: "Example".into(),
        short_name: "example".into(),
        cwd: "/projects/example".into(),
        additional_cwds: vec![],
        updated_at: None,
    });
    config
}

async fn seed(core: &Store, config: &Config, index: i32) {
    let thread = format!("session-{index}");
    let endpoint = runtimes::fingerprint(&config.hosts[0], &config.providers[0]);
    sqlx::query("INSERT INTO bindings(bot_id,chat_id,topic_id,instance_id,thread_id,workspace_id) VALUES(1,2,?,'runtime',?,'project')")
        .bind(index).bind(&thread).execute(core.pool()).await.unwrap();
    sqlx::query("INSERT INTO discord_bindings(bot,guild,channel,owner,workspace,instance,thread,cwd,endpoint) VALUES('1','2',?,'3','project','runtime',?,'/projects/example',?)")
        .bind(index.to_string()).bind(&thread).bind(&endpoint).execute(core.pool()).await.unwrap();
    sqlx::query("INSERT INTO slack_bindings(id,app,team,bot,channel,root,owner,workspace,instance,thread,cwd,endpoint) VALUES(?,'A','T','B','D',?,'U','project','runtime',?,'/projects/example',?)")
        .bind(index.to_string()).bind(format!("{index}.000001")).bind(&thread).bind(endpoint).execute(core.pool()).await.unwrap();
}

fn repositories(core: &Store) -> Vec<(Box<dyn Repository>, Scope, Target)> {
    vec![
        (
            Box::new(crate::telegram::storage::Store::new(core.clone())),
            Scope::Telegram { bot: 1, chat: 2 },
            Target::Telegram { topic: 1 },
        ),
        (
            Box::new(crate::discord::storage::Store::new(core.clone())),
            Scope::Discord {
                bot: 1,
                guild: 2,
                owner: 3,
            },
            Target::Discord {
                channel: "1".into(),
            },
        ),
        (
            Box::new(crate::slack::storage::Store::new(
                core.clone(),
                "A",
                "T",
                "B",
            )),
            Scope::Slack {
                app: "A".into(),
                team: "T".into(),
                bot: "B".into(),
                channel: "D".into(),
                owner: "U".into(),
            },
            Target::Slack { id: "1".into() },
        ),
    ]
}

#[tokio::test]
async fn all_platforms_share_revision_checked_observation_and_detach_rules() {
    let dir = tempfile::tempdir().unwrap();
    let core = Store::open(dir.path()).await.unwrap();
    let config = config();
    seed(&core, &config, 1).await;
    for (repository, scope, target) in repositories(&core) {
        let manager = Manager::new(repository.as_ref(), scope.clone());
        let original = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(original.state, State::Watching);
        assert!(observing(&core, &scope, &target).await.unwrap());
        manager
            .change(&target, original.version, Change::Pause)
            .await
            .unwrap();
        assert!(!observing(&core, &scope, &target).await.unwrap());
        let paused = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(paused.state, State::Paused);
        assert!(
            paused.record.active,
            "paused bindings still accept task input"
        );
        assert_eq!(paused.version.binding, original.version.binding);
        assert_eq!(
            paused.observation_change(),
            Some(("bindings.resume", Change::Resume))
        );
        // A stale detach must roll back the native UPDATE as well as preferences.
        assert!(
            manager
                .change(&target, original.version, Change::Detach)
                .await
                .is_err()
        );
        assert_eq!(
            manager.entry(&config, &target, "en").await.unwrap().state,
            State::Paused
        );
        let (left, right) = tokio::join!(
            manager.change(&target, paused.version, Change::Resume),
            manager.change(&target, paused.version, Change::Resume)
        );
        assert_ne!(
            left.is_ok(),
            right.is_ok(),
            "only one concurrent action wins"
        );
        let resumed = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(resumed.state, State::Watching);
        manager
            .change(&target, resumed.version, Change::Detach)
            .await
            .unwrap();
        let detached = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(detached.state, State::Detached);
        assert_eq!(detached.version.binding, original.version.binding + 1);
        assert_eq!(detached.observation_change(), None);
        assert!(
            manager
                .change(&target, resumed.version, Change::Detach)
                .await
                .is_err()
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operations")
        .fetch_one(core.pool())
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "binding management never dispatches runtime mutations"
    );
}

#[tokio::test]
async fn inventory_uses_cached_metadata_and_removed_projects_remain_manageable() {
    let dir = tempfile::tempdir().unwrap();
    let core = Store::open(dir.path()).await.unwrap();
    let mut config = config();
    for index in 1..=10 {
        seed(&core, &config, index).await;
    }
    remember(
        &core,
        &session::Session {
            reference: session::SessionRef {
                instance: "runtime".into(),
                thread: "session-1".into(),
            },
            title: "Readable title 🧭".into(),
            native_title: None,
            cwd: "/projects/example".into(),
            status: session::RuntimeStatus::Idle,
            active_turn: None,
            updated_at: None,
        },
    )
    .await
    .unwrap();
    for (repository, scope, target) in repositories(&core) {
        let manager = Manager::new(repository.as_ref(), scope.clone());
        let page = manager.page(&config, 0, "en").await.unwrap();
        assert_eq!((page.total, page.items.len(), page.next), (10, 8, Some(8)));
        let last = manager.page(&config, 8, "en").await.unwrap();
        assert_eq!(
            (last.items.len(), last.previous, last.next),
            (2, Some(0), None)
        );
        let entry = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(entry.title, "Readable title 🧭");
        assert_eq!(entry.workspace, "Development");
        assert_eq!(entry.project, "Example");
        assert_eq!(entry.runtime_status, Some(session::RuntimeStatus::Idle));
        assert!(entry.checked_at.is_some());
        assert_eq!(
            entry.jump_url(&scope).is_some(),
            !matches!(scope, Scope::Telegram { .. })
        );
    }
    config.workspaces.clear();
    for (repository, scope, target) in repositories(&core) {
        let manager = Manager::new(repository.as_ref(), scope);
        let invalid = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(invalid.state, State::Invalid);
        assert_eq!(invalid.observation_change(), None);
        manager
            .change(&target, invalid.version, Change::Detach)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn inventory_and_changes_are_isolated_by_native_account_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let core = Store::open(dir.path()).await.unwrap();
    let config = config();
    seed(&core, &config, 1).await;
    for (repository, scope, target) in repositories(&core) {
        let mut wrong_scopes = Vec::new();
        match scope {
            Scope::Telegram { bot, chat } => {
                wrong_scopes.push(Scope::Telegram { bot: bot + 1, chat });
                wrong_scopes.push(Scope::Telegram {
                    bot,
                    chat: chat + 1,
                });
            }
            Scope::Discord { bot, guild, owner } => {
                wrong_scopes.push(Scope::Discord {
                    bot: bot + 1,
                    guild,
                    owner,
                });
                wrong_scopes.push(Scope::Discord {
                    bot,
                    guild: guild + 1,
                    owner,
                });
                wrong_scopes.push(Scope::Discord {
                    bot,
                    guild,
                    owner: owner + 1,
                });
            }
            Scope::Slack {
                app,
                team,
                bot,
                channel,
                owner,
            } => {
                for field in 0..5 {
                    let mut values = [
                        app.clone(),
                        team.clone(),
                        bot.clone(),
                        channel.clone(),
                        owner.clone(),
                    ];
                    values[field].push('X');
                    let [app, team, bot, channel, owner] = values;
                    wrong_scopes.push(Scope::Slack {
                        app,
                        team,
                        bot,
                        channel,
                        owner,
                    });
                }
            }
        }
        for scope in wrong_scopes {
            let manager = Manager::new(repository.as_ref(), scope);
            assert!(match manager.page(&config, 0, "en").await {
                Ok(page) => page.total == 0,
                Err(_) => true,
            });
            assert!(manager.entry(&config, &target, "en").await.is_err());
            assert!(
                manager
                    .change(
                        &target,
                        Version {
                            binding: 1,
                            observation: 0
                        },
                        Change::Pause
                    )
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn changed_endpoint_ledger_marks_every_platform_binding_invalid_without_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let core = Store::open(dir.path()).await.unwrap();
    let config = config();
    seed(&core, &config, 1).await;
    core.check_instance("runtime", "previous-execution-endpoint", true)
        .await
        .unwrap();
    for (repository, scope, target) in repositories(&core) {
        let manager = Manager::new(repository.as_ref(), scope);
        let entry = manager.entry(&config, &target, "en").await.unwrap();
        assert_eq!(entry.state, State::Invalid);
        assert_eq!(entry.observation_change(), None);
        manager
            .change(&target, entry.version, Change::Detach)
            .await
            .unwrap();
    }
}
