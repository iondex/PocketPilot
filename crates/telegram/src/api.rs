use crate::{Error, Identity, safe_error};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use teloxide::{
    Bot,
    payloads::{
        CreateForumTopicSetters, EditForumTopicSetters, EditMessageTextSetters, GetUpdatesSetters,
        SendChatActionSetters, SendDocumentSetters, SendMessageSetters,
    },
    prelude::Requester,
    requests::Request,
    types::{
        AllowedUpdate, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile,
        LinkPreviewOptions, MessageId, ParseMode, ThreadId, Update, UpdateKind,
    },
};
use tokio::sync::Mutex;

#[path = "topic_icons.rs"]
mod topic_icons;

#[path = "command_menu.rs"]
mod command_menu;

/// Small persisted inputs; unsupported/media/group updates never become user actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub update_id: u32,
    #[serde(default)]
    pub locale: Option<String>,
    pub event_key: String,
    pub user: u64,
    pub chat: i64,
    pub topic: i32,
    pub date: i64,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<session::Attachment>,
    pub callback: Option<String>,
    pub callback_id: Option<String>,
    pub message_id: i32,
    /// Application snapshot captured when dispatching; absent on Telegram updates.
    #[serde(default)]
    pub config_hash: Option<String>,
    /// Local recovery marker, never populated from a Telegram message body.
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default)]
    pub reply_to_message_id: Option<i32>,
}

