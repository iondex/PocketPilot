use super::{Result, Service, Target, error};
use crate::discord::Api;
use crate::discord::Command;
use crate::discord::Input;
use engine::reload::Snapshot;
use engine::runtimes;
use session::RuntimeStatus;
use session::provider::Mutation;

impl Service {
    pub(super) async fn create_session(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<String> {
        let Command::New {
            workspace,
            project,
            text,
            confirmed,
        } = &input.command
        else {
            return Err("discord.error.create_confirmation".into());
        };
        if !confirmed || text.trim().is_empty() {
            return Err("discord.error.create_confirmation".into());
        }
        let ws = self.project(&snapshot.config, workspace, project)?;
        let actor = self.runtimes.actor(&ws.provider_id, "__create").await;
        let _guard = actor.lock().await;
        let receipt = self
            .mutate(
                api,
                input,
                snapshot,
                "create",
                Target {
                    workspace: ws,
                    thread: "__new",
                    binding: None,
                },
                Mutation::Create {
                    cwd: ws.cwd.clone(),
                },
            )
            .await?;
        let actor = self.runtimes.actor(&ws.provider_id, &receipt.thread).await;
        {
            let _guard = actor.lock().await;
            self.mutate(
                api,
                input,
                snapshot,
                "first-turn",
                Target {
                    workspace: ws,
                    thread: &receipt.thread,
                    binding: None,
                },
                Mutation::Start {
                    thread: receipt.thread.clone(),
                    text: text.clone(),
                    client_id: format!("pp:discord:{}:{}", input.bot, input.id),
                },
            )
            .await?;
        }
        Ok(receipt.thread)
    }

    pub(super) async fn command(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<String> {
        let locale = self.locale(input);
        match &input.command {
            Command::Unbind
            | Command::Bindings
            | Command::Workspaces
            | Command::Projects { .. }
            | Command::Sessions { .. } => Err("discord.ui.use_menu".into()),
            Command::Open {
                workspace,
                project,
                thread,
            } => {
                let ws = self.project(&snapshot.config, workspace, project)?;
                self.open(api, input, snapshot, &ws.id, thread).await
            }
            Command::New {
                workspace, project, ..
            } => {
                let thread = self.create_session(api, input, snapshot).await?;
                let ws = self.project(&snapshot.config, workspace, project)?;
                self.open(api, input, snapshot, &ws.id, &thread).await
            }
            Command::Ask { text } => {
                if text.trim().is_empty() {
                    return Err("discord.error.empty_input".into());
                }
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let actor = self.runtimes.actor(&b.instance, &b.thread).await;
                let _guard = actor.lock().await;
                let provider = self
                    .runtimes
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let mut current = provider.read(&b.thread).await.map_err(error)?;
                runtimes::require_workspace(&current.cwd, ws).map_err(error)?;
                if let Some(mutation) = crate::questions::prepare(
                    provider.as_ref(),
                    &b.thread,
                    text,
                    !input.uploads.is_empty(),
                )
                .await
                .map_err(error)?
                {
                    self.mutate(
                        api,
                        input,
                        snapshot,
                        "answer",
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        mutation,
                    )
                    .await?;
                    return Ok(i18n::text(&locale, "common.question.accepted"));
                }
                if current.status == RuntimeStatus::NotLoaded {
                    self.mutate(
                        api,
                        input,
                        snapshot,
                        "resume",
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        Mutation::Resume {
                            thread: b.thread.clone(),
                        },
                    )
                    .await?;
                    current = provider.read(&b.thread).await.map_err(error)?;
                }
                let client_id = format!("pp:discord:{}:{}", input.bot, input.id);
                let mutation = match current.status {
                    RuntimeStatus::Idle => Mutation::Start {
                        thread: b.thread.clone(),
                        text: text.clone(),
                        client_id,
                    },
                    RuntimeStatus::Active { .. } => Mutation::Steer {
                        thread: b.thread.clone(),
                        expected_turn: match current.active_turn {
                            Some(id) => id,
                            None => provider
                                .active_turn(&b.thread)
                                .await
                                .map_err(error)?
                                .ok_or("discord.error.turn_pending")?,
                        },
                        text: text.clone(),
                        client_id,
                    },
                    _ => return Err("discord.error.state".into()),
                };
                if !provider.capabilities().control_verified {
                    return Err("discord.error.read_only".into());
                }
                if input.uploads.len() > session::files::MAX_FILES {
                    return Err("common.file.failed".into());
                }
                let mut downloaded = Vec::new();
                for file in &input.uploads {
                    downloaded.push(api.download(file).await.map_err(error)?);
                }
                let instance = snapshot
                    .config
                    .providers
                    .iter()
                    .find(|p| p.id == b.instance)
                    .ok_or("discord.error.provider_removed")?;
                let (uploads, files) = engine::uploads::prepare(
                    instance,
                    &format!("discord:{}:{}", input.bot, input.id),
                    &b.thread,
                    downloaded,
                )
                .map_err(error)?;
                for (index, mutation) in uploads.into_iter().enumerate() {
                    self.mutate(
                        api,
                        input,
                        snapshot,
                        &format!("upload:{index}"),
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        mutation,
                    )
                    .await?;
                }
                self.mutate(
                    api,
                    input,
                    snapshot,
                    "input",
                    Target {
                        workspace: ws,
                        thread: &b.thread,
                        binding: Some(&b),
                    },
                    engine::uploads::attach(mutation, files),
                )
                .await?;
                Ok(i18n::text(&locale, "discord.input.accepted"))
            }
            Command::Rename { title } => {
                let b = self.bound(input).await?;
                self.validate_binding(&snapshot.config, &b)?;
                self.fresh(api, input, snapshot, true).await?;
                api.rename(input.channel, title).await.map_err(error)?;
                Ok(i18n::text(&locale, "discord.renamed"))
            }
            Command::Status | Command::Recent => {
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let provider = self
                    .runtimes
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let current = provider.read(&b.thread).await.map_err(error)?;
                runtimes::require_workspace(&current.cwd, ws).map_err(error)?;
                if matches!(input.command, Command::Recent) {
                    let history = self.history(provider.as_ref(), ws, &b.thread).await?;
                    let text = crate::discord::render::Conversation::latest(&history, &locale)
                        .map_err(|_| i18n::Message::new("discord.history.too_large"))?;
                    self.store
                        .discord_publish(
                            &b,
                            &format!("recent:{}", input.id),
                            &text,
                            &super::hashes(&history),
                        )
                        .await
                        .map_err(error)?;
                    Ok(i18n::text(&locale, "discord.recent.sent"))
                } else {
                    Ok(i18n::t!(
                        "common.session.details",
                        locale = &locale,
                        title = current.title,
                        state = crate::discord::ui::status(&current.status, &locale),
                        workspace = &ws.name,
                    ))
                }
            }
        }
    }
}
