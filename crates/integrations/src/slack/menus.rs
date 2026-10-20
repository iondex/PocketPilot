use super::*;
use crate::slack::render;
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
            c.blocks.push(render::button(
                &session::markdown::preview(
                    &format!(
                        "{} · {}",
                        host.name,
                        i18n::t!("slack.menu.count", locale = &locale, count = count)
                    ),
                    75,
                ),
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
        let provider = self.context.runtimes.get(&snapshot.config, ws).await?;
        let page = engine::runtimes::list_picker(
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
#[path = "menus_tests.rs"]
mod tests;
