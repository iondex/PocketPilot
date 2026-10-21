//! Modal opening must be the first response; submitted drafts are acknowledged before processing.
use super::*;

impl Handler {
    pub(super) async fn open_form(
        &self,
        http: &serenity::http::Http,
        interaction: &ComponentInteraction,
        token: &str,
    ) {
        if interaction.guild_id.map(|id| id.get()) != Some(self.api.guild)
            || interaction.message.author.id.get() != self.api.bot
            || !interaction
                .message
                .flags
                .is_some_and(|flags| flags.contains(MessageFlags::EPHEMERAL))
            || !matches!(interaction.data.kind, ComponentInteractionDataKind::Button)
        {
            return;
        }
        let input = Input {
            uploads: vec![],
            locale: Some(interaction.locale.clone()),
            id: interaction.id.get(),
            bot: self.api.bot,
            guild: self.api.guild,
            channel: interaction.channel_id.get(),
            user: interaction.user.id.get(),
            date: interaction.id.created_at().unix_timestamp(),
            command: crate::discord::Command::Workspaces,
        };
        // A modal cannot follow a deferred ACK. Limit preparation to local state,
        // owner validation and two seconds; never connect to an agent here.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.backend.new_session_form(&self.api, input, token),
        )
        .await;
        let response = match result {
            Ok(Ok(form)) => CreateInteractionResponse::Modal(
                CreateModal::new(format!("pp-create:{}", form.token), form.title).components(vec![
                    CreateActionRow::InputText(
                        CreateInputText::new(
                            InputTextStyle::Paragraph,
                            i18n::text(&interaction.locale, "discord.option.first_input"),
                            "text",
                        )
                        .required(true)
                        .min_length(1)
                        .max_length(4000)
                        .placeholder(i18n::text(
                            &interaction.locale,
                            "discord.ui.create_placeholder",
                        )),
                    ),
                ]),
            ),
            result => {
                let error = match result {
                    Ok(Err(error)) => error,
                    _ => i18n::Message::new("discord.ui.expired"),
                };
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .ephemeral(true)
                        .allowed_mentions(crate::discord::api::mentions())
                        .content(error.render(&interaction.locale)),
                )
            }
        };
        if interaction.create_response(http, response).await.is_err() {
            tracing::warn!("Discord creation form response unavailable; not resent");
        }
    }

    pub(super) async fn modal(&self, http: &serenity::http::Http, interaction: &ModalInteraction) {
        let Some(token) = interaction.data.custom_id.strip_prefix("pp-create:") else {
            return;
        };
        if interaction.guild_id.map(|id| id.get()) != Some(self.api.guild) {
            return;
        }
        if interaction.defer_ephemeral(http).await.is_err() {
            return;
        }
        let text = interaction
            .data
            .components
            .iter()
            .flat_map(|row| &row.components)
            .find_map(|component| match component {
                ActionRowComponent::InputText(input) if input.custom_id == "text" => {
                    input.value.clone()
                }
                _ => None,
            });
        let input = Input {
            uploads: vec![],
            locale: Some(interaction.locale.clone()),
            id: interaction.id.get(),
            bot: self.api.bot,
            guild: self.api.guild,
            channel: interaction.channel_id.get(),
            user: interaction.user.id.get(),
            date: interaction.id.created_at().unix_timestamp(),
            command: crate::discord::Command::Workspaces,
        };
        let result = match text {
            Some(text) if !text.trim().is_empty() && text.chars().count() <= 4000 => {
                self.backend
                    .menu(&self.api, input, Some(token.into()), Some(text))
                    .await
            }
            _ => Err(i18n::Message::new("error.discord.missing_input")),
        };
        match result {
            Ok(screen) => {
                if update_menu(http, interaction.application_id, &interaction.token, screen)
                    .await
                    .is_err()
                {
                    tracing::warn!("Discord creation confirmation unavailable; not resent");
                }
            }
            Err(error) => {
                if interaction
                    .edit_response(
                        http,
                        EditInteractionResponse::new()
                            .allowed_mentions(crate::discord::api::mentions())
                            .content(crate::discord::render::literal(
                                &error.render(&interaction.locale),
                            )),
                    )
                    .await
                    .is_err()
                {
                    tracing::warn!("Discord creation error response unavailable; not resent");
                }
            }
        }
    }
}
