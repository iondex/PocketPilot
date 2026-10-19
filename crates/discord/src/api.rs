use crate::{Error, Input, authorized};
use serenity::{all::*, http::Http};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct Api {
    pub(crate) http: Arc<Http>,
    pub guild: u64,
    pub bot: u64,
    pub(crate) proxy: network::Proxy,
}

impl Api {
    pub async fn download(
        &self,
        file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded, Error> {
        let url = reqwest::Url::parse(&file.location).map_err(|_| Error::Request)?;
        if url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("cdn.discordapp.com" | "media.discordapp.net")
            )
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Request);
        }
        let bytes =
            network::download::bytes(&self.proxy, url.as_str(), None, session::files::MAX_BYTES)
                .await
                .map_err(|_| Error::Request)?;
        Ok(session::files::Downloaded {
            name: file.name.clone(),
            image: file.image,
            bytes,
        })
    }
    /// Only reads identity and guild metadata; does not register commands or connect Gateway.
    pub async fn inspect(config: &crate::config::Config) -> Result<crate::Identity, Error> {
        let http = http(&config.token, &config.proxy)?;
        let bot = http.get_current_user().await.map_err(|_| Error::Request)?;
        let application = http
            .get_current_application_info()
            .await
            .map_err(|_| Error::Request)?;
        let guild = http
            .get_guild(GuildId::new(config.guild_id))
            .await
            .map_err(|_| Error::Request)?;
        Ok(crate::Identity {
            bot_id: bot.id.to_string(),
            application_id: application.id.to_string(),
            guild_id: guild.id.to_string(),
            owner_id: guild.owner_id.to_string(),
            install_url: format!(
                "https://discord.com/oauth2/authorize?client_id={}&scope=bot%20applications.commands&permissions=101392&guild_id={}&disable_guild_select=true",
                application.id, guild.id
            ),
        })
    }

    #[cfg(any(feature = "test-support", test))]
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
            proxy: Default::default(),
        }
    }

    pub async fn connect(
        token: &secrecy::SecretString,
        guild: u64,
        proxy: &network::Proxy,
    ) -> Result<Self, Error> {
        use secrecy::ExposeSecret;
        let http = Arc::new(http(token.expose_secret(), proxy)?);
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
        Ok(Self {
            http,
            guild,
            bot,
            proxy: proxy.clone(),
        })
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

    pub async fn channel(&self, category: u64, title: &str) -> Result<u64, Error> {
        let channel = GuildId::new(self.guild)
            .create_channel(
                &self.http,
                CreateChannel::new(crate::channel_name(title))
                    .category(ChannelId::new(category))
                    .topic(i18n::text("zh-CN", "discord.ui.channel_topic")),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(channel.id.get())
    }

    pub async fn rename(&self, channel: u64, title: &str) -> Result<(), Error> {
        ChannelId::new(channel)
            .edit(
                &self.http,
                EditChannel::new().name(crate::channel_name(title)),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(())
    }

    pub async fn send(&self, channel: u64, text: &str) -> Result<u64, Error> {
        self.send_in(channel, text, "zh-CN").await
    }

    pub async fn send_in(&self, channel: u64, text: &str, locale: &str) -> Result<u64, Error> {
        self.send_formatted(channel, text, locale, session::markdown::Format::Markdown)
            .await
    }

    pub(crate) async fn send_plain_in(
        &self,
        channel: u64,
        text: &str,
        locale: &str,
    ) -> Result<u64, Error> {
        self.send_formatted(channel, text, locale, session::markdown::Format::Plain)
            .await
    }

    async fn send_formatted(
        &self,
        channel: u64,
        text: &str,
        _locale: &str,
        format: session::markdown::Format,
    ) -> Result<u64, Error> {
        let parts = crate::render::messages(text, format).map_err(|_| Error::TooLarge)?;
        if parts.len() != 1 {
            return Err(Error::TooLarge);
        }
        self.send_rendered(channel, &parts[0]).await
    }

    pub(crate) async fn send_rendered(&self, channel: u64, text: &str) -> Result<u64, Error> {
        let message = CreateMessage::new()
            .allowed_mentions(mentions())
            .content(text);
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
                    .content(crate::render::literal(text))
                    .allowed_mentions(mentions()),
            )
            .await
            .map_err(|_| Error::Request)?;
        Ok(())
    }
}

pub(crate) fn http(token: &str, proxy: &network::Proxy) -> Result<Http, Error> {
    let client = proxy
        .http_builder()
        .map_err(|_| Error::Request)?
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
