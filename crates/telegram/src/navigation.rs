//! Durable, numbered private management screens. Navigation never grants agent permissions.
use super::*;
use crate::ledger::{Outbound, Scope};
use engine::reload::Snapshot;
use sqlx::Row;

#[path = "topics.rs"]
mod topics;

#[path = "new_session.rs"]
mod new_session;

#[path = "workspace_picker.rs"]
mod workspace_picker;

#[derive(Clone, Serialize, Deserialize)]
struct Choice {
    label: UiText,
    action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
struct Menu {
    heading: UiText,
    note: UiText,
    choices: Vec<Choice>,
    back: Action,
    refresh: Action,
    scope: Option<Scope>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum UiText {
    Message { id: String },
    Literal(String),
}

impl UiText {
    fn id(id: &str) -> Self {
        Self::Message { id: id.into() }
    }

    fn render(&self, locale: &str) -> String {
        match self {
            Self::Message { id } => ::i18n::text(locale, id),
            Self::Literal(text) => text.clone(),
        }
    }
}

impl From<String> for UiText {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

impl From<&str> for UiText {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}

struct SavedMenu {
    id: String,
    menu: Menu,
    after: i32,
    message: Option<i32>,
    published: Option<i64>,
    consumed: bool,
}

impl Store {
    async fn navigation(&self, bot: i64, input: &Input) -> Result<Option<SavedMenu>, StoreError> {
        let row=sqlx::query("SELECT id,payload,after_message_id,message_id,published_at,consumed FROM navigation WHERE bot_id=? AND chat_id=? AND topic_id=? AND user_id=?")
            .bind(bot).bind(input.chat).bind(input.topic).bind(input.user as i64).fetch_optional(&self.pool).await.map_err(|_|StoreError::Database)?;
        row.map(|row| {
            Ok(SavedMenu {
                id: row.get("id"),
                menu: serde_json::from_str(row.get("payload")).map_err(|_| StoreError::Database)?,
                after: row.get("after_message_id"),
                message: row.get("message_id"),
                published: row.get("published_at"),
                consumed: row.get("consumed"),
            })
        })
        .transpose()
    }

    async fn save_navigation(
        &self,
        bot: i64,
        input: &Input,
        id: &str,
        menu: &Menu,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO navigation(bot_id,chat_id,topic_id,user_id,id,payload,after_message_id) VALUES(?,?,?,?,?,?,?) ON CONFLICT(bot_id,chat_id,topic_id) DO UPDATE SET user_id=excluded.user_id,id=excluded.id,payload=excluded.payload,after_message_id=excluded.after_message_id,published_at=NULL,consumed=0")
            .bind(bot).bind(input.chat).bind(input.topic).bind(input.user as i64).bind(id)
            .bind(serde_json::to_string(menu).map_err(|_|StoreError::Database)?).bind(input.message_id)
            .execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }

    pub async fn navigation_receipt(
        &self,
        id: &str,
        message: i32,
        time: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE navigation SET message_id=?,published_at=? WHERE id=?")
            .bind(message)
            .bind(time)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        Ok(())
    }

    async fn claim_navigation(&self, id: &str) -> Result<bool, StoreError> {
        Ok(
            sqlx::query("UPDATE navigation SET consumed=1 WHERE id=? AND consumed=0")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|_| StoreError::Database)?
                .rows_affected()
                == 1,
        )
    }
}

fn md(text: &str) -> String {
    crate::api::escape_markdown(text)
}

impl Menu {
    fn render(&self, locale: &str) -> String {
        let mut text = format!("*{}*\n", md(&self.heading.render(locale)));
        let note = self.note.render(locale);
        if !note.is_empty() {
            text.push_str(&format!("\n{}\n", md(&note)));
        }
        for (index, choice) in self.choices.iter().enumerate() {
            text.push_str(&format!(
                "\n*{}*  {}",
                index + 1,
                md(&choice.label.render(locale))
            ));
        }
        text.push_str(&crate::i18n::text(locale, "tg.nav.footer"));
        text
    }
}

impl Service {
    async fn show_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        menu: Menu,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut body = Outbound::text(menu.render(crate::i18n::lang(input)));
        body.markdown = true;
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
        let mut choices: Vec<_> = snapshot
            .config
            .hosts
            .iter()
            .map(|host| Choice {
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
                action: Action::Host {
                    id: host.id.clone(),
                },
            })
            .collect();
        choices.push(Choice {
            label: UiText::id("tg.topics.heading"),
            action: Action::Topics { after: 0 },
        });
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.nav.hosts_heading"),
                note: UiText::id("tg.nav.hosts_note"),
                choices,
                back: Action::Home,
                refresh: Action::Home,
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
        let Some(host) = snapshot.config.hosts.iter().find(|h| h.id == id) else {
            return self.workspaces(input, &snapshot).await;
        };
        let page = workspace_picker::page(&snapshot.config, id, offset);
        let mut choices: Vec<_> = page
            .entries
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
        let mut note = crate::i18n::tr(input, "tg.nav.workspace_actions");
        note.push_str(&format!("\n\n{}", page.counter(input)));
        if incomplete {
            note.push_str(&format!(
                "\n{}",
                crate::i18n::tr(input, "tg.workspace.incomplete")
            ));
        }
        self.show_navigation(
            input,
            &snapshot,
            Menu {
                heading: format!(
                    "📁 {} › {}",
                    host.name,
                    crate::i18n::tr(input, "nav.workspaces")
                )
                .into(),
                note: note.into(),
                choices,
                back: if page.offset > 0 {
                    Action::HostPage {
                        id: id.into(),
                        offset: page.offset.saturating_sub(workspace_picker::PAGE_SIZE),
                    }
                } else {
                    Action::Home
                },
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
        let Ok(workspace) = self.workspace(&snapshot.config, id) else {
            return self.workspaces(input, snapshot).await;
        };
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let page =
            connections::list_picker(&self.store, provider.as_ref(), workspace, cursor.clone(), 6)
                .await?;
        let mut choices = page
            .data
            .into_iter()
            .map(|s| Choice {
                label: format!(
                    "{} · {}",
                    crate::presentation::emoji(&s.status),
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
        let host = snapshot
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
                    "💬 {} › {}",
                    self.workspace_breadcrumb(&snapshot.config, workspace)?,
                    crate::i18n::tr(input, "nav.sessions")
                )
                .into(),
                note: String::new().into(),
                choices,
                back: Action::Host { id: host },
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
                    crate::i18n::tr(input, "nav.new_session")
                )
                .into(),
                note: ::i18n::t!(
                    "tg.new.confirm",
                    locale = crate::i18n::lang(input),
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
        if number != 0
            && saved
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
        let mut config = self.config.snapshot().config.clone();
        catalog.load(&mut config).await?;
        self.config.publish_catalog(config)?;
        let next = self.config.snapshot();
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
        let mut config = self.config.snapshot().config.clone();
        catalog.load(&mut config).await?;
        self.config.publish_catalog(config)?;
        let next = self.config.snapshot();
        let mut routed = input.clone();
        routed.config_hash = Some(next.hash.clone());
        self.workspaces(&routed, &next).await
    }
}
