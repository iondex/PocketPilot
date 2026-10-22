//! Handle explicit Telegram commands through shared navigation and authorization rules.

use super::{Action, Error, Service, topic_title};
use crate::telegram::api::Input;
use crate::telegram::ledger::Outbound;
use engine::now;
use engine::runtimes;

impl Service {
    pub(super) async fn handle(&self, input: &Input) -> Result<(), Error> {
        let mut initial = input.clone();
        if self.default_locale != "auto" || initial.locale.is_none() {
            initial.locale = Some(if self.default_locale == "auto" {
                "zh-CN".into()
            } else {
                self.default_locale.clone()
            });
        }
        let input = &initial;
        if !self.store.owner.authorizes(input.user, input.chat, true) {
            return Ok(());
        }
        let snapshot = self.config.snapshot();
        let mut scoped = input.clone();
        scoped.config_hash = Some(snapshot.hash.clone());
        let saved_locale = self.store.locale(self.bot, input.user).await?;
        let locale = saved_locale
            .as_deref()
            .or(input.locale.as_deref())
            .unwrap_or(&self.default_locale);
        scoped.locale = Some(crate::telegram::i18n::normalize(locale).into());
        if saved_locale.is_none() {
            self.store
                .set_locale(
                    self.bot,
                    input.user,
                    scoped.locale.as_deref().unwrap_or("zh-CN"),
                )
                .await?;
        }

        if !scoped.uploads.is_empty() && scoped.text.trim().is_empty() {
            scoped.text = ::i18n::text(crate::telegram::i18n::lang(&scoped), "common.file.request");
        }
        if !input.attachments.is_empty() && input.uploads.is_empty() {
            self.respond(
                &scoped,
                &format!("attachments:{}", input.event_key),
                Outbound::ui("common.file.unavailable"),
            )
            .await?;
            return Ok(());
        }
        let off_topic = scoped.topic == 0 && scoped.callback.is_none();
        if off_topic {
            scoped = self.management_input(&scoped, &snapshot).await?;
        }
        let input = &scoped;
        if input
            .callback
            .as_deref()
            .is_some_and(|data| data.starts_with("nav:"))
        {
            return self.select_navigation_button(input, &snapshot).await;
        }
        if input.callback.is_some() {
            if input.requires_confirmation {
                let mut routed = input.clone();
                routed.callback = None;
                routed.callback_id = None;
                let routed = self.management_input(&routed, &snapshot).await?;
                return self.workspaces(&routed, &snapshot).await;
            }
            let body = self
                .store
                .consume_callback(self.bot, input, &snapshot.hash, now())
                .await?;
            let Some(body) = body else {
                let mut routed = input.clone();
                routed.callback = None;
                routed.callback_id = None;
                let routed = self.management_input(&routed, &snapshot).await?;
                return self.workspaces(&routed, &snapshot).await;
            };
            let action: Action = serde_json::from_str(&body).map_err(|_| Error::Expired)?;
            return match action {
                Action::Home => self.workspaces(input, &snapshot).await,
                Action::HomePage { offset } => {
                    self.host_page(input, &snapshot, offset, false).await
                }
                Action::Host { id } => self.host_workspaces(input, &snapshot, &id).await,
                Action::HostPage { id, offset } => {
                    self.host_workspace_page(input, &snapshot, &id, offset)
                        .await
                }
                Action::NewSession
                | Action::NewHostsPage { .. }
                | Action::NewHost { .. }
                | Action::NewHostPage { .. }
                | Action::NewPrompt { .. }
                | Action::Bindings { .. }
                | Action::Binding { .. }
                | Action::BindingConfirm { .. }
                | Action::BindingChange { .. }
                | Action::Topics { .. }
                | Action::ConfirmTopicRemoval { .. }
                | Action::ConfirmClearTopics
                | Action::ClearTopics { .. }
                | Action::RemoveTopic { .. } => self.workspaces(input, &snapshot).await,
                Action::Workspace { id, cursor } => {
                    self.sessions(input, &snapshot, &id, cursor).await
                }
                Action::Open { workspace, thread } => {
                    self.open(input, &snapshot, &workspace, &thread).await
                }
                Action::Create { workspace, text } => {
                    self.create(input, &snapshot, &workspace, &text).await
                }
                Action::SubmitDraft {
                    uploads,
                    text,
                    steer_only,
                    instance,
                    thread,
                    revision,
                } => {
                    let binding = self.bound(input).await?;
                    if binding.instance_id != instance
                        || binding.thread_id != thread
                        || binding.revision != revision
                    {
                        return Err(Error::Expired);
                    }
                    let mut confirmed = input.clone();
                    confirmed.uploads = uploads;
                    self.submit(&confirmed, &snapshot, &text, steer_only).await
                }
            };
        }
        let (command, args) = input
            .text
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((input.text.trim(), ""));
        let command = command.split('@').next().unwrap_or(command);
        let management;
        let input = if !off_topic
            && matches!(
                command,
                "/start"
                    | "/topics"
                    | "/bindings"
                    | "/workspaces"
                    | "/sessions"
                    | "/open"
                    | "/new"
                    | "/refresh"
                    | "/back"
                    | "/add"
                    | "/remove"
                    | "/language"
                    | "/lang"
            ) {
            management = self.management_input(input, &snapshot).await?;
            &management
        } else {
            input
        };
        if self
            .store
            .binding(self.bot, input.chat, input.topic)
            .await?
            .is_none()
            && !matches!(
                command,
                "/start"
                    | "/topics"
                    | "/bindings"
                    | "/workspaces"
                    | "/sessions"
                    | "/open"
                    | "/new"
                    | "/help"
                    | "/refresh"
                    | "/back"
                    | "/add"
                    | "/remove"
                    | "/language"
                    | "/lang"
            )
        {
            return self.default_message(input, &snapshot).await;
        }
        match command {
            "/language" | "/lang" => {
                let choice = args.trim();
                if !matches!(choice, "en" | "zh" | "zh-CN" | "zh-cn") {
                    return self
                        .respond(input, &input.event_key, Outbound::ui("tg.language.usage"))
                        .await;
                }
                self.store.set_locale(self.bot, input.user, choice).await?;
                let mut localized = input.clone();
                localized.locale = Some(crate::telegram::i18n::normalize(choice).into());
                self.workspaces(&localized, &snapshot).await
            }
            "/bindings" => self.binding_list(input, &snapshot, 0).await,
            "/topics" => {
                if args.trim() == "clear" {
                    self.confirm_clear_topics(input, &snapshot).await
                } else {
                    self.topics(input, &snapshot, 0).await
                }
            }
            "/add" => self.add_workspace(input, &snapshot, args.trim()).await,
            "/remove" => self.remove_workspace(input, &snapshot, args.trim()).await,
            "/start" | "/workspaces" => self.workspaces(input, &snapshot).await,
            "/refresh" | "/back" => self.select_navigation(input, &snapshot).await,
            "/help" => {
                let mut target = input.clone();
                let key = if self
                    .store
                    .binding(self.bot, input.chat, input.topic)
                    .await?
                    .is_some()
                {
                    "tg.help.session"
                } else {
                    target = self.management_input(input, &snapshot).await?;
                    "tg.help.management"
                };
                self.respond(
                    &target,
                    &input.event_key,
                    Outbound::text(crate::telegram::i18n::tr(input, key)),
                )
                .await
            }
            "/sessions" => self.sessions(input, &snapshot, args.trim(), None).await,
            "/open" => {
                let (workspace, thread) = args
                    .trim()
                    .split_once(char::is_whitespace)
                    .ok_or(Error::Unbound)?;
                self.open(input, &snapshot, workspace, thread.trim()).await
            }
            "/new" => {
                if args.trim().is_empty() {
                    return self.new_hosts(input, &snapshot).await;
                }
                let (workspace, text) = args
                    .trim()
                    .split_once(char::is_whitespace)
                    .ok_or(Error::Unbound)?;
                let workspace = self.workspace(&snapshot.config, workspace)?;
                if text.trim().is_empty() {
                    return Err(Error::Stale);
                }
                self.confirm_creation(input, &snapshot, &workspace.id, text.trim())
                    .await
            }
            "/status" | "/info" | "/history" => {
                let binding = self.bound(input).await?;
                let workspace = self.binding_workspace(&snapshot.config, &binding)?;
                let provider = self.runtimes.get(&snapshot.config, workspace).await?;
                let current = provider.read(&binding.thread_id).await?;
                runtimes::require_workspace(&current.cwd, workspace)?;
                tracing::info!(thread_id=%current.reference.thread,workspace_id=%workspace.id,status=?current.status,"session details requested");
                let text = if command == "/history" {
                    let history = self
                        .history(provider.as_ref(), workspace, &binding.thread_id, &snapshot)
                        .await?;
                    if args.trim() == "export" {
                        return self
                            .respond(
                                input,
                                &input.event_key,
                                Outbound::ui("common.export.unsupported"),
                            )
                            .await;
                    } else if args.trim() == "errors" {
                        self.error_history(input, &history)
                    } else {
                        self.history_summary(input, &history)
                    }
                } else {
                    let host = snapshot
                        .config
                        .providers
                        .iter()
                        .find(|p| p.id == workspace.provider_id)
                        .and_then(|p| snapshot.config.hosts.iter().find(|h| h.id == p.host_id))
                        .ok_or(runtimes::Error::Selection)?;
                    let state = match current.status {
                        session::RuntimeStatus::NotLoaded => "state.not_loaded",
                        session::RuntimeStatus::Idle => "state.idle",
                        session::RuntimeStatus::Active {
                            waiting_on_approval: true,
                            ..
                        } => "state.waiting_approval",
                        session::RuntimeStatus::Active {
                            waiting_on_user_input: true,
                            ..
                        } => "state.waiting_input",
                        session::RuntimeStatus::Active { .. } => "state.active",
                        session::RuntimeStatus::SystemError => "state.error",
                        session::RuntimeStatus::Unknown => "state.unknown",
                    };
                    ::i18n::t!(
                        "tg.session.details",
                        locale = crate::telegram::i18n::lang(input),
                        emoji = crate::telegram::presentation::emoji(&current.status),
                        title = &current.title,
                        state = crate::telegram::i18n::tr(input, state),
                        host = &host.name,
                        workspace = &workspace.name,
                        cwd = &current.cwd,
                        thread = &current.reference.thread,
                        instance = &workspace.provider_id,
                        turn = current.active_turn.as_deref().unwrap_or("—")
                    )
                    .to_string()
                };
                let body = if command == "/history" {
                    Outbound::agent(text)
                } else {
                    Outbound::text(text)
                };
                self.respond(input, &input.event_key, body).await
            }
            "/rename" => {
                if args.trim().is_empty() {
                    return self
                        .respond(input, &input.event_key, Outbound::ui("tg.rename.usage"))
                        .await;
                }
                let binding = self.bound(input).await?;
                let workspace = self.binding_workspace(&snapshot.config, &binding)?;
                let title = session::delivery::short_title(args, 40);
                let provider = self.runtimes.get(&snapshot.config, workspace).await?;
                let native = provider.read(&binding.thread_id).await?;
                runtimes::require_workspace(&native.cwd, workspace)?;
                if native.native_title.is_none() && provider.capabilities().control_verified {
                    self.mutate(
                        input,
                        &snapshot,
                        workspace,
                        "set-title",
                        &binding.thread_id,
                        session::provider::Mutation::SetTitleIfMissing {
                            thread: binding.thread_id.clone(),
                            title: title.clone(),
                        },
                    )
                    .await?;
                }
                self.store
                    .rename(self.bot, input.chat, input.topic, &title)
                    .await?;
                let mut body = Outbound::text(
                    ::i18n::t!(
                        "tg.rename.done",
                        locale = crate::telegram::i18n::lang(input),
                        title = &title
                    )
                    .to_string(),
                );
                body.topic_title = Some(topic_title(workspace, &title, &binding.thread_id));
                self.respond(input, &input.event_key, body).await
            }
            "/ask" | "/steer" => {
                if self.delayed(input) {
                    self.draft(input, &snapshot, args.trim(), command == "/steer")
                        .await
                } else {
                    self.submit(input, &snapshot, args.trim(), command == "/steer")
                        .await
                }
            }
            text if text.starts_with('/') => {
                self.respond(input, &input.event_key, Outbound::ui("tg.command.unknown"))
                    .await
            }
            _ => {
                if self.delayed(input) {
                    self.draft(input, &snapshot, input.text.trim(), false).await
                } else {
                    self.submit(input, &snapshot, input.text.trim(), false)
                        .await
                }
            }
        }
    }
}
