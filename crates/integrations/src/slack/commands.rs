//! Translate explicit Slack actions while retaining owner, message and binding scope.

use super::*;
use crate::slack::render;
use session::provider::Mutation;

impl Service {
    pub(super) async fn action_button(
        &self,
        r: &Request,
        hash: &str,
        label: &str,
        action: Action,
    ) -> Result<slack_morphism::prelude::SlackBlock> {
        let nonce = self.store.action(r, action, hash).await?;
        Ok(render::button(&i18n::text(&self.locale(), label), &nonce))
    }

    pub(super) async fn command(&self, r: &Request, hash: &str) -> Result<()> {
        let snapshot = self.fresh(r, hash)?;
        self.api.channel(&r.channel).await?;
        self.fresh(r, hash)?;
        match &r.command {
            Command::Bindings { offset } => self.binding_list(r, hash, &snapshot, *offset).await?,
            Command::Binding { target } => self.binding_detail(r, hash, &snapshot, target).await?,
            Command::BindingConfirm { target, version } => {
                self.binding_confirmation(r, hash, target, *version, &snapshot)
                    .await?
            }
            Command::BindingChange {
                target,
                version,
                change,
            } => {
                if r.date < self.cutoff.load(Ordering::SeqCst)
                    || !(0..=300).contains(&engine::now().saturating_sub(r.date))
                {
                    return Err(Error::Ui("slack.error.stale"));
                }
                crate::bindings::Manager::new(&self.store, self.binding_scope())
                    .change(target, *version, *change)
                    .await?;
                self.binding_detail(r, hash, &snapshot, target).await?;
            }
            Command::Menu => self.workspace_menu(r, hash, &snapshot, 0).await?,
            Command::Workspaces { offset } => {
                self.workspace_menu(r, hash, &snapshot, *offset).await?
            }
            Command::Projects { host, offset } => {
                self.project_menu(r, hash, &snapshot, host, *offset).await?
            }
            Command::Sessions { workspace, cursor } => {
                self.session_menu(r, hash, &snapshot, workspace, cursor.clone())
                    .await?
            }
            Command::Open { workspace, thread } => {
                self.open(r, hash, workspace, thread).await?;
            }
            Command::New { workspace, title } => {
                if title.trim().is_empty() || title.chars().count() > 120 {
                    return Err(Error::Ui("slack.error.title"));
                }
                let ws = self.workspace(&snapshot, workspace)?;
                let actor = self
                    .context
                    .runtimes
                    .actor(&ws.provider_id, "__create")
                    .await;
                let _guard = actor.lock().await;
                let receipt = self
                    .mutate(
                        r,
                        hash,
                        ws,
                        "__new",
                        "create",
                        Mutation::Create {
                            cwd: ws.cwd.clone(),
                        },
                    )
                    .await?;
                self.mutate(
                    r,
                    hash,
                    ws,
                    &receipt.thread,
                    "title",
                    Mutation::SetTitleIfMissing {
                        thread: receipt.thread.clone(),
                        title: title.trim().into(),
                    },
                )
                .await?;
                self.open(r, hash, workspace, &receipt.thread).await?;
            }
            Command::Input { text } => self.input(r, hash, text).await?,
            Command::Confirm { inbox } => {
                let old = self
                    .store
                    .inbox(inbox)
                    .await?
                    .ok_or(Error::Ui("slack.error.stale"))?;
                if old.state != "awaiting_confirmation" || old.config_hash != hash {
                    return Err(Error::Ui("slack.error.stale"));
                }
                let mut input: Request = serde_json::from_str(&old.request)?;
                if input.binding != r.binding
                    || input.revision != r.revision
                    || input.channel != r.channel
                    || input.root != r.root
                    || input.user != r.user
                {
                    return Err(Error::Ui("slack.error.scope"));
                }
                let Command::Input { text } = input.command.clone() else {
                    return Err(Error::Ui("slack.error.stale"));
                };
                if !self
                    .store
                    .state(inbox, "awaiting_confirmation", "done")
                    .await?
                {
                    return Err(Error::Ui("slack.error.stale"));
                }
                input.id = r.id.clone();
                input.date = r.date;
                self.input(&input, hash, &text).await?;
            }
            Command::Status | Command::History { .. } => {
                if matches!(r.command, Command::History { export: true }) {
                    return Err(Error::Ui("common.export.unsupported"));
                }
                let b = self.bound(r).await?;
                let ws = self.validate(&snapshot, &b)?;
                let provider = self.context.runtimes.get(&snapshot.config, ws).await?;
                let current = provider.read(&b.thread).await?;
                engine::runtimes::require_workspace(&current.cwd, ws)?;
                crate::bindings::remember(&self.store.core, &current).await?;
                if matches!(r.command, Command::Status) {
                    let text = i18n::t!(
                        "common.session.details",
                        locale = &self.locale(),
                        title = session::delivery::short_title(&current.title, 80),
                        state = render::state(&current.status, &self.locale()),
                        workspace = &ws.name,
                    );
                    self.fresh(r, hash)?;
                    self.bound(r).await?;
                    self.publish_card(&b, &text, hash, true).await?;
                    return Ok(());
                }
                let _permit = self
                    .context
                    .runtimes
                    .history_limit(&b.instance)
                    .await
                    .acquire_owned()
                    .await
                    .map_err(|_| Error::Request)?;
                let history = provider
                    .history(&b.thread)
                    .await?
                    .without_transient_errors();
                if !history.all_pages_read {
                    return Err(Error::Ui("slack.error.history"));
                }
                let c =
                    render::conversation(session::presentation::latest(&history, &self.locale()));
                self.fresh(r, hash)?;
                self.bound(r).await?;
                self.store.output(r, &c, hash).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn root_card(&self, b: &Binding, text: &str, hash: &str) -> Result<Content> {
        let r = Request {
            uploads: Vec::new(),
            menu_message: None,
            id: format!("card:{}:{}", b.id, engine::digest(text)),
            channel: b.channel.clone(),
            user: b.owner.clone(),
            root: Some(b.root.clone()),
            binding: Some(b.id.clone()),
            revision: Some(b.revision),
            date: engine::now(),
            command: Command::Status,
        };
        let mut c = render::message(text.into());
        c.document = false;
        for (label, command) in [
            ("slack.status", Command::Status),
            ("slack.history", Command::History { export: false }),
            (
                "bindings.manage",
                Command::Binding {
                    target: crate::bindings::Target::Slack { id: b.id.clone() },
                },
            ),
        ] {
            c.blocks.push(
                self.action_button(&r, hash, label, Action::Run(command))
                    .await?,
            );
        }
        render::compact_actions(&mut c);
        Ok(c)
    }

    async fn open(&self, r: &Request, hash: &str, workspace: &str, thread: &str) -> Result<()> {
        let snapshot = self.fresh(r, hash)?;
        let ws = self.workspace(&snapshot, workspace)?;
        let actor = self.context.runtimes.actor(&ws.provider_id, thread).await;
        let _guard = actor.lock().await;
        let provider = self.context.runtimes.get(&snapshot.config, ws).await?;
        let current = provider.read(thread).await?;
        engine::runtimes::require_workspace(&current.cwd, ws)?;
        crate::bindings::remember(&self.store.core, &current).await?;
        self.fresh(r, hash)?;
        self.api.channel(&r.channel).await?;
        let existing = self
            .store
            .existing(&r.channel, &ws.provider_id, thread)
            .await?;
        // Existing initialized threads already have a baseline. Reopening only refreshes
        // controls; fetching all history again would add latency without displaying it.
        let history = if existing
            .as_ref()
            .is_none_or(|binding| !binding.history_ready)
        {
            let _permit = self
                .context
                .runtimes
                .history_limit(&ws.provider_id)
                .await
                .acquire_owned()
                .await
                .map_err(|_| Error::Request)?;
            let history = crate::questions::history(provider.as_ref(), thread, &self.locale())
                .await?
                .without_transient_errors();
            if !history.all_pages_read {
                return Err(Error::Ui("slack.error.history"));
            }
            Some(history)
        } else {
            None
        };
        self.fresh(r, hash)?;
        let b = if let Some(mut b) = existing {
            self.validate(&snapshot, &b)?;
            if !b.active {
                self.store.reactivate(&b).await?;
                b.active = true;
                b.revision += 1;
            }
            b
        } else {
            let p = snapshot
                .config
                .providers
                .iter()
                .find(|p| p.id == ws.provider_id)
                .ok_or(Error::Ui("slack.error.scope"))?;
            let h = snapshot
                .config
                .hosts
                .iter()
                .find(|h| h.id == p.host_id)
                .ok_or(Error::Ui("slack.error.scope"))?;
            let mut b = Binding {
                id: uuid::Uuid::new_v4().to_string(),
                app: self.settings.app_id.clone(),
                team: self.settings.team_id.clone(),
                bot: self.bot.clone(),
                channel: r.channel.clone(),
                root: String::new(),
                owner: r.user.clone(),
                workspace: ws.id.clone(),
                instance: ws.provider_id.clone(),
                thread: thread.into(),
                cwd: current.cwd.clone(),
                endpoint: engine::runtimes::fingerprint(h, p),
                active: true,
                revision: 1,
                observed: "{}".into(),
                card: String::new(),
                history_ready: false,
            };
            self.store.reserve(&b).await?;
            self.fresh(r, hash)?;
            let text = i18n::t!(
                "common.session.details",
                locale = &self.locale(),
                title = session::delivery::short_title(&current.title, 80),
                state = render::state(&current.status, &self.locale()),
                workspace = &ws.name,
            );
            b.root = self
                .api
                .post(&r.channel, None, &render::message(text))
                .await?;
            // Persist the exact created root even if scope changed while the
            // request was in flight; later delivery rechecks authorization.
            self.store.bind(&b).await?;
            b
        };
        self.fresh(r, hash)?;
        if let Some(history) = history {
            let observed =
                super::history::capture(&b, &history, &self.store.namespace, &self.locale())?;
            self.store
                .publish(&b, &observed.content, &observed.cursor, hash)
                .await?;
        }
        // Explicit opening refreshes expired card controls without replaying history.
        self.store.card(&b, "").await?;
        let mut reply = render::message(i18n::t!(
            "slack.opened",
            locale = &self.locale(),
            channel = &b.channel,
            root = &b.root
        ));
        reply.blocks = vec![
            slack_morphism::prelude::SlackSectionBlock::new()
                .with_text(slack_morphism::md!(
                    "<https://app.slack.com/client/{}/{}/thread/{}-{}|{}>",
                    self.settings.team_id,
                    b.channel,
                    b.channel,
                    b.root,
                    i18n::text(&self.locale(), "slack.open")
                ))
                .into(),
        ];
        reply.blocks.insert(
            0,
            render::header(&i18n::text(&self.locale(), "slack.session.opened")),
        );
        reply.blocks.insert(1, render::context(&ws.name));
        reply.blocks.push(
            self.action_button(
                r,
                hash,
                "slack.sessions.back",
                Action::Run(Command::Sessions {
                    workspace: workspace.into(),
                    cursor: None,
                }),
            )
            .await?,
        );
        let host = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .ok_or(Error::Ui("slack.error.scope"))?
            .host_id
            .clone();
        reply.blocks.push(
            self.action_button(
                r,
                hash,
                "slack.menu.back",
                Action::Run(Command::Projects { host, offset: 0 }),
            )
            .await?,
        );
        self.menu_output(r, reply, hash).await?;
        Ok(())
    }
}
