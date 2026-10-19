use super::*;
use crate::render;
use slack_morphism::prelude::*;

impl Service {
    pub(super) async fn workspace_menu(
        &self,
        r: &Request,
        hash: &str,
        snapshot: &Snapshot,
        offset: usize,
    ) -> Result<()> {
        let locale = self.locale();
        let page = engine::navigation::page(
            engine::navigation::Navigator::new(&snapshot.config).workspaces(),
            offset,
            8,
        );
        let mut c = render::message(i18n::text(&locale, "slack.workspaces"));
        c.blocks = vec![
            render::header(&i18n::text(&locale, "slack.workspaces.choose")),
            render::context(&i18n::text(&locale, "slack.dm.help")),
        ];
        c.text = format!(
            "{}\n{}\n{}",
            c.text,
            i18n::text(&locale, "slack.dm.help"),
            page.items
                .iter()
                .map(|view| view.host.name.clone())
                .collect::<Vec<_>>()
                .join("\n")
        );
        if page.items.is_empty() {
            c.blocks.push(render::plain(&i18n::text(
                &locale,
                "slack.workspaces.empty",
            )));
        }
        for view in page.items {
            let host = view.host;
            let count = view.projects.len();
            let nonce = self
                .store
                .action(
                    r,
                    Action::Run(Command::Projects {
                        host: host.id.clone(),
                        offset: 0,
                    }),
                    hash,
                )
                .await?;
            c.blocks.push(render::section_action(
                &host.name,
                &i18n::t!("slack.menu.count", locale = &locale, count = count),
                &i18n::text(&locale, "slack.choose"),
                &nonce,
            ));
        }
        if let Some(offset) = page.previous {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.previous",
                    Action::Run(Command::Workspaces { offset }),
                )
                .await?,
            );
        }
        if let Some(offset) = page.next {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.next",
                    Action::Run(Command::Workspaces { offset }),
                )
                .await?,
            );
        }
        self.menu_output(r, c, hash).await
    }

    pub(super) async fn project_menu(
        &self,
        r: &Request,
        hash: &str,
        snapshot: &Snapshot,
        host: &str,
        offset: usize,
    ) -> Result<()> {
        if host.is_empty() {
            return self.workspace_menu(r, hash, snapshot, 0).await;
        }
        let locale = self.locale();
        let view = engine::navigation::Navigator::new(&snapshot.config)
            .workspace(host)
            .map_err(|_| Error::Ui("error.navigation.selection"))?;
        let host = view.host;
        let page = engine::navigation::page(view.projects, offset, 80);
        let mut options = Vec::new();
        for ws in page.items {
            let nonce = self
                .store
                .action(
                    r,
                    Action::Run(Command::Sessions {
                        workspace: ws.id.clone(),
                        cursor: None,
                    }),
                    hash,
                )
                .await?;
            options.push(
                SlackBlockChoiceItem::new(pt!(session::delivery::short_title(&ws.name, 75)), nonce)
                    .with_description(pt!(session::delivery::short_title(&ws.cwd, 75))),
            );
        }
        let mut c = render::message(format!(
            "{} · {}",
            host.name,
            i18n::text(&locale, "slack.projects")
        ));
        c.blocks = vec![
            render::header(&host.name),
            render::context(&i18n::t!(
                "slack.menu.count",
                locale = &locale,
                count = page.total
            )),
        ];
        if options.is_empty() {
            c.blocks
                .push(render::plain(&i18n::text(&locale, "slack.menu.empty")));
        } else {
            c.blocks.push(render::project_select(
                &i18n::text(&locale, "slack.menu.choose"),
                vec![SlackBlockOptionGroup::new(
                    pt!(session::delivery::short_title(&host.name, 75)),
                    options,
                )],
            ));
        }
        if let Some(offset) = page.previous {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.previous",
                    Action::Run(Command::Projects {
                        host: host.id.clone(),
                        offset,
                    }),
                )
                .await?,
            );
        }
        if let Some(offset) = page.next {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.next",
                    Action::Run(Command::Projects {
                        host: host.id.clone(),
                        offset,
                    }),
                )
                .await?,
            );
        }
        c.blocks.push(
            self.action_button(r, hash, "slack.workspaces.back", Action::Run(Command::Menu))
                .await?,
        );
        self.menu_output(r, c, hash).await
    }

    pub(super) async fn session_menu(
        &self,
        r: &Request,
        hash: &str,
        snapshot: &Snapshot,
        workspace: &str,
        cursor: Option<String>,
    ) -> Result<()> {
        let locale = self.locale();
        let selected = engine::navigation::Navigator::new(&snapshot.config)
            .parent(workspace)
            .map_err(|_| Error::Ui("error.navigation.selection"))?;
        let ws = selected.project;
        let host = selected.host;
        let provider = self.context.connections.get(&snapshot.config, ws).await?;
        let page = engine::connections::list_picker(
            &self.store.core,
            provider.as_ref(),
            ws,
            cursor.clone(),
            8,
        )
        .await?;
        self.fresh(r, hash)?;
        let mut c = render::message(ws.name.clone());
        let mut summary = Vec::new();
        c.blocks = vec![
            render::header(&ws.name),
            render::context(&format!("{} · {}", host.name, ws.cwd)),
        ];
        c.blocks.push(
            self.action_button(
                r,
                hash,
                "slack.new",
                Action::NewModal {
                    workspace: workspace.into(),
                },
            )
            .await?,
        );
        if page.data.is_empty() {
            c.blocks
                .push(render::plain(&i18n::text(&locale, "slack.sessions.empty")));
        }
        for session in page.data {
            summary.push(format!(
                "{} · {}",
                session::delivery::short_title(&session.title, 100),
                render::state(&session.status, &locale)
            ));
            let nonce = self
                .store
                .action(
                    r,
                    Action::Run(Command::Open {
                        workspace: workspace.into(),
                        thread: session.reference.thread.clone(),
                    }),
                    hash,
                )
                .await?;
            c.blocks.push(render::section_action(
                &session.title,
                &render::state(&session.status, &locale),
                &i18n::text(&locale, "slack.open"),
                &nonce,
            ));
        }
        c.text = format!("{}\n{}", c.text, summary.join("\n"));
        if let Some(cursor) = page.next_cursor {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.next",
                    Action::Run(Command::Sessions {
                        workspace: workspace.into(),
                        cursor: Some(cursor),
                    }),
                )
                .await?,
            );
        }
        if cursor.is_some() {
            c.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.sessions.first",
                    Action::Run(Command::Sessions {
                        workspace: workspace.into(),
                        cursor: None,
                    }),
                )
                .await?,
            );
        }
        c.blocks.push(
            self.action_button(
                r,
                hash,
                "slack.menu.back",
                Action::Run(Command::Projects {
                    host: host.id.clone(),
                    offset: 0,
                }),
            )
            .await?,
        );
        self.menu_output(r, c, hash).await
    }

    pub(super) async fn menu_output(
        &self,
        r: &Request,
        mut content: Content,
        hash: &str,
    ) -> Result<()> {
        render::compact_actions(&mut content);
        let mut delivery = r.clone();
        delivery.binding = None;
        delivery.revision = None;
        if let Some(message) = &r.menu_message {
            delivery.root = Some(message.clone());
            content.edit = true;
        }
        self.store.output(&delivery, &content, hash).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[tokio::test]
    async fn workspace_then_project_menus_filter_scope_and_update_the_existing_screen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocketpilot.toml");
        std::fs::write(&path, "[[hosts]]\nid='alpha'\nname='Alpha'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n[[hosts]]\nid='beta'\nname='Beta'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']").unwrap();
        let config = Arc::new(
            engine::reload::ConfigFile::open(path, |path, text| {
                let mut config: engine::config::Config = toml::from_str(text).unwrap();
                config.prepare(path, text)?;
                for id in ["alpha", "beta"] {
                    config.providers.push(engine::model::Provider {
                        id: format!("p-{id}"),
                        host_id: id.into(),
                        kind: "codex".into(),
                        connection_mode: "existing_proxy".into(),
                        compatibility_profile: "fixture".into(),
                        control_verified: false,
                        endpoint: None,
                    });
                    config.workspaces.push(engine::config::Workspace {
                        id: format!("project-{id}"),
                        provider_id: format!("p-{id}"),
                        name: format!("Project {id}"),
                        short_name: id.into(),
                        cwd: format!("/projects/{id}"),
                        additional_cwds: Vec::new(),
                        updated_at: None,
                    });
                }
                config.validate()?;
                Ok(engine::reload::ParsedConfig {
                    config,
                    restart_key: String::new(),
                })
            })
            .unwrap(),
        );
        let core = engine::storage::Store::open(dir.path()).await.unwrap();
        let (_stop, receiver) = watch::channel(false);
        let service = Service {
            settings: crate::config::Config {
                proxy: Default::default(),
                app_token: "xapp-fixture".into(),
                bot_token: "xoxb-fixture".into(),
                app_id: "A1".into(),
                team_id: "T1".into(),
                owner_user_id: "U1".into(),
            },
            context: Context {
                config: config.clone(),
                connections: Arc::new(engine::connections::Connections::new(core.clone())),
                store: core.clone(),
            },
            store: Store::new(core, "A1", "T1", "U2"),
            api: Arc::new(crate::tests::NoNetwork),
            bot: "U2".into(),
            direct_channel: "D1".into(),
            stop: receiver,
            cutoff: AtomicI64::new(0),
            verified: AtomicBool::new(true),
        };
        let snapshot = config.snapshot();
        let mut r = Request {
            id: "menu".into(),
            channel: "D1".into(),
            user: "U1".into(),
            root: None,
            menu_message: None,
            binding: None,
            revision: None,
            date: engine::now(),
            command: Command::Menu,
        };
        service
            .workspace_menu(&r, &snapshot.hash, &snapshot, 0)
            .await
            .unwrap();
        let row = sqlx::query("SELECT content FROM slack_outbox WHERE source='menu'")
            .fetch_one(service.store.core.pool())
            .await
            .unwrap();
        let workspace_menu: Content = serde_json::from_str(row.get("content")).unwrap();
        assert_eq!(
            workspace_menu.blocks.len(),
            4,
            "one header, one hint and two workspace rows"
        );
        assert!(
            workspace_menu.blocks[2..]
                .iter()
                .all(|b| matches!(b, SlackBlock::Section(s) if s.accessory.is_some()))
        );
        r.id = "projects".into();
        r.menu_message = Some("100.000001".into());
        service
            .project_menu(&r, &snapshot.hash, &snapshot, "alpha", 0)
            .await
            .unwrap();
        let row = sqlx::query("SELECT content,root FROM slack_outbox WHERE source='projects'")
            .fetch_one(service.store.core.pool())
            .await
            .unwrap();
        let project_menu: Content = serde_json::from_str(row.get("content")).unwrap();
        assert!(project_menu.edit);
        assert_eq!(row.get::<String, _>("root"), "100.000001");
        let SlackBlock::Actions(actions) = &project_menu.blocks[2] else {
            panic!("expected a project selector")
        };
        let SlackActionBlockElement::StaticSelect(select) = &actions.elements[0] else {
            panic!("expected a native static select")
        };
        let groups = select.option_groups.as_ref().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].options.len(),
            1,
            "projects from Beta must not appear under Alpha"
        );
        let nonce = &groups[0].options[0].value;
        let (selected, _) = service
            .store
            .consume(
                nonce,
                Some("D1"),
                "U1",
                &snapshot.hash,
                None,
                InteractionSource {
                    id: Some("selection"),
                    message: Some("100.000001"),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(selected.command, Command::Sessions { workspace, .. } if workspace == "project-alpha")
        );
        assert_eq!(selected.menu_message.as_deref(), Some("100.000001"));
        assert!(
            selected.root.is_none(),
            "navigation message must not become an agent session root"
        );
        if let Some(output) = std::env::var_os("POCKETPILOT_BLOCK_PREVIEW") {
            let output = std::path::PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(
                output.join("slack-workspaces.json"),
                serde_json::to_string_pretty(&workspace_menu).unwrap(),
            )
            .unwrap();
            std::fs::write(
                output.join("slack-projects.json"),
                serde_json::to_string_pretty(&project_menu).unwrap(),
            )
            .unwrap();
            let mut sessions = render::message("Project alpha · sessions".into());
            sessions.blocks = vec![
                render::header("Project alpha"),
                render::context("Alpha · /projects/alpha"),
                render::section_action(
                    "Review deployment",
                    &render::state(&session::RuntimeStatus::NotLoaded, "en"),
                    "Open session",
                    "fixture-a",
                ),
                render::section_action(
                    "Inspect logs",
                    &render::state(&session::RuntimeStatus::Idle, "en"),
                    "Open session",
                    "fixture-b",
                ),
                render::button("New session", "fixture-new"),
                render::button("Switch project", "fixture-back"),
            ];
            render::compact_actions(&mut sessions);
            assert_eq!(
                sessions.blocks.len(),
                5,
                "session buttons belong beside their titles; related footer controls share one row"
            );
            std::fs::write(
                output.join("slack-sessions.json"),
                serde_json::to_string_pretty(&sessions).unwrap(),
            )
            .unwrap();
        }
    }
}
