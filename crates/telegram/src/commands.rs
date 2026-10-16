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
                body.text = format!(
                    "配对码（10 分钟有效）\n{code}\n\n请在运行 PocketPilot 的机器上执行：\npocketpilot pair approve {code}\n\n批准后再次发送 /start。"
                );
                body.expires_at = Some(now() + 600);
            }
            return self
                .store
                .enqueue(self.bot, input.chat, input.topic, &input.event_key, &body)
                .await
                .map_err(Into::into);
        }
        let snapshot = self.config.snapshot();
        let mut scoped = input.clone();
        scoped.config_hash = Some(snapshot.hash.clone());
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
                Action::NewPrompt { .. } => self.workspaces(input, &snapshot).await,
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
        let input = if matches!(
            command,
            "/start"
                | "/workspaces"
                | "/sessions"
                | "/open"
                | "/new"
                | "/help"
                | "/refresh"
                | "/back"
                | "/add"
                | "/remove"
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
                    | "/workspaces"
                    | "/sessions"
                    | "/open"
                    | "/new"
                    | "/help"
                    | "/refresh"
                    | "/back"
                    | "/add"
                    | "/remove"
            )
        {
            return self.default_message(input, &snapshot).await;
        }
        match command {
            "/add"=>self.add_workspace(input,&snapshot,args.trim()).await,
            "/remove"=>self.remove_workspace(input,&snapshot,args.trim()).await,
            "/start"|"/workspaces"=>self.workspaces(input,&snapshot).await,
            "/refresh"|"/back"=>self.select_navigation(input,&snapshot).await,
            "/help"=>self.respond(input,&input.event_key,Outbound::text("回复序号选择主机、工作区和会话。\n0 返回 · /start 首页 · /refresh 刷新\n/new 工作区 任务 新建会话\n\n在会话 topic 直接发送任务。\n/status 状态 · /history 历史\n/rename 名称 · /unbind 解除关联")).await,
            "/sessions"=>self.sessions(input,&snapshot,args.trim(),None).await,
            "/open"=>{
                let (workspace,thread)=args.trim().split_once(char::is_whitespace).ok_or(Error::Unbound)?;
                self.open(input,&snapshot,workspace,thread.trim()).await
            }
            "/new"=>{
                let (workspace,text)=args.trim().split_once(char::is_whitespace).ok_or(Error::Unbound)?;
                let workspace=self.workspace(&snapshot.config,workspace)?;
                if text.trim().is_empty() { return Err(Error::Stale); }
                self.confirm_creation(input,&snapshot,&workspace.id,text.trim()).await
            }
            "/status"|"/info"|"/history"=>{
                let binding=self.bound(input).await?;
                let workspace=self.binding_workspace(&snapshot.config,&binding)?;
                let provider=self.connections.get(&snapshot.config,workspace).await?;
                let current=provider.read(&binding.thread_id).await?;
                connections::require_workspace(&current.cwd,workspace)?;
                tracing::info!(thread_id=%current.reference.thread,workspace_id=%workspace.id,status=?current.status,"session details requested");
                let text=if command=="/history" {
                    let text=session::delivery::agent_history(&self.history(provider.as_ref(),workspace,&binding.thread_id,&snapshot).await?);
                    if text.is_empty(){"暂无回复。".into()}else{text}
                }else {format!("{}\n{}",session::delivery::short_title(&current.title,40),current.status.label())};
                self.respond(input,&input.event_key,Outbound::text(text)).await
            }
            "/unbind"=>{
                self.bound(input).await?;
                self.store.unbind(self.bot,input.chat,input.topic).await?;
                self.respond(input,&input.event_key,Outbound::text("已解除此 topic 的映射，会话继续运行。此 topic 只能重新连接原会话。" )).await
            }
            "/rename"=>{
                let binding=self.bound(input).await?;
                let workspace=self.binding_workspace(&snapshot.config,&binding)?;
                let title=session::delivery::short_title(args,40);
                self.store.rename(self.bot,input.chat,input.topic,&title).await?;
                let mut body=Outbound::text(format!("显示名已设为 {title}"));
                body.topic_title=Some(topic_title(workspace,&title,&binding.thread_id));
                self.respond(input,&input.event_key,body).await
            }
            "/ask"|"/steer"=>{
                if self.delayed(input) {self.draft(input,&snapshot,args.trim(),command=="/steer").await}
                else {self.submit(input,&snapshot,args.trim(),command=="/steer").await}
            },
            text if text.starts_with('/')=>self.respond(input,&input.event_key,Outbound::text("未知命令。使用 /help；如需把斜杠文本发送给 agent，使用 /ask 文本。")).await,
            _=>{
                if self.delayed(input) {self.draft(input,&snapshot,input.text.trim(),false).await}
                else {self.submit(input,&snapshot,input.text.trim(),false).await}
            },
        }
    }
}
