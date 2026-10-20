//! Acknowledge, authorize and dispatch private Discord interactions.
use crate::discord::{Api, Backend, Input};
use serenity::{all::*, async_trait};
use std::{collections::BTreeMap, sync::Arc};

use super::command_menu;

pub(super) struct Handler {
    pub(super) backend: Arc<dyn Backend>,
    pub(super) api: Api,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        tracing::info!(
            guild_id = self.api.guild,
            bot_id = ready.user.id.get(),
            "Discord Gateway ready"
        );
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Component(component) = &interaction {
            self.component(&ctx.http, component).await;
            return;
        }
        if let Interaction::Command(command) = interaction {
            self.command(&ctx.http, command).await;
        }
    }
}

impl Handler {
    fn command_input(&self, command: &CommandInteraction) -> Result<Input, crate::discord::Error> {
        let mut fields = BTreeMap::new();
        let mut confirmed = false;
        let mut uploads = Vec::new();
        for option in &command.data.options {
            match &option.value {
                CommandDataOptionValue::String(value) => {
                    fields.insert(option.name.clone(), value.clone());
                }
                CommandDataOptionValue::Boolean(value) if option.name == "confirm" => {
                    confirmed = *value
                }
                CommandDataOptionValue::Attachment(id) => {
                    if let Some(file) = command.data.resolved.attachments.get(id) {
                        uploads.push(session::files::Incoming {
                            name: file.filename.clone(),
                            location: file.url.clone(),
                            image: file
                                .content_type
                                .as_ref()
                                .is_some_and(|m| session::files::image_mime(m)),
                        });
                    }
                }
                _ => {}
            }
        }
        if !uploads.is_empty() && fields.get("text").is_none_or(|s| s.trim().is_empty()) {
            fields.insert(
                "text".into(),
                i18n::text(&command.locale, "common.file.request"),
            );
        }
        let action = crate::discord::parse(&command.data.name, &fields, confirmed)?;
        Ok(Input {
            uploads,
            locale: Some(command.locale.clone()),
            id: command.id.get(),
            bot: self.api.bot,
            guild: self.api.guild,
            channel: command.channel_id.get(),
            user: command.user.id.get(),
            date: command.id.created_at().unix_timestamp(),
            command: action,
        })
    }

    async fn command(&self, http: &serenity::http::Http, command: CommandInteraction) {
        if command.guild_id.map(|g| g.get()) != Some(self.api.guild)
            || !command_menu::owns(&command.data.name)
        {
            return;
        }
        // ACK before SSH or owner lookup, keeping all command replies private.
        if command.defer_ephemeral(http).await.is_err() {
            return;
        }
        let mut channel_link = false;
        let text = match self.command_input(&command) {
            Ok(input) => {
                let opened = matches!(
                    input.command,
                    crate::discord::Command::Open { .. } | crate::discord::Command::New { .. }
                );
                if self.api.authorize(&input).await.is_err() {
                    i18n::text(&command.locale, "discord.owner_only")
                } else if matches!(
                    input.command,
                    crate::discord::Command::Workspaces
                        | crate::discord::Command::Projects { .. }
                        | crate::discord::Command::Sessions { .. }
                ) {
                    match self.backend.menu(&self.api, input, None, None).await {
                        Ok(screen) => {
                            if update_menu(http, command.application_id, &command.token, screen)
                                .await
                                .is_err()
                            {
                                tracing::warn!("Discord menu response unavailable; not resent");
                            }
                            return;
                        }
                        Err(error) => error.render(&command.locale),
                    }
                } else {
                    match self.backend.handle(&self.api, input).await {
                        Ok(text) => {
                            // Open/New return only localized static text, a numeric channel ID,
                            // and a localized state. Preserve that generated navigation link.
                            channel_link = opened;
                            text
                        }
                        Err(error) => error.render(&command.locale),
                    }
                }
            }
            Err(error) => i18n::LocalizedError::localized(&error, &command.locale),
        };
        let parts = if channel_link {
            vec![text]
        } else {
            crate::discord::render::messages(&text, session::markdown::Format::Plain)
                .unwrap_or_else(|_| vec![i18n::text(&command.locale, "discord.history.too_large")])
        };
        let mut parts = parts.into_iter();
        let reply = EditInteractionResponse::new()
            .allowed_mentions(crate::discord::api::mentions())
            .content(parts.next().unwrap_or_default());
        if command.edit_response(http, reply).await.is_err() {
            tracing::warn!("Discord interaction response unavailable; not resent");
            return;
        }
        for part in parts {
            let reply = CreateInteractionResponseFollowup::new()
                .ephemeral(true)
                .allowed_mentions(crate::discord::api::mentions())
                .content(part);
            if command.create_followup(http, reply).await.is_err() {
                tracing::warn!("Discord interaction continuation unavailable; not resent");
                break;
            }
        }
    }

    async fn component(&self, http: &serenity::http::Http, interaction: &ComponentInteraction) {
        let Some(token) = interaction.data.custom_id.strip_prefix("pp:") else {
            return;
        };
        if interaction.guild_id.map(|id| id.get()) != Some(self.api.guild)
            || interaction.message.author.id.get() != self.api.bot
            || !interaction
                .message
                .flags
                .is_some_and(|f| f.contains(MessageFlags::EPHEMERAL))
        {
            return;
        }
        let value = match &interaction.data.kind {
            ComponentInteractionDataKind::Button => None,
            ComponentInteractionDataKind::StringSelect { values } if values.len() == 1 => {
                values.first().cloned()
            }
            _ => return,
        };
        // Acknowledge the update before catalog/SSH work; the result edits this same private menu.
        if interaction.defer(http).await.is_err() {
            return;
        }
        let input = Input {
            uploads: Vec::new(),
            locale: Some(interaction.locale.clone()),
            id: interaction.id.get(),
            bot: self.api.bot,
            guild: self.api.guild,
            channel: interaction.channel_id.get(),
            user: interaction.user.id.get(),
            date: interaction.id.created_at().unix_timestamp(),
            command: crate::discord::Command::Workspaces,
        };
        let result = self
            .backend
            .menu(&self.api, input, Some(token.into()), value)
            .await;
        match result {
            Ok(screen) => {
                if update_menu(http, interaction.application_id, &interaction.token, screen)
                    .await
                    .is_err()
                {
                    tracing::warn!("Discord menu update unavailable; operation not replayed");
                }
            }
            Err(error) => {
                // Leave the last usable screen in place; only the clicking user sees the error.
                let reply = CreateInteractionResponseFollowup::new()
                    .ephemeral(true)
                    .content(error.render(&interaction.locale))
                    .allowed_mentions(crate::discord::api::mentions());
                if interaction.create_followup(http, reply).await.is_err() {
                    tracing::warn!("Discord menu error response unavailable; not resent");
                }
            }
        }
    }
}

async fn update_menu(
    http: &serenity::http::Http,
    application_id: ApplicationId,
    token: &str,
    screen: crate::discord::ui::Screen,
) -> Result<(), crate::discord::Error> {
    crate::discord::native::edit(
        http,
        serenity::http::Route::WebhookOriginalInteractionResponse {
            application_id,
            token,
        },
        &screen.message(),
    )
    .await
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