pub fn input(update: &Update) -> Option<Input> {
    let (message, user, text, callback, callback_id) = match &update.kind {
        UpdateKind::Message(m) => {
            let user = m.from.as_ref()?;
            if user.is_bot || m.sender_chat.is_some() {
                return None;
            }
            let body = m.text().or_else(|| m.caption()).unwrap_or("");
            if body.is_empty() && m.document().is_none() && m.photo().is_none() {
                return None;
            }
            (m, user, body.to_owned(), None, None)
        }
        UpdateKind::CallbackQuery(q) => {
            if q.from.is_bot {
                return None;
            }
            let m = q.message.as_ref()?.regular_message()?;
            (
                m,
                &q.from,
                String::new(),
                Some(q.data.clone()?),
                Some(q.id.to_string()),
            )
        }
        _ => return None,
    };
    if crate::access(
        user.id.0,
        message.chat.id.0,
        message.chat.is_private(),
        false,
    ) == crate::Access::Denied
    {
        return None;
    }
    Some(Input {
        update_id: update.id.0,
        locale: user.language_code.clone(),
        event_key: if callback.is_some() {
            format!("callback:{}", callback_id.as_deref()?)
        } else {
            format!("message:{}:{}", message.chat.id.0, message.id.0)
        },
        user: user.id.0,
        chat: message.chat.id.0,
        topic: message.thread_id.map_or(0, |t| t.0.0),
        date: message.date.timestamp(),
        text,
        attachments: if callback.is_none() {
            let mut files = Vec::new();
            if let Some(document) = message.document() {
                files.push(session::Attachment {
                    name: document
                        .file_name
                        .clone()
                        .unwrap_or_else(|| "attachment".into()),
                });
            }
            if message.photo().is_some() {
                files.push(session::Attachment {
                    name: "image".into(),
                });
            }
            files
        } else {
            Vec::new()
        },
        callback,
        callback_id,
        message_id: message.id.0,
        config_hash: None,
        requires_confirmation: false,
        reply_to_message_id: message.reply_to_message().map(|m| m.id.0),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Button {
    pub label: String,
    pub data: String,
}

#[derive(Clone)]
pub struct Api {
    bot: Bot,
    timeout: Duration,
    icons: Arc<Mutex<topic_icons::Cache>>,
}

impl Api {
    #[cfg(any(test, feature = "test-support"))]
    pub fn local_fixture(token: SecretString, endpoint: &str) -> Result<Self, Error> {
        let url = url::Url::parse(endpoint).map_err(|_| Error::Protocol)?;
        if url.scheme() != "http"
            || !url.host_str().is_some_and(|h| {
                h.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            })
        {
            return Err(Error::Protocol);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            bot: Bot::with_client(token.expose_secret(), client).set_api_url(url),
            timeout: Duration::from_secs(2),
            icons: Arc::default(),
        })
    }

    pub fn new(token: SecretString) -> Result<Self, Error> {
        Self::with_builder(token, reqwest::Client::builder())
    }

    fn with_builder(token: SecretString, builder: reqwest::ClientBuilder) -> Result<Self, Error> {
        // teloxide's default HTTP timeout is 17s, shorter than a 30s long poll.
        let client = builder
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(50))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            bot: Bot::with_client(token.expose_secret(), client),
            timeout: Duration::from_secs(50),
            icons: Arc::default(),
        })
    }

    pub async fn identity(&self) -> Result<Identity, Error> {
        let me = account(&self.bot, self.timeout).await?;
        let webhook = self.call(self.bot.get_webhook_info()).await?;
        Ok(Identity {
            bot_id: me.id,
            username: me.username,
            webhook_enabled: webhook.url.is_some(),
            topics_enabled: me.has_topics_enabled,
            user_topic_creation_enabled: me.allows_users_to_create_topics,
        })
    }

    pub async fn updates(&self, offset: Option<i32>) -> Result<Vec<Update>, Error> {
        let mut request = self
            .bot
            .get_updates()
            .timeout(30)
            .limit(100)
            .allowed_updates(vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery]);
        if let Some(offset) = offset {
            request = request.offset(offset);
        }
        self.call(request).await
    }

    pub async fn send(
        &self,
        chat: i64,
        topic: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
    ) -> Result<i32, Error> {
        self.send_formatted(chat, topic, text, buttons, false).await
    }

    pub async fn send_formatted(
        &self,
        chat: i64,
        topic: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
        markdown: bool,
    ) -> Result<i32, Error> {
        self.send_mode(
            chat,
            topic,
            text,
            buttons,
            if markdown {
                Some(ParseMode::MarkdownV2)
            } else {
                None
            },
        )
        .await
    }

    pub async fn send_outbound(
        &self,
        chat: i64,
        topic: i32,
        body: &crate::ledger::Outbound,
    ) -> Result<i32, Error> {
        if body.document {
            self.send_document(chat, topic, &body.text, body.buttons.clone())
                .await
        } else if body.agent_markdown {
            self.send_agent(chat, topic, &body.text, body.buttons.clone())
                .await
        } else {
            self.send_formatted(chat, topic, &body.text, body.buttons.clone(), body.markdown)
                .await
        }
    }

    pub async fn send_agent(
        &self,
        chat: i64,
        topic: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
    ) -> Result<i32, Error> {
        if text.len() > 45 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        if text.encode_utf16().count() > 3800 {
            return self.send_document(chat, topic, text, buttons).await;
        }
        let rendered = crate::markdown::render(text);
        if rendered.plain.encode_utf16().count() > 3800 || text.encode_utf16().count() > 3800 {
            return self.send_document(chat, topic, text, buttons).await;
        }
        match self
            .send_mode(
                chat,
                topic,
                &rendered.html,
                buttons.clone(),
                Some(ParseMode::Html),
            )
            .await
        {
            Err(Error::Formatting) => {
                self.send_mode(chat, topic, &rendered.plain, buttons, None)
                    .await
            }
            other => other,
        }
    }

    async fn send_document(
        &self,
        chat: i64,
        topic: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
    ) -> Result<i32, Error> {
        if text.len() > 45 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let markup = InlineKeyboardMarkup::new(buttons.into_iter().map(|r| {
            r.into_iter()
                .map(|b| InlineKeyboardButton::callback(b.label, b.data))
        }));
        let mut request = self
            .bot
            .send_document(
                ChatId(chat),
                InputFile::memory(text.as_bytes().to_vec()).file_name("session.md"),
            )
            .reply_markup(markup);
        if topic > 0 {
            request = request.message_thread_id(ThreadId(MessageId(topic)));
        }
        Ok(self.call(request).await?.id.0)
    }

    async fn send_mode(
        &self,
        chat: i64,
        topic: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
        mode: Option<ParseMode>,
    ) -> Result<i32, Error> {
        if text.len() > 45 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let markup = InlineKeyboardMarkup::new(buttons.into_iter().map(|row| {
            row.into_iter()
                .map(|b| InlineKeyboardButton::callback(b.label, b.data))
        }));
        if !matches!(mode, Some(ParseMode::Html)) && text.encode_utf16().count() > 3800 {
            let mut request = self
                .bot
                .send_document(
                    ChatId(chat),
                    InputFile::memory(text.as_bytes().to_vec()).file_name("session.md"),
                )
                .reply_markup(markup);
            if topic > 0 {
                request = request.message_thread_id(ThreadId(MessageId(topic)));
            }
            return Ok(self.call(request).await?.id.0);
        }
        let mut request = self
            .bot
            .send_message(ChatId(chat), text)
            .reply_markup(markup)
            .link_preview_options(no_preview());
        if let Some(mode) = mode {
            request = request.parse_mode(mode);
        }
        if topic > 0 {
            request = request.message_thread_id(ThreadId(MessageId(topic)));
        }
        let message = self.call(request).await?;
        tracing::info!(requested_topic=topic,returned_topic=?message.thread_id.map(|t|t.0.0),message_id=message.id.0,"Telegram send receipt");
        Ok(message.id.0)
    }

    pub async fn edit(&self, chat: i64, message: i32, text: &str) -> Result<(), Error> {
        self.edit_buttons(chat, message, text, Vec::new()).await
    }

    pub async fn edit_buttons(
        &self,
        chat: i64,
        message: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
    ) -> Result<(), Error> {
        self.edit_formatted(chat, message, text, buttons, false)
            .await
    }

    pub async fn edit_formatted(
        &self,
        chat: i64,
        message: i32,
        text: &str,
        buttons: Vec<Vec<Button>>,
        markdown: bool,
    ) -> Result<(), Error> {
        let markup = InlineKeyboardMarkup::new(buttons.into_iter().map(|row| {
            row.into_iter()
                .map(|b| InlineKeyboardButton::callback(b.label, b.data))
        }));
        let mut request = self
            .bot
            .edit_message_text(ChatId(chat), MessageId(message), text)
            .reply_markup(markup)
            .link_preview_options(no_preview());
        if markdown {
            request = request.parse_mode(ParseMode::MarkdownV2);
        }
        match self.call(request).await {
            Ok(_) | Err(Error::NotModified) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn create_topic(&self, chat: i64, title: &str) -> Result<i32, Error> {
        self.create_topic_with_icon(chat, title, "💬").await
    }

    pub async fn create_topic_with_icon(
        &self,
        chat: i64,
        title: &str,
        symbol: &str,
    ) -> Result<i32, Error> {
        let mut request = self.bot.create_forum_topic(ChatId(chat), title);
        match self.icon(symbol).await {
            Ok(icon) => request = request.icon_custom_emoji_id(icon),
            Err(Error::RetryAfter(seconds)) => return Err(Error::RetryAfter(seconds)),
            Err(error) => tracing::warn!(
                error_id = ::i18n::LocalizedError::message(&error).id,
                "topic creation will use default icon; state icon remains unconfirmed"
            ),
        }
        Ok(self.call(request).await?.thread_id.0.0)
    }

    pub async fn rename_topic(&self, chat: i64, topic: i32, title: &str) -> Result<(), Error> {
        match self
            .call(
                self.bot
                    .edit_forum_topic(ChatId(chat), ThreadId(MessageId(topic)))
                    .name(title),
            )
            .await
        {
            Ok(_) | Err(Error::NotModified) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// With both optional fields omitted, Telegram preserves the name and icon.
    pub async fn check_topic(&self, chat: i64, topic: i32) -> Result<(), Error> {
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.call(
                self.bot
                    .edit_forum_topic(ChatId(chat), ThreadId(MessageId(topic))),
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
        {
            Ok(_) | Err(Error::NotModified) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn delete_topic(&self, chat: i64, topic: i32) -> Result<(), Error> {
        self.call(
            self.bot
                .delete_forum_topic(ChatId(chat), ThreadId(MessageId(topic))),
        )
        .await?;
        Ok(())
    }

    pub async fn set_appearance(
        &self,
        chat: i64,
        topic: i32,
        appearance: &crate::presentation::Appearance,
        current_title: Option<&str>,
    ) -> Result<(), Error> {
        let mut request = self
            .bot
            .edit_forum_topic(ChatId(chat), ThreadId(MessageId(topic)));
        if current_title != Some(appearance.title.as_str()) {
            request = request.name(appearance.title.clone());
        }
        let icon = self.icon(&appearance.emoji).await?;
        request = request.icon_custom_emoji_id(icon);
        let started = tokio::time::Instant::now();
        match self.call(request).await {
            Ok(_) | Err(Error::NotModified) => {
                tracing::info!(
                    topic_id = topic,
                    emoji = appearance.emoji,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "Telegram topic icon confirmed"
                );
                Ok(())
            }
            Err(error) => {
                tracing::warn!(
                    topic_id = topic,
                    emoji = appearance.emoji,
                    error_id = ::i18n::LocalizedError::message(&error).id,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "Telegram topic icon update failed"
                );
                Err(error)
            }
        }
    }

    pub async fn typing(&self, chat: i64, topic: i32) -> Result<(), Error> {
        self.call(
            self.bot
                .send_chat_action(ChatId(chat), ChatAction::Typing)
                .message_thread_id(ThreadId(MessageId(topic))),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_status(&self, chat: i64, message: i32) -> Result<(), Error> {
        match self
            .call(self.bot.delete_message(ChatId(chat), MessageId(message)))
            .await
        {
            Ok(_) => Ok(()),
            Err(Error::MessageUnavailable | Error::Rejected) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn answer_callback(&self, id: &str) -> Result<(), Error> {
        self.call(
            self.bot
                .answer_callback_query(teloxide::types::CallbackQueryId(id.into())),
        )
        .await?;
        Ok(())
    }

    async fn call<R: Request<Err = teloxide::RequestError>>(
        &self,
        request: R,
    ) -> Result<<R::Payload as teloxide::requests::Payload>::Output, Error> {
        let started = tokio::time::Instant::now();
        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| Error::Timeout);
        let (result, reason) = match response {
            Ok(Ok(value)) => (Ok(value), "none"),
            Ok(Err(error)) => {
                let reason = crate::error_reason(&error);
                (Err(safe_error(error)), reason)
            }
            Err(error) => (Err(error), "deadline"),
        };
        if let Err(error) = &result
            && !matches!(error, Error::NotModified)
        {
            tracing::warn!(
                method = <R::Payload as teloxide::requests::Payload>::NAME,
                error_id = ::i18n::LocalizedError::message(error).id,
                reason,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "Telegram API request failed"
            );
        }
        result
    }
}

fn no_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

pub fn escape_markdown(text: &str) -> String {
    teloxide::utils::markdown::escape(text)
}

// teloxide 0.17 predates the private-topic getMe flag. Extend this one response
// with a typed DTO; all message/topic requests retain the SDK's typed API.
#[derive(Deserialize)]
pub(crate) struct Account {
    pub id: u64,
    pub username: Option<String>,
    #[serde(default)]
    pub has_topics_enabled: bool,
    #[serde(default)]
    pub allows_users_to_create_topics: bool,
}

#[derive(Deserialize)]
struct AccountResponse {
    ok: bool,
    result: Option<Account>,
    error_code: Option<u16>,
}

pub(crate) async fn account(bot: &Bot, timeout: Duration) -> Result<Account, Error> {
    let work = async {
        let mut url = bot.api_url();
        url.path_segments_mut()
            .map_err(|_| Error::Protocol)?
            .push(&format!("bot{}", bot.token()))
            .push("getMe");
        let mut response = bot
            .client()
            .post(url)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(Error::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        let response: AccountResponse =
            serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?;
        if !response.ok {
            return Err(if response.error_code == Some(401) {
                Error::Authentication
            } else {
                Error::Rejected
            });
        }
        response.result.ok_or(Error::Protocol)
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| Error::Timeout)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn icon_catalog_failure_is_not_cached_or_reported_as_applied() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let calls = attempts.clone();
        Mock::given(path_regex("(?i).*/getforumtopiciconstickers$"))
            .respond_with(move |_: &wiremock::Request| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200).set_body_string(r#"{"ok":false,"error_code":500,"description":"Internal Server Error"}"#)
                } else {
                    ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[{"file_id":"f","file_unique_id":"u","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"💬","custom_emoji_id":"idle"}]}"#)
                }
            }).mount(&server).await;
        Mock::given(path_regex("(?i).*/editforumtopic$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
        let appearance = crate::presentation::Appearance {
            title: "Synthetic title".into(),
            emoji: "💬".into(),
        };
        assert!(
            api.set_appearance(10, 11, &appearance, Some(&appearance.title))
                .await
                .is_err(),
            "an unavailable icon must not produce a success receipt"
        );
        assert!(matches!(
            api.set_appearance(10, 11, &appearance, Some(&appearance.title))
                .await,
            Err(Error::IconUnavailable(_))
        ));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "catalog failure must back off"
        );
        api.icons.lock().await.retry_at = None;
        api.set_appearance(10, 11, &appearance, Some(&appearance.title))
            .await
            .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        #[derive(Deserialize)]
        struct Edit {
            icon_custom_emoji_id: Option<String>,
        }
        let requests = server.received_requests().await.unwrap();
        let edit = requests
            .iter()
            .find(|r| r.url.path().to_lowercase().ends_with("/editforumtopic"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Edit>(&edit.body)
                .unwrap()
                .icon_custom_emoji_id
                .as_deref(),
            Some("idle")
        );
    }

    #[tokio::test]
    async fn missing_icons_and_catalog_rate_limits_never_send_an_empty_icon_edit() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
        for response in [
            r#"{"ok":true,"result":[{"file_id":"f","file_unique_id":"u","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"📁","custom_emoji_id":"folder"}]}"#,
            r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 20","parameters":{"retry_after":20}}"#,
        ] {
            let server = MockServer::start().await;
            Mock::given(path_regex("(?i).*/getforumtopiciconstickers$"))
                .respond_with(ResponseTemplate::new(200).set_body_string(response))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path_regex("(?i).*/editforumtopic$"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#),
                )
                .expect(0)
                .mount(&server)
                .await;
            let api =
                Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
            let appearance = crate::presentation::Appearance {
                title: "Synthetic title".into(),
                emoji: "💬".into(),
            };
            for _ in 0..2 {
                let result = api
                    .set_appearance(10, 11, &appearance, Some(&appearance.title))
                    .await;
                if response.contains("429") {
                    assert!(matches!(result, Err(Error::RetryAfter(seconds)) if seconds >= 20));
                } else {
                    assert!(matches!(result, Err(Error::IconUnavailable(_))));
                }
            }
        }
    }

    #[tokio::test]
    async fn production_poll_budget_survives_a_response_slower_than_the_sdk_default() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
        let server = MockServer::start().await;
        Mock::given(path_regex("(?i).*/getupdates$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"ok":true,"result":[]}"#)
                    .set_delay(Duration::from_secs(18)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = Api::with_builder(
            "1:abcdefghijklmnopqrstuvwxyz".into(),
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap();
        api.bot = api.bot.set_api_url(url::Url::parse(&server.uri()).unwrap());
        assert!(api.updates(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn identity_extension_keeps_the_token_in_the_url_path_and_reads_private_topics() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
        let server = MockServer::start().await;
        Mock::given(path_regex("(?i).*/getme$"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"ok":true,"result":{"id":1,"username":"fixture","has_topics_enabled":true,"allows_users_to_create_topics":true}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path_regex("(?i).*/getwebhookinfo$")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{"url":"","has_custom_certificate":false,"pending_update_count":0}}"#)).expect(1).mount(&server).await;
        let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
        let identity = api.identity().await.unwrap();
        assert!(identity.topics_enabled);
        assert!(identity.user_topic_creation_enabled);
        assert!(!identity.webhook_enabled);
        assert_eq!(identity.bot_id, 1);
    }

    #[test]
    fn sdk_preserves_private_topic_and_denies_group_sender() {
        let private = r#"{"update_id":1,"message":{"message_id":7,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"User"},"chat":{"id":10,"type":"private","first_name":"User"},"text":"hello","is_topic_message":true}}"#;
        let update: Update = serde_json::from_str(private).unwrap();
        let event = input(&update).unwrap();
        assert_eq!(event.topic, 11);
        assert_eq!(event.event_key, "message:10:7");
        let group = private.replace(
            "\"id\":10,\"type\":\"private\",\"first_name\":\"User\"",
            "\"id\":-100,\"type\":\"supergroup\",\"title\":\"Group\"",
        );
        assert!(input(&serde_json::from_str(&group).unwrap()).is_none());
    }

    #[test]
    fn media_captions_are_text_and_images_never_become_urls_or_downloads() {
        let update = r#"{"update_id":1,"message":{"message_id":7,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"User"},"chat":{"id":10,"type":"private","first_name":"User"},"photo":[{"file_id":"private-file","file_unique_id":"unique","width":640,"height":480}],"caption":"read this caption"}}"#;
        let event = input(&serde_json::from_str(update).unwrap()).unwrap();
        assert_eq!(event.text, "read this caption");
        assert_eq!(event.topic, 11);
        assert_eq!(event.attachments[0].name, "image");
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("private-file")
        );
        let photo_only = update.replace(",\"caption\":\"read this caption\"", "");
        assert!(
            input(&serde_json::from_str(&photo_only).unwrap())
                .unwrap()
                .text
                .is_empty()
        );
        assert!(no_preview().is_disabled);
    }
}
