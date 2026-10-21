//! Native cards adapt the common navigation and binding state machines.
use super::*;
use crate::bindings::{self, Manager, Target};

impl Service {
    async fn control(
        &self,
        card: &mut Card,
        nonces: &mut Vec<String>,
        input: &Input,
        hash: &str,
        label: &str,
        action: Action,
    ) -> Result<()> {
        let nonce = self.store.action(&input.chat, action, hash).await?;
        card.button(label, &nonce);
        nonces.push(nonce);
        Ok(())
    }

    pub(super) async fn menu(&self, input: &Input, hash: &str, action: Action) -> Result<()> {
        let snapshot = self.fresh(input, hash)?;
        let navigator = Navigator::new(&snapshot.config);
        let mut card = Card::new(&self.text("feishu.title"));
        let mut nonces = Vec::new();
        match action {
            Action::Home { offset } => {
                card.markdown(self.text("feishu.workspaces"));
                let page = navigator.workspaces_page(offset, 8);
                for workspace in page.items {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &format!("🖥️ {}", workspace.host.name),
                        Action::Projects {
                            host: workspace.host.id.clone(),
                            offset: 0,
                        },
                    )
                    .await?;
                }
                if let Some(offset) = page.previous {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.previous"),
                        Action::Home { offset },
                    )
                    .await?;
                }
                if let Some(offset) = page.next {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.next"),
                        Action::Home { offset },
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.bindings"),
                    Action::Bindings { offset: 0 },
                )
                .await?;
            }
            Action::Projects { host, offset } => {
                let page = navigator
                    .projects_page(&host, offset, 8)
                    .map_err(|_| Error::Ui("feishu.error.scope"))?;
                card.markdown(format!(
                    "🖥️ **{}**\n\n{}",
                    super::super::render::literal(&page.host.name),
                    self.text("feishu.projects")
                ));
                for project in page.projects.items {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &format!("📁 {}", project.name),
                        Action::Sessions {
                            project: project.id.clone(),
                            cursor: None,
                        },
                    )
                    .await?;
                }
                if let Some(offset) = page.projects.previous {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.previous"),
                        Action::Projects {
                            host: host.clone(),
                            offset,
                        },
                    )
                    .await?;
                }
                if let Some(offset) = page.projects.next {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.next"),
                        Action::Projects {
                            host: host.clone(),
                            offset,
                        },
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Home { offset: 0 },
                )
                .await?;
            }
            Action::Sessions { project, cursor } => {
                let page = navigator
                    .sessions_page(
                        &self.store.core,
                        &self.context.runtimes,
                        &project,
                        cursor,
                        8,
                    )
                    .await?;
                card.markdown(format!(
                    "📁 **{}**\n\n{}",
                    super::super::render::literal(&page.selection.project.name),
                    self.text("feishu.sessions")
                ));
                for session in page.sessions.data {
                    let title = if session.title.trim().is_empty() {
                        self.text("bindings.untitled")
                    } else {
                        session::delivery::short_title(&session.title, 70)
                    };
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &format!(
                            "{} {title}",
                            session::presentation::status_icon(&session.status)
                        ),
                        Action::Groups {
                            project: project.clone(),
                            session: session.reference.thread,
                        },
                    )
                    .await?;
                }
                if let Some(cursor) = page.sessions.next_cursor {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.next"),
                        Action::Sessions {
                            project: project.clone(),
                            cursor: Some(cursor),
                        },
                    )
                    .await?;
                }
                if page.sessions.offset > 0 {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.first"),
                        Action::Sessions {
                            project: project.clone(),
                            cursor: None,
                        },
                    )
                    .await?;
                }
                if snapshot
                    .config
                    .providers
                    .iter()
                    .any(|p| p.id == page.selection.project.provider_id && p.control_verified)
                {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.new"),
                        Action::New {
                            project: project.clone(),
                        },
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Projects {
                        host: page.selection.host.id.clone(),
                        offset: navigator
                            .project_offset(&project, 8)
                            .map_err(|_| Error::Ui("feishu.error.scope"))?,
                    },
                )
                .await?;
            }
            Action::New { project } => {
                let selection = navigator
                    .parent(&project)
                    .map_err(|_| Error::Ui("feishu.error.scope"))?;
                card.markdown(format!(
                    "📁 **{}**\n\n{}",
                    super::super::render::literal(&selection.project.name),
                    self.text("feishu.create_note")
                ));
                let nonce = self
                    .store
                    .action(
                        &input.chat,
                        Action::Create {
                            project: project.clone(),
                        },
                        hash,
                    )
                    .await?;
                card.title_form(
                    &self.text("feishu.name"),
                    &self.text("feishu.create"),
                    &nonce,
                );
                nonces.push(nonce);
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Sessions {
                        project,
                        cursor: None,
                    },
                )
                .await?;
            }
            Action::Create { .. } => return Err(Error::Ui("feishu.error.title")),
            Action::Groups { project, session } => {
                let selection = navigator
                    .parent(&project)
                    .map_err(|_| Error::Ui("feishu.error.scope"))?;
                let provider = self
                    .context
                    .runtimes
                    .get(&snapshot.config, selection.project)
                    .await?;
                let current = provider.read(&session).await?;
                engine::runtimes::require_workspace(&current.cwd, selection.project)?;
                card.markdown(format!(
                    "{}\n\n{}",
                    super::super::render::literal(&current.title),
                    self.text("feishu.groups")
                ));
                for group in &self.settings.groups {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &format!("💬 {}", group.name),
                        Action::Bind {
                            project: project.clone(),
                            session: session.clone(),
                            group: group.chat_id.clone(),
                        },
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Sessions {
                        project,
                        cursor: None,
                    },
                )
                .await?;
            }
            Action::Bind {
                project,
                session,
                group,
            } => {
                let binding = self.bind(input, hash, &project, &session, &group).await?;
                return Box::pin(self.menu(input, hash, Action::Binding { id: binding.id })).await;
            }
            Action::Bindings { offset } => {
                let scope = super::super::binding_store::scope(&self.settings);
                let page = Manager::new(&self.store, scope)
                    .page(&snapshot.config, offset, 8, &self.locale())
                    .await?;
                card.markdown(self.text("feishu.bindings"));
                for entry in page.items {
                    let Target::Feishu { id } = entry.record.target else {
                        continue;
                    };
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &format!("📌 {} · {}", entry.project, entry.title),
                        Action::Binding { id },
                    )
                    .await?;
                }
                if let Some(offset) = page.previous {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.previous"),
                        Action::Bindings { offset },
                    )
                    .await?;
                }
                if let Some(offset) = page.next {
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.next"),
                        Action::Bindings { offset },
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Home { offset: 0 },
                )
                .await?;
            }
            Action::Binding { id } => {
                let b = self.store.binding(&id).await?;
                if b.state == "ready"
                    && let Ok(project) = self.validate(&snapshot, &b)
                {
                    let provider = self.context.runtimes.get(&snapshot.config, project).await?;
                    let current = provider.read(&b.session).await?;
                    engine::runtimes::require_workspace(&current.cwd, project)?;
                    crate::bindings::remember(&self.store.core, &current).await?;
                }
                let scope = super::super::binding_store::scope(&self.settings);
                let entry = Manager::new(&self.store, scope)
                    .entry(
                        &snapshot.config,
                        &Target::Feishu { id: id.clone() },
                        &self.locale(),
                    )
                    .await?;
                card.markdown(super::super::render::literal(
                    &entry.details(&self.locale()),
                ));
                let group = self
                    .settings
                    .groups
                    .iter()
                    .find(|g| g.chat_id == b.chat)
                    .map(|g| g.name.as_str())
                    .unwrap_or_default();
                card.markdown(format!("💬 {}", super::super::render::literal(group)));
                if b.state == "unknown" {
                    card.markdown(self.text("feishu.error.unknown"));
                }
                for control in entry.controls(false) {
                    let action = match control.action {
                        bindings::Action::ConfirmDetach { version } => Action::Change {
                            id: id.clone(),
                            version,
                            change: bindings::Change::Detach,
                            confirmed: false,
                        },
                        bindings::Action::Change { version, change } => Action::Change {
                            id: id.clone(),
                            version,
                            change,
                            confirmed: true,
                        },
                    };
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &i18n::text(&self.locale(), control.label),
                        action,
                    )
                    .await?;
                }
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.refresh"),
                    Action::Binding { id },
                )
                .await?;
                self.control(
                    &mut card,
                    &mut nonces,
                    input,
                    hash,
                    &self.text("feishu.back"),
                    Action::Bindings { offset: 0 },
                )
                .await?;
            }
            Action::Change {
                id,
                version,
                change,
                confirmed,
            } => {
                let scope = super::super::binding_store::scope(&self.settings);
                let target = Target::Feishu { id: id.clone() };
                let manager = Manager::new(&self.store, scope);
                let entry = manager
                    .confirmation(&snapshot.config, &target, Some(version), &self.locale())
                    .await?;
                if change == bindings::Change::Detach && !confirmed {
                    card.markdown(format!(
                        "{}\n\n{}",
                        super::super::render::literal(&entry.details(&self.locale())),
                        self.text("feishu.detach_note")
                    ));
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.confirm"),
                        Action::Change {
                            id: id.clone(),
                            version,
                            change,
                            confirmed: true,
                        },
                    )
                    .await?;
                    self.control(
                        &mut card,
                        &mut nonces,
                        input,
                        hash,
                        &self.text("feishu.back"),
                        Action::Binding { id },
                    )
                    .await?;
                } else {
                    let actor = self
                        .context
                        .runtimes
                        .actor(
                            &entry.record.reference.instance,
                            &entry.record.reference.thread,
                        )
                        .await;
                    let _guard = actor.lock().await;
                    self.fresh(input, hash)?;
                    manager.change(&target, version, change).await?;
                    return Box::pin(self.menu(input, hash, Action::Bindings { offset: 0 })).await;
                }
            }
        }
        self.fresh(input, hash)?;
        self.store
            .output(
                &Destination {
                    chat: input.chat.clone(),
                    root: None,
                },
                &[card],
                hash,
                None,
                None,
                &nonces,
            )
            .await
    }

    pub(super) fn status_card(
        &self,
        snapshot: &engine::reload::Snapshot,
        b: &Binding,
        current: &session::Session,
    ) -> Result<Card> {
        let project = self.validate(snapshot, b)?;
        let title = if current.title.trim().is_empty() {
            self.text("bindings.untitled")
        } else {
            current.title.clone()
        };
        let mut card = Card::new(&title);
        let host = Navigator::new(&snapshot.config)
            .parent(&project.id)
            .map_err(|_| Error::Ui("feishu.error.scope"))?
            .host;
        card.markdown(format!(
            "🖥️ {} · 📁 {}\n\n{}\n\n{}",
            super::super::render::literal(&host.name),
            super::super::render::literal(&project.name),
            super::super::render::literal(&current.status.label_in(&self.locale())),
            self.text("feishu.thread_note")
        ));
        Ok(card)
    }

    async fn bind(
        &self,
        input: &Input,
        hash: &str,
        project: &str,
        session: &str,
        group: &str,
    ) -> Result<Binding> {
        let snapshot = self.fresh(input, hash)?;
        if !self.settings.group(group) {
            return Err(Error::Ui("feishu.error.scope"));
        }
        let selected = Navigator::new(&snapshot.config)
            .parent(project)
            .map_err(|_| Error::Ui("feishu.error.scope"))?;
        let provider = self
            .context
            .runtimes
            .get(&snapshot.config, selected.project)
            .await?;
        let current = provider.read(session).await?;
        engine::runtimes::require_workspace(&current.cwd, selected.project)?;
        let instance = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == selected.project.provider_id)
            .ok_or(Error::Ui("feishu.error.scope"))?;
        let mut b = Binding {
            id: uuid::Uuid::new_v4().to_string(),
            chat: group.into(),
            root: String::new(),
            topic: String::new(),
            project: project.into(),
            instance: instance.id.clone(),
            session: session.into(),
            cwd: current.cwd.clone(),
            endpoint: engine::runtimes::fingerprint(selected.host, instance),
            revision: 1,
            active: true,
            cursor: "{}".into(),
            ready: false,
            status: String::new(),
            state: "ready".into(),
        };
        self.store.reserve(&b).await?;
        let result: Result<()> = async {
            self.fresh(input, hash)?;
            let card = self.status_card(&snapshot, &b, &current)?;
            let root = self
                .api
                .send(
                    &Destination {
                        chat: group.into(),
                        root: None,
                    },
                    &card,
                    &b.id,
                )
                .await?;
            b.root = root.message_id;
            self.store
                .provision(&b.id, Some(&b.root), None, false)
                .await?;
            let mut opened = Card::new(&self.text("feishu.title"));
            opened.markdown(self.text("feishu.thread_note"));
            let reply = self
                .api
                .send(
                    &Destination {
                        chat: group.into(),
                        root: Some(b.root.clone()),
                    },
                    &opened,
                    &uuid::Uuid::new_v4().to_string(),
                )
                .await?;
            if reply.thread_id.is_empty() {
                return Err(Error::Request);
            }
            b.topic = reply.thread_id;
            self.store
                .provision(&b.id, None, Some(&b.topic), false)
                .await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = self.store.provision(&b.id, None, None, true).await;
        }
        result?;
        self.store.binding(&b.id).await
    }
}
