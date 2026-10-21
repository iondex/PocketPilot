//! Localized slash-command definitions and namespaced registration.
use crate::discord::{Api, Error};
use serenity::all::*;

pub(super) fn owns(name: &str) -> bool {
    name == "pp" || name.starts_with("pp-")
}

pub(super) async fn register(api: &Api) -> Result<(), Error> {
    for old in GuildId::new(api.guild)
        .get_commands(&api.http)
        .await
        .map_err(|_| Error::Request)?
    {
        if old.name == "pp-history" {
            GuildId::new(api.guild)
                .delete_command(&api.http, old.id)
                .await
                .map_err(|_| Error::Request)?;
        }
    }
    for command in commands() {
        GuildId::new(api.guild)
            .create_command(&api.http, command)
            .await
            .map_err(|_| Error::Request)?;
    }
    Ok(())
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
        localized_command("pp-bindings", "bindings.heading"),
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
