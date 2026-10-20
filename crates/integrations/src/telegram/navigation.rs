//! Durable inline management screens. Navigation never grants agent permissions.
use super::*;
use crate::telegram::ledger::Outbound;
use engine::reload::Snapshot;
use navigation_model::{Choice, Menu, UiText};
use navigation_store::SavedMenu;

#[path = "navigation_model.rs"]
mod navigation_model;

#[path = "navigation_store.rs"]
mod navigation_store;

#[path = "topics.rs"]
mod topics;

#[path = "new_session.rs"]
mod new_session;

#[path = "catalog.rs"]
mod catalog;

const HOST_PAGE_SIZE: usize = 8;

const PROJECT_PAGE_SIZE: usize = 8;

impl Service {
    async fn show_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        menu: Menu,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let previous = self.store.navigation(self.bot, input).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut body = Outbound::text(menu.render(crate::telegram::i18n::lang(input)));
        body.markdown = true;
        body.buttons = menu.buttons(&id, crate::telegram::i18n::lang(input));
        let fresh_entry = input
            .text
            .split_whitespace()
            .next()
            .is_some_and(|command| command.split('@').next() == Some("/start"));
        if !fresh_entry {
            body.edit_message = previous.as_ref().and_then(|saved| saved.message);
        }
        body.navigation_id = Some(id.clone());
        body.scope = menu.scope.clone();
        self.store
            .save_navigation(self.bot, input, &id, &menu)
            .await?;
        // Pairing is checked again by the sender. Scope is checked when selecting
        // a choice, so display/rate changes do not expire a harmless menu.
        self.store
            .enqueue(
                self.bot,
                input.chat,
                input.topic,
                &format!("menu:{}", input.event_key),
                &body,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn workspaces(&self, input: &Input, snapshot: &Snapshot) -> Result<(), Error> {
        self.host_page(input, snapshot, 0, false).await
    }

    pub(super) async fn host_page(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        offset: usize,
        creating: bool,
    ) -> Result<(), Error> {
        let page = crate::navigation::Navigator::new(&snapshot.config)
            .workspaces_page(offset, HOST_PAGE_SIZE);
        let mut choices: Vec<_> = page
            .items
            .iter()
            .map(|view| {
                let host = view.host;
                Choice {
                    label: format!(
                        "{} {}",
                        if matches!(host.transport, engine::config::TransportKind::SSHProxy) {
                            "🌐"
                        } else {
                            "💻"
                        },
                        host.name
                    )
                    .into(),
                    action: if creating {
                        Action::NewHost {
                            id: host.id.clone(),
                        }
                    } else {
                        Action::Host {
                            id: host.id.clone(),
                        }
                    },
                }
            })
            .collect();
        if let Some(offset) = page.next {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: host_page_action(offset, creating),
            });
        }
        if !creating {
            choices.extend([
                Choice {
                    label: UiText::id("tg.new.heading"),
                    action: Action::NewSession,
                },
                Choice {
                    label: UiText::id("tg.topics.heading"),
                    action: Action::Topics { after: 0 },
                },
            ]);
        }
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id(if creating {
                    "tg.new.heading"
                } else {
                    "tg.nav.hosts_heading"
                }),
                note: UiText::id(if creating {
                    "tg.new.choose_host"
                } else {
                    "tg.nav.hosts_note"
                }),
                choices,
                back: page
                    .previous
                    .map_or(Action::Home, |offset| host_page_action(offset, creating)),
                refresh: host_page_action(page.offset, creating),
                scope: None,
            },
        )
        .await
    }

    pub(super) async fn host_workspaces(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
    ) -> Result<(), Error> {
        self.host_workspace_page(input, snapshot, id, 0).await
    }

    pub(super) async fn host_workspace_page(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        offset: usize,
    ) -> Result<(), Error> {
        let (snapshot, incomplete) = self.workspace_snapshot(input, snapshot, id).await?;
        let Ok(view) = crate::navigation::Navigator::new(&snapshot.config).projects_page(
            id,
            offset,
            PROJECT_PAGE_SIZE,
        ) else {
            return self.workspaces(input, &snapshot).await;
        };
        let host = view.host;
        let page = view.projects;
        let mut choices: Vec<_> = page
            .items
            .iter()
            .map(|w| Choice {
                label: format!("📁 {}", w.name).into(),
                action: Action::Workspace {
                    id: w.id.clone(),
                    cursor: None,
                },
            })
            .collect();
        if let Some(offset) = page.next {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: Action::HostPage {
                    id: id.into(),
                    offset,
                },
            });
        }
        let mut note = crate::telegram::i18n::tr(input, "tg.nav.workspace_actions");
        note.push_str(&format!(
            "\n\n{}",
            navigation_model::page_counter(&page, crate::telegram::i18n::lang(input))
        ));
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
                    "📁 {} › {}",
                    host.name,
                    crate::telegram::i18n::tr(input, "nav.workspaces")
                )
                .into(),
                note: note.into(),
                choices,
                back: page
                    .previous
                    .map_or(Action::Home, |offset| Action::HostPage {
                        id: id.into(),
                        offset,
                    }),
                refresh: Action::HostPage {
                    id: id.into(),
                    offset: page.offset,
                },
                scope: None,
            },
        )
        .await
    }

    pub(super) async fn sessions(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        cursor: Option<String>,
    ) -> Result<(), Error> {
        let loaded = match crate::navigation::Navigator::new(&snapshot.config)
            .sessions_page(&self.store, &self.runtimes, id, cursor.clone(), 6)
            .await
        {
            Err(runtimes::Error::Selection) => return self.workspaces(input, snapshot).await,
            result => result?,
        };
        let workspace = loaded.selection.project;
        let host = loaded.selection.host.id.clone();
        let project_offset = crate::navigation::Navigator::new(&snapshot.config)
            .project_offset(&workspace.id, PROJECT_PAGE_SIZE)
            .map_err(|_| runtimes::Error::Selection)?;
        let page = loaded.sessions;
        let mut choices = page
            .data
            .into_iter()
            .map(|s| Choice {
                label: format!(
                    "{} · {}",
                    crate::telegram::presentation::emoji(&s.status),
                    session::delivery::short_title(&s.title, 40)
                )
                .into(),
                action: Action::Open {
                    workspace: workspace.id.clone(),
                    thread: s.reference.thread,
                },
            })
            .collect::<Vec<_>>();
        choices.push(Choice {
            label: UiText::id("tg.new.heading"),
            action: Action::NewPrompt {
                workspace: workspace.id.clone(),
            },
        });
        if let Some(cursor) = page.next_cursor {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: Some(cursor),
                },
            });
        }
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!(
                    "💬 {} › {}",
                    self.workspace_breadcrumb(&snapshot.config, workspace)?,
                    crate::telegram::i18n::tr(input, "nav.sessions")
                )
                .into(),
                note: String::new().into(),
                choices,
                back: Action::HostPage {
                    id: host,
                    offset: project_offset,
                },
                refresh: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor,
                },
                scope: Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?),
            },
        )
        .await
    }

    pub(super) async fn confirm_creation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        text: &str,
    ) -> Result<(), Error> {
        let workspace = self.workspace(&snapshot.config, id)?;
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!(
                    "✨ {} › {}",
                    self.workspace_breadcrumb(&snapshot.config, workspace)?,
                    crate::telegram::i18n::tr(input, "nav.new_session")
                )
                .into(),
                note: ::i18n::t!(
                    "tg.new.confirm",
                    locale = crate::telegram::i18n::lang(input),
                    title = text,
                    cwd = &workspace.cwd
                )
                .into(),
                choices: vec![Choice {
                    label: UiText::id("tg.new.confirm_action"),
                    action: Action::Create {
                        workspace: workspace.id.clone(),
                        text: text.into(),
                    },
                }],
                back: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: None,
                },
                refresh: Action::NewPrompt {
                    workspace: workspace.id.clone(),
                },
                scope: Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?),
            },
        )
        .await
    }

    pub(super) async fn opened_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
    ) -> Result<(), Error> {
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.session.added"),
                note: UiText::id("tg.session.first_message"),
                choices: vec![Choice {
                    label: UiText::id("nav.back_sessions"),
                    action: Action::Workspace {
                        id: workspace.id.clone(),
                        cursor: None,
                    },
                }],
                back: Action::Home,
                refresh: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: None,
                },
                scope: None,
            },
        )
        .await
    }

    async fn show_action(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        action: Action,
    ) -> Result<(), Error> {
        match action {
            Action::Home => self.workspaces(input, snapshot).await,
            Action::HomePage { offset } => self.host_page(input, snapshot, offset, false).await,
            Action::NewHostsPage { offset } => self.host_page(input, snapshot, offset, true).await,
            Action::NewSession => self.new_hosts(input, snapshot).await,
            Action::NewHost { id } => self.new_workspaces(input, snapshot, &id).await,
            Action::NewHostPage { id, offset } => {
                self.new_workspace_page(input, snapshot, &id, offset).await
            }
            Action::Host { id } => self.host_workspaces(input, snapshot, &id).await,
            Action::HostPage { id, offset } => {
                self.host_workspace_page(input, snapshot, &id, offset).await
            }
            Action::Workspace { id, cursor } => self.sessions(input, snapshot, &id, cursor).await,
            Action::Topics { after } => self.topics(input, snapshot, after).await,
            Action::ConfirmTopicRemoval { topic } => {
                self.confirm_topic_removal(input, snapshot, topic).await
            }
            Action::ConfirmClearTopics => self.confirm_clear_topics(input, snapshot).await,
            Action::NewPrompt { workspace } => self.new_prompt(input, snapshot, &workspace).await,
            _ => self.workspaces(input, snapshot).await,
        }
    }

    pub(super) async fn select_navigation_button(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        if self.store.management_topic(self.bot, input.chat).await? != Some(input.topic) {
            return Ok(());
        }
        let Some(data) = input
            .callback
            .as_deref()
            .and_then(|s| s.strip_prefix("nav:"))
        else {
            return Ok(());
        };
        let Some((id, selection)) = data.split_once(':') else {
            return Ok(());
        };
        let mut trusted = input.clone();
        // Selection below is authorized from the private persisted page, never callback payload actions.
        trusted.callback = None;
        trusted.callback_id = None;
        let Some(saved) = self.store.navigation(self.bot, &trusted).await? else {
            return self.workspaces(&trusted, snapshot).await;
        };
        if saved.published.is_none() {
            if self.store.navigation_pending(self.bot, &saved.id).await? {
                return Ok(());
            }
            return self
                .restore_navigation(&trusted, snapshot, saved.menu)
                .await;
        }
        if saved.consumed {
            // A claimed write may already have reached the provider even if its UI update did not.
            return self
                .show_action(&trusted, snapshot, saved.menu.refresh)
                .await;
        }
        if saved.id != id || saved.message != Some(input.message_id) {
            return self
                .restore_navigation(&trusted, snapshot, saved.menu)
                .await;
        }
        let action = match selection {
            "b" => return self.show_action(&trusted, snapshot, saved.menu.back).await,
            "h" => return self.workspaces(&trusted, snapshot).await,
            "r" => {
                trusted.text = "/refresh".into();
                return self
                    .show_action(&trusted, snapshot, saved.menu.refresh)
                    .await;
            }
            _ => match selection
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|n| saved.menu.choices.get(n))
            {
                Some(choice) => choice.action.clone(),
                None => {
                    return self
                        .show_action(&trusted, snapshot, saved.menu.refresh)
                        .await;
                }
            },
        };
        // Telegram callbacks contain the menu's original message timestamp, not click time.
        // Mutating confirmations instead rely on our delivery receipt and inbox recovery marker.
        let stale = input.requires_confirmation
            || saved
                .published
                .is_none_or(|at| at < self.started_at || now().saturating_sub(at) > 300);
        if stale {
            match &action {
                Action::Create { workspace, text } => {
                    return self
                        .confirm_creation(&trusted, snapshot, workspace, text)
                        .await;
                }
                Action::RemoveTopic { topic } => {
                    return self.confirm_topic_removal(&trusted, snapshot, *topic).await;
                }
                Action::ClearTopics { .. } => {
                    return self.confirm_clear_topics(&trusted, snapshot).await;
                }
                Action::Open { .. } => {
                    return self
                        .show_action(&trusted, snapshot, saved.menu.refresh)
                        .await;
                }
                _ => {}
            }
        }
        trusted.date = now();
        self.apply_navigation(&trusted, snapshot, saved, action)
            .await
    }

    async fn restore_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        menu: Menu,
    ) -> Result<(), Error> {
        let confirmation = menu.choices.iter().any(|choice| {
            matches!(
                choice.action,
                Action::Create { .. } | Action::RemoveTopic { .. } | Action::ClearTopics { .. }
            )
        });
        if confirmation
            && menu
                .scope
                .as_ref()
                .is_none_or(|scope| self.scope_allowed(&snapshot.config, scope))
        {
            self.show_navigation(input, snapshot, menu).await
        } else {
            self.show_action(input, snapshot, menu.refresh).await
        }
    }

    pub(super) async fn select_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        if self.capture_new_title(input, snapshot).await? {
            return Ok(());
        }
        let text = input.text.trim();
        let number = if matches!(text, "/back" | "返回") {
            Some(0)
        } else {
            text.parse::<usize>().ok()
        };
        if number.is_none() && !matches!(text, "/refresh" | "刷新") {
            return self
                .respond(
                    input,
                    &format!("management-reject:{}", input.event_key),
                    Outbound::ui("tg.management.reject"),
                )
                .await;
        }
        let Some(saved) = self.store.navigation(self.bot, input).await? else {
            return self.workspaces(input, snapshot).await;
        };
        if matches!(text, "/refresh" | "刷新") {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        let Some(number) = number else {
            return self
                .respond(
                    input,
                    &format!("management-reject:{}", input.event_key),
                    Outbound::ui("tg.management.reject"),
                )
                .await;
        };
        if input
            .reply_to_message_id
            .is_some_and(|id| Some(id) != saved.message)
            || input.message_id <= saved.after
            || saved.consumed
        {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        if saved.published.is_none() {
            return self
                .respond(
                    input,
                    &format!("menu-loading:{}", input.event_key),
                    Outbound::ui("tg.nav.loading"),
                )
                .await;
        }
        let action = if number == 0 {
            saved.menu.back.clone()
        } else if let Some(choice) = saved.menu.choices.get(number - 1) {
            choice.action.clone()
        } else {
            return self
                .respond(
                    input,
                    &format!("menu-range:{}", input.event_key),
                    Outbound::ui("tg.nav.invalid_number"),
                )
                .await;
        };
        if number == 0 {
            return self.show_action(input, snapshot, action).await;
        }
        self.apply_navigation(input, snapshot, saved, action).await
    }

    async fn apply_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        saved: SavedMenu,
        action: Action,
    ) -> Result<(), Error> {
        if saved
            .menu
            .scope
            .as_ref()
            .is_some_and(|scope| !self.scope_allowed(&snapshot.config, scope))
        {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        if let Action::Create { workspace, text } = &action
            && (self.delayed(input) || saved.published.is_some_and(|at| input.date < at))
        {
            return self
                .confirm_creation(input, snapshot, workspace, text)
                .await;
        }
        if let Action::RemoveTopic { topic } = action
            && (self.delayed(input) || saved.published.is_some_and(|at| input.date < at))
        {
            return self.confirm_topic_removal(input, snapshot, topic).await;
        }
        if matches!(action, Action::ClearTopics { .. })
            && (self.delayed(input) || saved.published.is_some_and(|at| input.date < at))
        {
            return self.confirm_clear_topics(input, snapshot).await;
        }
        if !self.store.claim_navigation(&saved.id).await? {
            return Ok(());
        }
        match action {
            Action::Open { workspace, thread } => {
                self.open(input, snapshot, &workspace, &thread).await
            }
            Action::Create { workspace, text } => {
                self.create(input, snapshot, &workspace, &text).await
            }
            Action::RemoveTopic { topic } => self.remove_topic(input, snapshot, topic).await,
            Action::ClearTopics { topics } => self.clear_topics(input, snapshot, topics).await,
            action => self.show_action(input, snapshot, action).await,
        }
    }
}

