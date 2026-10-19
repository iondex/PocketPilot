use crate::{Api, Backend, Error, Input};
use secrecy::{ExposeSecret, SecretString};
use serenity::{all::*, async_trait};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;

struct Handler {
    backend: Arc<dyn Backend>,
    api: Api,
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

fn ours(name: &str) -> bool {
    name == "pp" || name.starts_with("pp-")
}

impl Handler {
    async fn command(&self, http: &serenity::http::Http, command: CommandInteraction) {
        if command.guild_id.map(|g| g.get()) != Some(self.api.guild) || !ours(&command.data.name) {
            return;
        }
        // ACK before SSH or owner lookup, keeping all command replies private.
        if command.defer_ephemeral(http).await.is_err() {
            return;
        }
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
        let parsed = crate::parse(&command.data.name, &fields, confirmed);
        let mut channel_link = false;
        let text = match parsed {
            Ok(action) => {
                let opened = matches!(
                    action,
                    crate::Command::Open { .. } | crate::Command::New { .. }
                );
                let input = Input {
                    uploads,
                    locale: Some(command.locale.clone()),
                    id: command.id.get(),
                    bot: self.api.bot,
                    guild: self.api.guild,
                    channel: command.channel_id.get(),
                    user: command.user.id.get(),
                    date: command.id.created_at().unix_timestamp(),
                    command: action,
                };
                if self.api.authorize(&input).await.is_err() {
                    i18n::text(&command.locale, "discord.owner_only")
                } else if matches!(
                    input.command,
                    crate::Command::Workspaces
                        | crate::Command::Projects { .. }
                        | crate::Command::Sessions { .. }
                ) {
                    match self.backend.menu(&self.api, input, None, None).await {
                        Ok(screen) => {
                            let reply = EditInteractionResponse::new()
                                .content(screen.text)
                                .components(screen.components)
                                .allowed_mentions(crate::api::mentions());
                            if command.edit_response(http, reply).await.is_err() {
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
            crate::render::messages(&text, session::markdown::Format::Plain)
                .unwrap_or_else(|_| vec![i18n::text(&command.locale, "discord.history.too_large")])
        };
        let mut parts = parts.into_iter();
        let reply = EditInteractionResponse::new()
            .allowed_mentions(crate::api::mentions())
            .content(parts.next().unwrap_or_default());
        if command.edit_response(http, reply).await.is_err() {
            tracing::warn!("Discord interaction response unavailable; not resent");
            return;
        }
        for part in parts {
            let reply = CreateInteractionResponseFollowup::new()
                .ephemeral(true)
                .allowed_mentions(crate::api::mentions())
                .content(part);
            if command.create_followup(http, reply).await.is_err() {
                tracing::warn!("Discord interaction continuation unavailable; not resent");
                break;
            }
        }
    }
}

impl Handler {
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
            command: crate::Command::Workspaces,
        };
        let result = self
            .backend
            .menu(&self.api, input, Some(token.into()), value)
            .await;
        match result {
            Ok(screen) => {
                let reply = EditInteractionResponse::new()
                    .content(screen.text)
                    .components(screen.components)
                    .allowed_mentions(crate::api::mentions());
                if interaction.edit_response(http, reply).await.is_err() {
                    tracing::warn!("Discord menu update unavailable; operation not replayed");
                }
            }
            Err(error) => {
                // Leave the last usable screen in place; only the clicking user sees the error.
                let reply = CreateInteractionResponseFollowup::new()
                    .ephemeral(true)
                    .content(error.render(&interaction.locale))
                    .allowed_mentions(crate::api::mentions());
                if interaction.create_followup(http, reply).await.is_err() {
                    tracing::warn!("Discord menu error response unavailable; not resent");
                }
            }
        }
    }
}

fn string(name: &str, description: &str, required: bool) -> CreateCommandOption {
    CreateCommandOption::new(
        CommandOptionType::String,
        name,
        i18n::text("zh-CN", description),
    )
    .description_localized("en-US", i18n::text("en", description))
    .description_localized("en-GB", i18n::text("en", description))
    .required(required)
}

fn localized_command(name: &str, id: &str) -> CreateCommand {
    CreateCommand::new(name)
        .description(i18n::text("zh-CN", id))
        .description_localized("en-US", i18n::text("en", id))
        .description_localized("en-GB", i18n::text("en", id))
}

fn commands() -> Vec<CreateCommand> {
    vec![
        localized_command("pp", "discord.cmd.menu"),
        localized_command("pp-workspaces", "discord.cmd.workspaces"),
        localized_command("pp-projects", "discord.cmd.projects").add_option(string(
            "workspace",
            "discord.option.workspace",
            false,
        )),
        localized_command("pp-sessions", "discord.cmd.sessions")
            .add_option(string("workspace", "discord.option.workspace", false))
            .add_option(string("project", "discord.option.project", false)),
        localized_command("pp-open", "discord.cmd.open"),
        localized_command("pp-new", "discord.cmd.new")
            .add_option(string("workspace", "discord.option.workspace", true))
            .add_option(string("project", "discord.option.project", true))
            .add_option(string("text", "discord.option.first_input", true))
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Boolean,
                    "confirm",
                    i18n::text("zh-CN", "discord.option.confirm"),
                )
                .description_localized("en-US", i18n::text("en", "discord.option.confirm"))
                .required(true),
            ),
        localized_command("pp-ask", "discord.cmd.ask")
            .add_option(string("text", "discord.option.input", false))
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Attachment,
                    "file",
                    i18n::text("zh-CN", "discord.option.file"),
                )
                .description_localized("en-US", i18n::text("en", "discord.option.file")),
            ),
        localized_command("pp-status", "discord.cmd.status"),
        localized_command("pp-recent", "discord.cmd.recent"),
        localized_command("pp-rename", "discord.cmd.rename").add_option(string(
            "title",
            "discord.option.title",
            true,
        )),
        localized_command("pp-unbind", "discord.cmd.unbind"),
    ]
    .into_iter()
    .map(|c| {
        c.default_member_permissions(Permissions::ADMINISTRATOR)
            .dm_permission(false)
    })
    .collect()
}

