use super::{Action, Error, Service, topic_title};
use crate::api::Input;
use crate::ledger::Outbound;
use engine::connections;
use engine::now;
use engine::storage::StoreError;
use std::time::Duration;
use tokio::time::Instant;

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
        if crate::access(input.user, input.chat, true, false) == crate::Access::Denied {
            return Ok(());
        }
        let paired = self
            .store
            .pairing()
            .is_paired(self.bot, input.user, input.chat)
            .await
            .map_err(|_| StoreError::Database)?;
        if !paired {
            let starting = input.text.split_whitespace().next() == Some("/start");
            let mut attempts = self.pair_attempts.lock().await;
            let key = (input.user, starting);
            if attempts
                .get(&key)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
            {
                return Ok(());
            }
            attempts.retain(|_, at| at.elapsed() < Duration::from_secs(600));
            if attempts.len() >= 4096 {
                return Ok(());
            }
            attempts.insert(key, Instant::now());
            drop(attempts);
            let mut body = Outbound::text(
                "请先发送 /start 申请配对，并在运行 PocketPilot 的机器上用 CLI 批准。",
            );
            body.require_pairing = false;
            if starting {
                let code = self
                    .store
                    .pairing()
                    .request_pairing(self.bot, input.user, input.chat, true, now())
                    .await
                    .map_err(|_| StoreError::Database)?;
                body.text =
                    rust_i18n::t!("pair_code", locale = crate::i18n::lang(input), code = code)
                        .to_string();
                body.expires_at = Some(now() + 600);
            }
            body.text = crate::i18n::tr(input, &body.text);
            return self
                .store
                .enqueue(self.bot, input.chat, input.topic, &input.event_key, &body)
                .await
                .map_err(Into::into);
        }
        let snapshot = self.config.snapshot();
        let mut scoped = input.clone();
        scoped.config_hash = Some(snapshot.hash.clone());
        let saved_locale = self.store.locale(self.bot, input.user).await?;
        let locale = saved_locale
            .as_deref()
            .or(input.locale.as_deref())
            .unwrap_or(&self.default_locale);
        scoped.locale = Some(crate::i18n::normalize(locale).into());
        if saved_locale.is_none() {
            self.store
                .set_locale(
                    self.bot,
                    input.user,
                    scoped.locale.as_deref().unwrap_or("zh-CN"),
                )
                .await?;
        }

        let off_topic = scoped.topic == 0 && scoped.callback.is_none();
        if off_topic {
            scoped = self.management_input(&scoped, &snapshot).await?;
        }
        let input = &scoped;
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
                Action::Host { id } => self.host_workspaces(input, &snapshot, &id).await,
                Action::NewPrompt { .. }
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
                    self.submit(input, &snapshot, &text, steer_only).await
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
                    | "/cleartopics"
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
                    | "/cleartopics"
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
                        .respond(
                            input,
                            &input.event_key,
                            Outbound::text("🌐 中文：/language zh-CN\nEnglish: /language en"),
                        )
                        .await;
                }
                self.store.set_locale(self.bot, input.user, choice).await?;
                let mut localized = input.clone();
                localized.locale = Some(crate::i18n::normalize(choice).into());
                self.workspaces(&localized, &snapshot).await
            }
            "/topics" => self.topics(input, &snapshot, 0).await,
            "/cleartopics" => self.confirm_clear_topics(input, &snapshot).await,
            "/add" => self.add_workspace(input, &snapshot, args.trim()).await,
            "/remove" => self.remove_workspace(input, &snapshot, args.trim()).await,
            "/start" | "/workspaces" => self.workspaces(input, &snapshot).await,
            "/refresh" | "/back" => self.select_navigation(input, &snapshot).await,
            "/help" => {
                self.respond(
                    input,
                    &input.event_key,
                    Outbound::text(crate::i18n::tr(input, "help")),
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
                    return self.workspaces(input, &snapshot).await;
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
                let provider = self.connections.get(&snapshot.config, workspace).await?;
                let current = provider.read(&binding.thread_id).await?;
                connections::require_workspace(&current.cwd, workspace)?;
                tracing::info!(thread_id=%current.reference.thread,workspace_id=%workspace.id,status=?current.status,"session details requested");
                let text = if command == "/history" {
                    let text = session::delivery::agent_history(
                        &self
                            .history(provider.as_ref(), workspace, &binding.thread_id, &snapshot)
                            .await?,
                    );
                    if text.is_empty() {
                        "暂无回复。".into()
                    } else {
                        text
                    }
                } else {
                    let host = snapshot
                        .config
                        .providers
                        .iter()
                        .find(|p| p.id == workspace.provider_id)
                        .and_then(|p| snapshot.config.hosts.iter().find(|h| h.id == p.host_id))
                        .ok_or(connections::Error::Selection)?;
                    let state = match current.status {
                        session::RuntimeStatus::NotLoaded => "未加载",
                        session::RuntimeStatus::Idle => "空闲",
                        session::RuntimeStatus::Active {
                            waiting_on_approval: true,
                            ..
                        } => "等待审批",
                        session::RuntimeStatus::Active {
                            waiting_on_user_input: true,
                            ..
                        } => "等待回答",
                        session::RuntimeStatus::Active { .. } => "运行中",
                        session::RuntimeStatus::SystemError => "服务异常",
                        session::RuntimeStatus::Unknown => "状态未知",
                    };
                    rust_i18n::t!(
                        "details",
                        locale = crate::i18n::lang(input),
                        emoji = crate::presentation::emoji(&current.status),
                        title = &current.title,
                        state = crate::i18n::tr(input, state),
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
            "/unbind" => {
                self.bound(input).await?;
                self.store.unbind(self.bot, input.chat, input.topic).await?;
                self.respond(
                    input,
                    &input.event_key,
                    Outbound::text(
                        "已解除此 topic 的映射，会话继续运行。此 topic 只能重新连接原会话。",
                    ),
                )
                .await
            }
            "/rename" => {
                if args.trim().is_empty() {
                    return self
                        .respond(
                            input,
                            &input.event_key,
                            Outbound::text("请使用 /rename 新名称。"),
                        )
                        .await;
                }
                let binding = self.bound(input).await?;
                let workspace = self.binding_workspace(&snapshot.config, &binding)?;
                let title = session::delivery::short_title(args, 40);
                let provider = self.connections.get(&snapshot.config, workspace).await?;
                let native = provider.read(&binding.thread_id).await?;
                connections::require_workspace(&native.cwd, workspace)?;
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
                    rust_i18n::t!("renamed", locale = crate::i18n::lang(input), title = &title)
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
                self.respond(
                    input,
                    &input.event_key,
                    Outbound::text(
                        "未知命令。使用 /help；如需把斜杠文本发送给 agent，使用 /ask 文本。",
                    ),
                )
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
