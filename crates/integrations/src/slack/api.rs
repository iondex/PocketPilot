//! Keep Slack HTTP calls and bounded resource downloads behind typed platform methods.

use crate::slack::{Error, Result, config::Config, model::Content};
use slack_morphism::{errors::SlackClientError, prelude::*};
use std::{sync::Arc, time::Duration};

#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    async fn download(
        &self,
        _file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded> {
        Err(Error::Request)
    }

    async fn channel(&self, channel: &str) -> Result<()>;

    async fn post(&self, channel: &str, root: Option<&str>, content: &Content) -> Result<String>;

    async fn update(&self, channel: &str, root: &str, content: &Content) -> Result<()>;
}

pub type Client =
    SlackClient<SlackClientHyperConnector<hyper_rustls::HttpsConnector<crate::network::Connector>>>;

pub struct Api {
    pub proxy: crate::network::Proxy,
    pub client: Arc<Client>,
    pub token: SlackApiToken,
    pub direct_channel: String,
}

// The SDK has no mrkdwn field on these requests. Keep its DTOs and add the
// documented top-level flag so notifications stay literal beside rich_text blocks.
#[derive(serde::Serialize)]
struct LiteralFallback<T> {
    #[serde(flatten)]
    request: T,
    mrkdwn: bool,
    link_names: bool,
}

pub fn map_error(error: SlackClientError) -> Error {
    match error {
        SlackClientError::RateLimitError(e) => Error::RateLimit(
            e.retry_after
                .map(|d| d.as_secs())
                .unwrap_or(30)
                .clamp(1, 3600),
        ),
        SlackClientError::ApiError(e) if e.code == "ratelimited" => Error::RateLimit(30),
        SlackClientError::ApiError(e)
            if matches!(
                e.code.as_str(),
                "invalid_auth" | "token_revoked" | "not_authed"
            ) =>
        {
            Error::Ui("slack.error.credentials")
        }
        SlackClientError::ApiError(e)
            if matches!(e.code.as_str(), "missing_scope" | "no_permission") =>
        {
            Error::Ui("slack.error.permissions")
        }
        SlackClientError::ApiError(e)
            if matches!(
                e.code.as_str(),
                "not_in_channel" | "channel_not_found" | "is_archived"
            ) =>
        {
            Error::Ui("slack.error.channel")
        }
        _ => Error::Request,
    }
}

pub async fn bounded<T>(
    future: impl std::future::Future<Output = std::result::Result<T, SlackClientError>>,
) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .map_err(|_| Error::Request)?
        .map_err(map_error)
}

impl Api {
    pub async fn connect(config: &Config) -> Result<(Arc<Self>, String)> {
        crate::network::initialize_tls();
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|_| Error::Request)?
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(crate::network::Connector(config.proxy.clone()));
        let connector = SlackClientHyperConnector::with_connector(https).with_rate_control(
            SlackApiRateControlConfig::new()
                .with_max_retries(0)
                .with_max_delay_timeout(Duration::from_secs(2)),
        );
        let client = Arc::new(SlackClient::new(connector));
        let token = SlackApiToken::new(config.bot_token.clone().into())
            .with_team_id(config.team_id.clone().into());
        let response = bounded(client.open_session(&token).auth_test()).await?;
        if response.team_id.value() != &config.team_id
            || response.bot_id.is_none()
            || response.user_id.value() == &config.owner_user_id
        {
            return Err(Error::Ui("slack.error.identity"));
        }
        let direct_channel =
            Self::owner_conversation(&client, &token, &config.owner_user_id).await?;
        Ok((
            Arc::new(Self {
                proxy: config.proxy.clone(),
                client,
                token,
                direct_channel,
            }),
            response.user_id.to_string(),
        ))
    }

    pub(crate) async fn owner_conversation(
        client: &Client,
        token: &SlackApiToken,
        owner: &str,
    ) -> Result<String> {
        let request = SlackApiConversationsOpenRequest::new().with_users(vec![owner.into()]);
        let opened = bounded(client.open_session(token).conversations_open(&request)).await?;
        let channel = opened.channel.id.to_string();
        if !channel.starts_with('D') {
            return Err(Error::Ui("slack.error.identity"));
        }
        Ok(channel)
    }

    pub fn content(content: &Content) -> SlackMessageContent {
        let fallback = if content.format == session::markdown::Format::Markdown {
            session::markdown::Document::parse(&content.text)
                .map(|doc| doc.plain())
                .unwrap_or_else(|_| content.text.clone())
        } else {
            content.text.clone()
        };
        let mut result =
            SlackMessageContent::new().with_text(crate::slack::render::escape(&fallback));
        if !content.blocks.is_empty() {
            result = result.with_blocks(content.blocks.clone());
        }
        result
    }
}

#[async_trait::async_trait]
impl Platform for Api {
    async fn download(
        &self,
        file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded> {
        let info = bounded(
            self.client
                .open_session(&self.token)
                .files_info(&SlackApiFilesInfoRequest::new(file.location.clone().into())),
        )
        .await?;
        let url = info
            .file
            .url_private_download
            .clone()
            .or(info.file.url_private.clone())
            .ok_or(Error::Request)?;
        if url.scheme() != "https"
            || url.host_str() != Some("files.slack.com")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Request);
        }
        let bytes = crate::network::download::bytes(
            &self.proxy,
            url.as_str(),
            Some(self.token.token_value.value()),
            session::files::MAX_BYTES,
        )
        .await
        .map_err(|_| Error::Request)?;
        Ok(session::files::Downloaded {
            name: info.file.name.unwrap_or_else(|| file.name.clone()),
            image: info
                .file
                .mimetype
                .as_ref()
                .is_some_and(|m| session::files::image_mime(m.value())),
            bytes,
        })
    }

    async fn channel(&self, channel: &str) -> Result<()> {
        if channel != self.direct_channel {
            return Err(Error::Ui("slack.error.scope"));
        }
        Ok(())
    }

    async fn post(&self, channel: &str, root: Option<&str>, content: &Content) -> Result<String> {
        self.channel(channel).await?;
        let session = self.client.open_session(&self.token);
        let request = SlackApiChatPostMessageRequest::new(channel.into(), Self::content(content))
            .opt_thread_ts(root.map(Into::into))
            .with_reply_broadcast(false)
            .with_unfurl_links(false)
            .with_unfurl_media(false)
            .with_parse("none".into());
        let request = LiteralFallback {
            request,
            mrkdwn: false,
            link_names: false,
        };
        let response: SlackApiChatPostMessageResponse = bounded(
            session
                .http_session_api
                .http_post("chat.postMessage", &request, None),
        )
        .await?;
        Ok(response.ts.to_string())
    }

    async fn update(&self, channel: &str, root: &str, content: &Content) -> Result<()> {
        self.channel(channel).await?;
        let request =
            SlackApiChatUpdateRequest::new(channel.into(), Self::content(content), root.into());
        let request = LiteralFallback {
            request,
            mrkdwn: false,
            link_names: false,
        };
        let _: SlackApiChatUpdateResponse = bounded(
            self.client
                .open_session(&self.token)
                .http_session_api
                .http_post("chat.update", &request, None),
        )
        .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::AttachmentSource for Api {
    type Error = Error;

    async fn download(
        &self,
        file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded> {
        Platform::download(self, file).await
    }
}
