//! Feishu/Lark REST/authentication stay behind the SDK and named response DTOs.
use super::{
    Error, Result,
    config::{Config, identifier},
    model::{Destination, Resource},
    render::Card,
};
use crate::AttachmentSource;
use openlark_core::{
    api::{ApiRequest, ApiResponseTrait, ResponseFormat},
    constants::{AccessTokenType, AppType},
    http::Transport,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Deserialize, Serialize)]
pub struct Identity {
    pub open_id: String,
    pub app_name: String,
}

#[derive(Debug, Deserialize)]
struct BotResponse {
    bot: Identity,
}

impl ApiResponseTrait for BotResponse {
    fn data_format() -> ResponseFormat {
        ResponseFormat::Flatten
    }
}

#[derive(Debug, Deserialize)]
pub struct NativeMessage {
    pub message_id: String,
    #[serde(default)]
    pub thread_id: String,
}

impl ApiResponseTrait for NativeMessage {}

#[derive(Serialize)]
struct SendBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    receive_id: Option<&'a str>,
    msg_type: &'static str,
    content: String,
    uuid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_in_thread: Option<bool>,
}

#[derive(Serialize)]
struct PatchBody {
    content: String,
}

#[async_trait::async_trait]
pub(super) trait Platform: AttachmentSource<Error = Error> {
    async fn identity(&self) -> Result<Identity>;

    async fn send(
        &self,
        destination: &Destination,
        card: &Card,
        operation: &str,
    ) -> Result<NativeMessage>;

    async fn patch(&self, message: &str, card: &Card) -> Result<()>;
}

pub struct Api {
    pub(super) config: Arc<openlark_core::config::Config>,
}

impl Api {
    pub fn new(region: super::config::Region, settings: &Config) -> Result<Self> {
        Self::configured(settings, region.base_url(), false)
    }

    #[cfg(test)]
    pub(super) fn fixture(settings: &Config, base: &str) -> Result<Self> {
        Self::configured(settings, base, true)
    }

    fn configured(settings: &Config, base: &str, custom: bool) -> Result<Self> {
        crate::network::initialize_tls();
        // Feishu and Lark use direct connections only, including token refresh.
        let http = reqwest_feishu::Client::builder()
            .no_proxy()
            .redirect(reqwest_feishu::redirect::Policy::none())
            .retry(reqwest_feishu::retry::never())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Request)?;
        let config = openlark_core::config::Config::builder()
            .app_id(&settings.app_id)
            .app_secret(&settings.app_secret)
            .app_type(AppType::SelfBuild)
            .base_url(base)
            .allow_custom_base_url(custom)
            .http_client(http)
            .req_timeout(Duration::from_secs(30))
            .retry_count(0)
            .enable_log(false)
            .max_response_size((session::files::MAX_BYTES + 1024 * 1024) as u64)
            .build();
        let client =
            openlark_client::Client::with_core_config(config).map_err(|_| Error::Request)?;
        Ok(Self {
            config: Arc::new(client.config().clone()),
        })
    }
}

#[async_trait::async_trait]
impl Platform for Api {
    async fn identity(&self) -> Result<Identity> {
        let request = ApiRequest::<()>::get("/open-apis/bot/v3/info")
            .with_supported_access_token_types(vec![AccessTokenType::Tenant]);
        let response =
            Transport::<BotResponse>::request_typed(request, &self.config, None, "bot identity")
                .await
                .map_err(Error::from_sdk)?;
        if !identifier(&response.bot.open_id) {
            return Err(Error::Request);
        }
        Ok(response.bot)
    }

    async fn send(
        &self,
        destination: &Destination,
        card: &Card,
        operation: &str,
    ) -> Result<NativeMessage> {
        if !identifier(&destination.chat) || !card.bounded() || operation.len() > 50 {
            return Err(Error::Ui("feishu.error.scope"));
        }
        let path = if let Some(root) = &destination.root {
            if !identifier(root) {
                return Err(Error::Ui("feishu.error.scope"));
            }
            format!("/open-apis/im/v1/messages/{root}/reply")
        } else {
            "/open-apis/im/v1/messages".into()
        };
        let body = SendBody {
            receive_id: destination
                .root
                .is_none()
                .then_some(destination.chat.as_str()),
            msg_type: "interactive",
            content: serde_json::to_string(card)?,
            uuid: operation,
            reply_in_thread: destination.root.as_ref().map(|_| true),
        };
        let bytes = serde_json::to_vec(&body)?;
        if bytes.len() > 28_000 {
            return Err(Error::Ui("feishu.error.large"));
        }
        let mut request = ApiRequest::<()>::post(path)
            .with_supported_access_token_types(vec![AccessTokenType::Tenant])
            .body(bytes);
        if destination.root.is_none() {
            request = request.query("receive_id_type", "chat_id");
        }
        let reply =
            Transport::<NativeMessage>::request_typed(request, &self.config, None, "send message")
                .await
                .map_err(Error::from_sdk)?;
        if !identifier(&reply.message_id)
            || (!reply.thread_id.is_empty() && !identifier(&reply.thread_id))
        {
            return Err(Error::Request);
        }
        Ok(reply)
    }

    async fn patch(&self, message: &str, card: &Card) -> Result<()> {
        if !identifier(message) || !card.bounded() {
            return Err(Error::Ui("feishu.error.scope"));
        }
        let request = ApiRequest::<()>::patch(format!("/open-apis/im/v1/messages/{message}"))
            .with_supported_access_token_types(vec![AccessTokenType::Tenant])
            .body(serde_json::to_vec(&PatchBody {
                content: serde_json::to_string(card)?,
            })?);
        Transport::<()>::request_typed(request, &self.config, None, "update card")
            .await
            .map_err(Error::from_sdk)
    }
}

#[async_trait::async_trait]
impl AttachmentSource for Api {
    type Error = Error;

    async fn download(
        &self,
        file: &session::files::Incoming,
    ) -> Result<session::files::Downloaded> {
        let source: Resource = serde_json::from_str(&file.location)?;
        if !identifier(&source.message) || !identifier(&source.key) || source.image != file.image {
            return Err(Error::Ui("feishu.error.scope"));
        }
        let request = ApiRequest::<()>::get(format!(
            "/open-apis/im/v1/messages/{}/resources/{}",
            source.message, source.key
        ))
        .query("type", if source.image { "image" } else { "file" })
        .with_supported_access_token_types(vec![AccessTokenType::Tenant]);
        let bytes =
            Transport::<Vec<u8>>::request_typed(request, &self.config, None, "message resource")
                .await
                .map_err(Error::from_sdk)?;
        if bytes.len() > session::files::MAX_BYTES {
            return Err(Error::Ui("common.file.failed"));
        }
        Ok(session::files::Downloaded {
            name: session::files::name(&file.name),
            image: file.image,
            bytes,
        })
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