impl Service {
    pub(super) async fn add_workspace(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        args: &str,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let Some(menu) = self.store.navigation(self.bot, input).await? else {
            return self.workspaces(input, snapshot).await;
        };
        let host_id = match menu.menu.refresh {
            Action::Host { id } | Action::HostPage { id, .. } => id,
            _ => {
                return self
                    .respond(
                        input,
                        &input.event_key,
                        Outbound::ui("tg.workspace.choose_host"),
                    )
                    .await;
            }
        };
        let providers: Vec<_> = snapshot
            .config
            .providers
            .iter()
            .filter(|p| p.host_id == host_id)
            .collect();
        if providers.len() != 1 {
            return self
                .respond(
                    input,
                    &input.event_key,
                    Outbound::text(if providers.is_empty() {
                        "tg.workspace.no_provider"
                    } else {
                        "tg.workspace.multiple_providers"
                    }),
                )
                .await;
        }
        let (cwd, name) = args
            .split_once(char::is_whitespace)
            .map_or((args, None), |(p, n)| (p, Some(n.trim())));
        let catalog = engine::catalog::Catalog::new(self.store.pool().clone());
        catalog
            .add_workspace(&snapshot.config, &providers[0].id, cwd, name)
            .await?;
        let next = self.refresh_catalog_snapshot(&catalog).await?;
        let mut routed = input.clone();
        routed.config_hash = Some(next.hash.clone());
        self.host_workspaces(&routed, &next, &host_id).await
    }

    pub(super) async fn remove_workspace(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let catalog = engine::catalog::Catalog::new(self.store.pool().clone());
        catalog.remove_workspace(&workspace.id).await?;
        let next = self.refresh_catalog_snapshot(&catalog).await?;
        let mut routed = input.clone();
        routed.config_hash = Some(next.hash.clone());
        self.workspaces(&routed, &next).await
    }
}

fn host_page_action(offset: usize, creating: bool) -> Action {
    match (creating, offset) {
        (false, 0) => Action::Home,
        (true, 0) => Action::NewSession,
        (false, offset) => Action::HomePage { offset },
        (true, offset) => Action::NewHostsPage { offset },
    }
}
