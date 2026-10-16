//! Host/workspace selection for /new; title input never becomes an agent task.
use super::*;

impl Service {
    pub(in crate::runtime) async fn new_hosts(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let choices = snapshot
            .config
            .hosts
            .iter()
            .map(|host| Choice {
                label: format!("🌐 {}", host.name).into(),
                action: Action::NewHost {
                    id: host.id.clone(),
                },
            })
            .collect();
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.new.heading"),
                note: UiText::id("tg.new.choose_host"),
                choices,
                back: Action::Home,
                refresh: Action::NewSession,
                scope: None,
            },
        )
        .await
    }

    pub(super) async fn new_workspaces(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        host_id: &str,
    ) -> Result<(), Error> {
        let Some(host) = snapshot.config.hosts.iter().find(|h| h.id == host_id) else {
            return self.new_hosts(input, snapshot).await;
        };
        let choices = snapshot
            .config
            .workspaces
            .iter()
            .filter(|w| {
                snapshot
                    .config
                    .providers
                    .iter()
                    .any(|p| p.id == w.provider_id && p.host_id == host.id)
            })
            .map(|w| Choice {
                label: format!(
                    "📁 {} · {}",
                    w.name,
                    session::delivery::short_title(&w.cwd, 50)
                )
                .into(),
                action: Action::NewPrompt {
                    workspace: w.id.clone(),
                },
            })
            .collect();
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!(
                    "✨ {} › {}",
                    host.name,
                    crate::i18n::tr(input, "nav.workspaces")
                )
                .into(),
                note: UiText::id("tg.new.choose_workspace"),
                choices,
                back: Action::NewSession,
                refresh: Action::NewHost {
                    id: host.id.clone(),
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
        let host = config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .and_then(|p| config.hosts.iter().find(|h| h.id == p.host_id))
            .ok_or(connections::Error::Selection)?;
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
            .ok_or(connections::Error::Selection)?;
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
                    locale = crate::i18n::lang(input),
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
