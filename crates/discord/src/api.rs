use crate::{Error, Input, authorized};
use serenity::{all::*, http::Http};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct Api {
    pub(crate) http: Arc<Http>,
    pub guild: u64,
    pub bot: u64,
}

impl Api {
    #[cfg(feature = "test-support")]
    pub fn local_fixture(endpoint: &str, guild: u64, bot: u64) -> Self {
        Self {
            http: Arc::new(
                serenity::http::HttpBuilder::new("fixture-token")
                    .client(
                        reqwest::Client::builder()
                            .no_proxy()
                            .build()
                            .expect("fixture HTTP client"),
                    )
                    .proxy(endpoint)
                    .ratelimiter_disabled(true)
                    .build(),
            ),
            guild,
            bot,
        }
    }

    pub async fn connect(token: &secrecy::SecretString, guild: u64) -> Result<Self, Error> {
        use secrecy::ExposeSecret;
        let http = Arc::new(http(token.expose_secret())?);
        let bot = http
            .get_current_user()
            .await
            .map_err(|_| Error::Request)?
            .id
            .get();
        http.get_guild(GuildId::new(guild))
            .await
            .map_err(|_| Error::Request)?;
        let application = http
            .get_current_application_info()
            .await
            .map_err(|_| Error::Request)?;
        http.set_application_id(application.id);
        Ok(Self { http, guild, bot })
    }

    pub async fn authorize(&self, input: &Input) -> Result<(), Error> {
        let guild = tokio::time::timeout(
            Duration::from_secs(15),
            self.http.get_guild(GuildId::new(self.guild)),
        )
        .await
        .map_err(|_| Error::Request)?
        .map_err(|_| Error::Request)?;
        if input.bot != self.bot
            || !authorized(
                self.guild,
                Some(input.guild),
                guild.owner_id.get(),
                input.user,
            )
        {
            return Err(Error::Denied);
        }
        Ok(())
    }

    pub async fn category(&self, name: &str, owner: u64) -> Result<u64, Error> {
        let permissions = [
            PermissionOverwrite {
                kind: PermissionOverwriteType::Role(RoleId::new(self.guild)),
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            },
            PermissionOverwrite {
                kind: PermissionOverwriteType::Member(UserId::new(owner)),
                allow: Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::READ_MESSAGE_HISTORY,
                deny: Permissions::empty(),
            },
            PermissionOverwrite {
                kind: PermissionOverwriteType::Member(UserId::new(self.bot)),
                allow: Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::READ_MESSAGE_HISTORY
                    | Permissions::ATTACH_FILES
                    | Permissions::MANAGE_CHANNELS,
                deny: Permissions::empty(),
            },
        ];
        let channel = GuildId::new(self.guild)
            .create_channel(
                &self.http,
                CreateChannel::new(session::delivery::short_title(name, 90))
                    .kind(ChannelType::Category)
                    .permissions(permissions),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(channel.id.get())
    }

    pub async fn channel(&self, category: u64, title: &str, thread: &str) -> Result<u64, Error> {
        let channel = GuildId::new(self.guild)
            .create_channel(
                &self.http,
                CreateChannel::new(crate::channel_name(title, thread))
                    .category(ChannelId::new(category))
                    .topic(format!(
                        "PocketPilot · {thread} · /pp-status /pp-ask /pp-history"
                    )),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(channel.id.get())
    }

    pub async fn rename(&self, channel: u64, title: &str, thread: &str) -> Result<(), Error> {
        ChannelId::new(channel)
            .edit(
                &self.http,
                EditChannel::new().name(crate::channel_name(title, thread)),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(())
    }

    pub async fn send(&self, channel: u64, text: &str) -> Result<u64, Error> {
        if text.len() > 8 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let mut message = CreateMessage::new().allowed_mentions(mentions());
        if text.chars().count() > 1800 {
            message = message
                .content("会话消息集合见附件。")
                .add_file(CreateAttachment::bytes(
                    text.as_bytes().to_vec(),
                    "session.md",
                ));
        } else {
            message = message.content(text);
        }
        ChannelId::new(channel)
            .send_message(&self.http, message)
            .await
            .map(|m| m.id.get())
            .map_err(|_| Error::Request)
    }

    pub async fn card(&self, channel: u64, message: u64, text: &str) -> Result<(), Error> {
        ChannelId::new(channel)
            .edit_message(
                &self.http,
                MessageId::new(message),
                EditMessage::new()
                    .content(text)
                    .allowed_mentions(mentions()),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(())
    }
}

pub(crate) fn http(token: &str) -> Result<Http, Error> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| Error::Request)?;
    Ok(serenity::http::HttpBuilder::new(token)
        .client(client)
        .build())
}

pub(crate) fn mentions() -> CreateAllowedMentions {
    CreateAllowedMentions::new()
        .all_users(false)
        .all_roles(false)
        .everyone(false)
        .replied_user(false)
}