/// Installs only namespaced guild commands; other applications' commands are untouched.
pub async fn run(
    token: SecretString,
    api: Api,
    backend: Arc<dyn Backend>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let http = api.http.clone();
    let guild = api.guild;
    let bot = api.bot;
    for old in GuildId::new(guild)
        .get_commands(&http)
        .await
        .map_err(|_| Error::Request)?
    {
        if old.name == "pp-history" {
            GuildId::new(guild)
                .delete_command(&http, old.id)
                .await
                .map_err(|_| Error::Request)?;
        }
    }
    for command in commands() {
        GuildId::new(guild)
            .create_command(&http, command)
            .await
            .map_err(|_| Error::Request)?;
    }
    let proxy = api.proxy.clone();
    let mut client = serenity::client::ClientBuilder::new_with_http(
        crate::api::http(token.expose_secret(), &api.proxy)?,
        GatewayIntents::GUILDS,
    )
    .event_handler(Handler { backend, api })
    .await
    .map_err(|_| Error::Request)?;
    let _relay = if proxy.enabled() {
        let upstream = client.ws_url.lock().await.clone();
        let relay = network::Relay::start(proxy, upstream)
            .await
            .map_err(|_| Error::Request)?;
        *client.ws_url.lock().await = relay.endpoint.clone();
        Some(relay)
    } else {
        None
    };
    tracing::info!(
        guild_id = guild,
        bot_id = bot,
        "Discord slash-command prototype ready"
    );
    let manager = client.shard_manager.clone();
    let result = tokio::select! { result=client.start()=>result.map_err(|_|Error::Request), _=stop.changed()=>Ok(()) };
    manager.shutdown_all().await;
    result
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
