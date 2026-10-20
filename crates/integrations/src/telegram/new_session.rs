//! Host/workspace selection for /new; title input never becomes an agent task.
use super::*;

impl Service {
    pub(in crate::telegram::runtime) async fn new_hosts(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        self.host_page(input, snapshot, 0, true).await
    }

    pub(super) async fn new_workspaces(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        host_id: &str,
    ) -> Result<(), Error> {
        self.new_workspace_page(input, snapshot, host_id, 0).await
    }

    pub(super) async fn new_workspace_page(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        host_id: &str,
        offset: usize,
    ) -> Result<(), Error> {
        let (snapshot, incomplete) = self.workspace_snapshot(input, snapshot, host_id).await?;
        let Ok(view) = crate::navigation::Navigator::new(&snapshot.config).projects_page(
            host_id,
            offset,
            PROJECT_PAGE_SIZE,
        ) else {
            return self.new_hosts(input, &snapshot).await;
        };
        let host = view.host;
        let page = view.projects;
        let mut choices: Vec<_> = page
            .items
            .iter()
            .map(|w| Choice {
                label: format!("📁 {}", w.name).into(),
                action: Action::NewPrompt {
                    workspace: w.id.clone(),
                },
            })
            .collect();
        if let Some(offset) = page.next {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: Action::NewHostPage {
                    id: host_id.into(),
                    offset,
                },
            });
        }
        let mut note = format!(
            "{}\n\n{}",
            crate::telegram::i18n::tr(input, "tg.new.choose_workspace"),
            navigation_model::page_counter(&page, crate::telegram::i18n::lang(input))
        );
        if incomplete {
            note.push_str(&format!(
                "\n{}",
                crate::telegram::i18n::tr(input, "tg.workspace.incomplete")
            ));
        }
        self.show_navigation(
            input,
            &snapshot,
            Menu {
                heading: format!(
                    "✨ {} › {}",
                    host.name,
                    crate::telegram::i18n::tr(input, "nav.workspaces")
                )
                .into(),
                note: note.into(),
                choices,
                back: page
                    .previous
                    .map_or(Action::NewSession, |offset| Action::NewHostPage {
                        id: host_id.into(),
                        offset,
                    }),
                refresh: Action::NewHostPage {
                    id: host.id.clone(),
                    offset: page.offset,
                },
                scope: None,
            },
        )
        .await
    }

    pub(super) fn workspace_breadcrumb(
        &self,
        config: &engine::config::Config,
        workspace: &Workspace,
    ) -> Result<String, Error> {
        let host = crate::navigation::Navigator::new(config)
            .parent(&workspace.id)
            .map_err(|_| runtimes::Error::Selection)?
            .host;
        Ok(format!("{} › {}", host.name, workspace.name))
    }

    pub(super) async fn new_prompt(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace_id: &str,
    ) -> Result<(), Error> {
        let workspace = self.workspace(&snapshot.config, workspace_id)?;
        let host_id = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .map(|p| p.host_id.clone())
            .ok_or(runtimes::Error::Selection)?;
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!(
                    "✨ {}",
                    self.workspace_breadcrumb(&snapshot.config, workspace)?
                )
                .into(),
                note: ::i18n::t!(
                    "tg.new.title_prompt",
                    locale = crate::telegram::i18n::lang(input),
                    cwd = &workspace.cwd
                )
                .into(),
                choices: Vec::new(),
                back: Action::NewHost { id: host_id },
                refresh: Action::NewPrompt {
                    workspace: workspace.id.clone(),
                },
                scope: Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?),
            },
        )
        .await
    }

    pub(super) async fn capture_new_title(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<bool, Error> {
        let text = input.text.trim();
        if text.is_empty() || text.starts_with('/') || matches!(text, "0" | "返回" | "刷新") {
            return Ok(false);
        }
        let Some(saved) = self.store.navigation(self.bot, input).await? else {
            return Ok(false);
        };
        if !saved.menu.choices.is_empty() {
            return Ok(false);
        }
        let Action::NewPrompt { workspace } = &saved.menu.refresh else {
            return Ok(false);
        };
        if input
            .reply_to_message_id
            .is_some_and(|id| Some(id) != saved.message)
            || input.message_id <= saved.after
            || saved.consumed
            || saved
                .menu
                .scope
                .as_ref()
                .is_none_or(|s| !self.scope_allowed(&snapshot.config, s))
        {
            self.new_hosts(input, snapshot).await?;
        } else if saved.published.is_none() {
            self.respond(
                input,
                &format!("menu-loading:{}", input.event_key),
                Outbound::ui("tg.nav.loading"),
            )
            .await?;
        } else if self.store.claim_navigation(&saved.id).await? {
            self.confirm_creation(input, snapshot, workspace, text)
                .await?;
        }
        Ok(true)
    }
}
