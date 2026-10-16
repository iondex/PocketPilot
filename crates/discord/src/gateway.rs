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
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };
        if command.guild_id.map(|g| g.get()) != Some(self.api.guild)
            || !command.data.name.starts_with("pp-")
        {
            return;
        }
        // ACK before SSH or owner lookup, keeping all command replies private.
        if command.defer_ephemeral(&ctx.http).await.is_err() {
            return;
        }
        let mut fields = BTreeMap::new();
        let mut confirmed = false;
        for option in &command.data.options {
            match &option.value {
                CommandDataOptionValue::String(value) => {
                    fields.insert(option.name.clone(), value.clone());
                }
                CommandDataOptionValue::Boolean(value) if option.name == "confirm" => {
                    confirmed = *value
                }
                _ => {}
            }
        }
        let parsed = crate::parse(&command.data.name, &fields, confirmed);
        let text = match parsed {
            Ok(action) => {
                let input = Input {
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
                } else {
                    self.backend
                        .handle(&self.api, input)
                        .await
                        .unwrap_or_else(|e| e.render(&command.locale))
                }
            }
            Err(error) => i18n::LocalizedError::localized(&error, &command.locale),
        };
        let mut reply = EditInteractionResponse::new().allowed_mentions(crate::api::mentions());
        if text.len() > 8 * 1024 * 1024 {
            reply = reply.content(i18n::text(&command.locale, "discord.history.too_large"));
        } else if text.chars().count() > 1800 {
            reply = reply
                .content(i18n::text(&command.locale, "discord.history.attachment"))
                .new_attachment(CreateAttachment::bytes(text.into_bytes(), "session.md"));
        } else {
            reply = reply.content(text);
        }
        if command.edit_response(&ctx.http, reply).await.is_err() {
            tracing::warn!("Discord interaction response unavailable; not resent");
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
        localized_command("pp-workspaces", "discord.cmd.workspaces"),
        localized_command("pp-sessions", "discord.cmd.sessions")
            .add_option(string("workspace", "discord.option.workspace", true))
            .add_option(string("cursor", "discord.option.cursor", false)),
        localized_command("pp-open", "discord.cmd.open")
            .add_option(string("workspace", "discord.option.workspace", true))
            .add_option(string("thread", "discord.option.thread", true)),
        localized_command("pp-new", "discord.cmd.new")
            .add_option(string("workspace", "discord.option.workspace", true))
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
        localized_command("pp-ask", "discord.cmd.ask").add_option(string(
            "text",
            "discord.option.input",
            true,
        )),
        localized_command("pp-status", "discord.cmd.status"),
        localized_command("pp-history", "discord.cmd.history"),
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
    for command in commands() {
        GuildId::new(guild)
            .create_command(&http, command)
            .await
            .map_err(|_| Error::Request)?;
    }
    let mut client = serenity::client::ClientBuilder::new_with_http(
        crate::api::http(token.expose_secret())?,
        GatewayIntents::GUILDS,
    )
    .event_handler(Handler { backend, api })
    .await
    .map_err(|_| Error::Request)?;
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
