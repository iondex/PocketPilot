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
                    id: command.id.get(),
                    bot: self.api.bot,
                    guild: self.api.guild,
                    channel: command.channel_id.get(),
                    user: command.user.id.get(),
                    date: command.id.created_at().unix_timestamp(),
                    command: action,
                };
                if self.api.authorize(&input).await.is_err() {
                    "此原型仅允许配置服务器的 owner 使用。".into()
                } else {
                    self.backend
                        .handle(&self.api, input)
                        .await
                        .unwrap_or_else(|e| e)
                }
            }
            Err(_) => "无效命令。".into(),
        };
        let mut reply = EditInteractionResponse::new().allowed_mentions(crate::api::mentions());
        if text.len() > 8 * 1024 * 1024 {
            reply = reply.content("历史超过单个附件上限，未截断。请在原客户端查看。");
        } else if text.chars().count() > 1800 {
            reply = reply
                .content("会话消息集合见附件。")
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
    CreateCommandOption::new(CommandOptionType::String, name, description).required(required)
}

fn commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("pp-workspaces").description("允许的工作区"),
        CreateCommand::new("pp-sessions")
            .description("查看工作区会话")
            .add_option(string("workspace", "工作区短名或 ID", true))
            .add_option(string("cursor", "上一页返回的分页游标", false)),
        CreateCommand::new("pp-open")
            .description("将已有会话加入自己的 channel")
            .add_option(string("workspace", "工作区短名或 ID", true))
            .add_option(string("thread", "Codex thread ID", true)),
        CreateCommand::new("pp-new")
            .description("创建会话并发送首条输入")
            .add_option(string("workspace", "工作区短名或 ID", true))
            .add_option(string("text", "首条用户输入", true))
            .add_option(
                CreateCommandOption::new(CommandOptionType::Boolean, "confirm", "确认创建并发送")
                    .required(true),
            ),
        CreateCommand::new("pp-ask")
            .description("继续当前 channel 会话；活跃时 steer")
            .add_option(string("text", "用户输入", true)),
        CreateCommand::new("pp-status").description("当前 channel 会话状态"),
        CreateCommand::new("pp-history").description("一个集合查看完整可见历史"),
        CreateCommand::new("pp-rename")
            .description("仅修改 channel 显示名")
            .add_option(string("title", "显示名称", true)),
        CreateCommand::new("pp-unbind").description("仅解除 channel 映射，会话继续运行"),
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
