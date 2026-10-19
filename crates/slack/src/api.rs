use crate::{Error, Result, config::Config, model::Content};
use slack_morphism::{errors::SlackClientError, prelude::*};
use std::{sync::Arc, time::Duration};

pub struct Upload {
    pub id: String,
    pub url: String,
}

#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    async fn channel(&self, channel: &str) -> Result<()>;

    async fn post(
        &self,
        channel: &str,
        root: Option<&str>,
        recipient: Option<&str>,
        content: &Content,
    ) -> Result<String>;

    async fn update(&self, channel: &str, root: &str, content: &Content) -> Result<()>;

    async fn prepare_file(&self, length: usize) -> Result<Upload>;

    async fn upload_file(&self, upload: &Upload, bytes: Vec<u8>) -> Result<()>;

    async fn complete_file(&self, upload: &Upload, channel: &str, root: Option<&str>)
    -> Result<()>;
}

pub struct Api {
    pub client: Arc<SlackHyperClient>,
    pub token: SlackApiToken,
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
        let connector = SlackClientHyperConnector::new()
            .map_err(|_| Error::Request)?
            .with_rate_control(
                SlackApiRateControlConfig::new()
                    .with_max_retries(0)
                    .with_max_delay_timeout(Duration::from_secs(2)),
            );
        let api = Arc::new(Self {
            client: Arc::new(SlackClient::new(connector)),
            token: SlackApiToken::new(config.bot_token.clone().into())
                .with_team_id(config.team_id.clone().into()),
        });
        let response = bounded(api.client.open_session(&api.token).auth_test()).await?;
        if response.team_id.value() != &config.team_id || response.bot_id.is_none() {
            return Err(Error::Ui("slack.error.identity"));
        }
        Ok((api, response.user_id.to_string()))
    }

    pub fn content(content: &Content) -> SlackMessageContent {
        let mut result = SlackMessageContent::new().with_text(crate::render::escape(&content.text));
        if !content.blocks.is_empty() {
            result = result.with_blocks(content.blocks.clone());
        }
        result
    }
}

#[async_trait::async_trait]
impl Platform for Api {
    async fn channel(&self, channel: &str) -> Result<()> {
        let response = bounded(
            self.client
                .open_session(&self.token)
                .conversations_info(&SlackApiConversationsInfoRequest::new(channel.into())),
        )
        .await?;
        let flags = response.channel.flags;
        if flags.is_member != Some(true)
            || flags.is_archived == Some(true)
            || flags.is_shared != Some(false)
            || flags.is_org_shared == Some(true)
            || flags.is_im == Some(true)
            || flags.is_mpim == Some(true)
        {
            return Err(Error::Ui("slack.error.channel"));
        }
        Ok(())
    }

    async fn post(
        &self,
        channel: &str,
        root: Option<&str>,
        recipient: Option<&str>,
        content: &Content,
    ) -> Result<String> {
        let session = self.client.open_session(&self.token);
        if let Some(user) = recipient {
            let request = SlackApiChatPostEphemeralRequest::new(
                channel.into(),
                user.into(),
                Self::content(content),
            )
            .opt_thread_ts(root.map(Into::into));
            bounded(session.chat_post_ephemeral(&request)).await?;
            return Ok(String::new());
        }
        let request = SlackApiChatPostMessageRequest::new(channel.into(), Self::content(content))
            .opt_thread_ts(root.map(Into::into))
            .with_reply_broadcast(false)
            .with_unfurl_links(false)
            .with_unfurl_media(false)
            .with_parse("none".into());
        Ok(bounded(session.chat_post_message(&request))
            .await?
            .ts
            .to_string())
    }

    async fn update(&self, channel: &str, root: &str, content: &Content) -> Result<()> {
        let request =
            SlackApiChatUpdateRequest::new(channel.into(), Self::content(content), root.into());
        bounded(self.client.open_session(&self.token).chat_update(&request)).await?;
        Ok(())
    }

    async fn prepare_file(&self, length: usize) -> Result<Upload> {
        let response = bounded(
            self.client
                .open_session(&self.token)
                .get_upload_url_external(&SlackApiFilesGetUploadUrlExternalRequest::new(
                    "session.md".into(),
                    length,
                )),
        )
        .await?;
        Ok(Upload {
            id: response.file_id.to_string(),
            url: response.upload_url.0.to_string(),
        })
    }

    async fn upload_file(&self, upload: &Upload, bytes: Vec<u8>) -> Result<()> {
        bounded(self.client.open_session(&self.token).files_upload_via_url(
            &SlackApiFilesUploadViaUrlRequest::new(
                SlackFileUploadUrl(upload.url.parse().map_err(|_| Error::Request)?),
                bytes,
                "text/markdown; charset=utf-8".into(),
            ),
        ))
        .await?;
        Ok(())
    }

    async fn complete_file(
        &self,
        upload: &Upload,
        channel: &str,
        root: Option<&str>,
    ) -> Result<()> {
        let request = SlackApiFilesCompleteUploadExternalRequest::new(vec![
            SlackApiFilesComplete::new(upload.id.clone().into()).with_title("session.md".into()),
        ])
        .with_channel_id(channel.into())
        .opt_thread_ts(root.map(Into::into));
        bounded(
            self.client
                .open_session(&self.token)
                .files_complete_upload_external(&request),
        )
        .await?;
        Ok(())
    }
}
