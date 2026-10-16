use super::{Result, Service, Target, error};
use crate::Api;
use crate::Command;
use crate::Input;
use engine::connections;
use engine::reload::Snapshot;
use session::RuntimeStatus;
use session::provider::Mutation;

impl Service {
    pub(super) async fn command(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<String> {
        let locale = self.locale(input);
        match &input.command {
            Command::Workspaces => Ok(snapshot
                .config
                .workspaces
                .iter()
                .map(|w| format!("{} · {}", w.short_name, w.name))
                .collect::<Vec<_>>()
                .join("\n")),
            Command::Sessions { workspace, cursor } => {
                let ws = self.workspace(&snapshot.config, workspace)?;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let page =
                    connections::list_picker(&self.store, provider.as_ref(), ws, cursor.clone(), 8)
                        .await
                        .map_err(error)?;
                let mut text = page
                    .data
                    .iter()
                    .map(|s| {
                        format!(
                            "{} · {}\n{}",
                            s.status.label_in(&locale),
                            session::delivery::short_title(&s.title, 45),
                            s.reference.thread
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                text.push_str(&i18n::t!(
                    "discord.sessions.open",
                    locale = &locale,
                    workspace = &ws.short_name
                ));
                if let Some(cursor) = page.next_cursor {
                    text.push_str(&i18n::t!(
                        "discord.sessions.next",
                        locale = &locale,
                        cursor = cursor
                    ));
                }
                Ok(text)
            }
            Command::Open { workspace, thread } => {
                self.open(api, input, snapshot, workspace, thread).await
            }
            Command::New {
                workspace,
                text,
                confirmed,
            } => {
                if !confirmed || text.trim().is_empty() {
                    return Err("discord.error.create_confirmation".into());
                }
                let ws = self.workspace(&snapshot.config, workspace)?;
                let actor = self.connections.actor(&ws.provider_id, "__create").await;
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
                let actor = self
                    .connections
                    .actor(&ws.provider_id, &receipt.thread)
                    .await;
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
                self.open(api, input, snapshot, workspace, &receipt.thread)
                    .await
            }
            Command::Ask { text } => {
                if text.trim().is_empty() {
                    return Err("discord.error.empty_input".into());
                }
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let actor = self.connections.actor(&b.instance, &b.thread).await;
                let _guard = actor.lock().await;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let mut current = provider.read(&b.thread).await.map_err(error)?;
                connections::require_workspace(&current.cwd, ws).map_err(error)?;
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
                let receipt = self
                    .mutate(
                        api,
                        input,
                        snapshot,
                        "input",
                        Target {
                            workspace: ws,
                            thread: &b.thread,
                            binding: Some(&b),
                        },
                        mutation,
                    )
                    .await?;
                Ok(i18n::t!(
                    "discord.input.accepted",
                    locale = &locale,
                    turn = receipt.turn.unwrap_or_default()
                ))
            }
            Command::Unbind => {
                let b = self.bound(input).await?;
                self.fresh(api, input, snapshot, true).await?;
                self.store.discord_unbind(&b).await.map_err(error)?;
                Ok(i18n::text(&locale, "discord.unbound"))
            }
            Command::Rename { title } => {
                let b = self.bound(input).await?;
                self.validate_binding(&snapshot.config, &b)?;
                self.fresh(api, input, snapshot, true).await?;
                api.rename(input.channel, title, &b.thread)
                    .await
                    .map_err(error)?;
                Ok(i18n::text(&locale, "discord.renamed"))
            }
            Command::Status | Command::History => {
                let b = self.bound(input).await?;
                let ws = self.validate_binding(&snapshot.config, &b)?;
                let provider = self
                    .connections
                    .get(&snapshot.config, ws)
                    .await
                    .map_err(error)?;
                let current = provider.read(&b.thread).await.map_err(error)?;
                connections::require_workspace(&current.cwd, ws).map_err(error)?;
                if matches!(input.command, Command::History) {
                    Ok(session::delivery::full_history_in(
                        &self.history(provider.as_ref(), ws, &b.thread).await?,
                        &locale,
                    ))
                } else {
                    Ok(i18n::t!(
                        "discord.session.details",
                        locale = &locale,
                        title = current.title,
                        state = current.status.label_in(&locale),
                        workspace = &ws.name,
                        thread = &b.thread
                    ))
                }
            }
        }
    }
}
