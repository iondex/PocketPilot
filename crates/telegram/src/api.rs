use crate::{Error, Identity, safe_error};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teloxide::{
    Bot,
    payloads::{
        EditForumTopicSetters, EditMessageTextSetters, GetUpdatesSetters, SendDocumentSetters,
        SendMessageSetters,
    },
    prelude::Requester,
    requests::Request,
    types::{
        AllowedUpdate, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId,
        ThreadId, Update, UpdateKind,
    },
};

/// Small persisted inputs; unsupported/media/group updates never become user actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub update_id: u32,
    pub event_key: String,
    pub user: u64,
    pub chat: i64,
    pub topic: i32,
    pub date: i64,
    pub text: String,
    pub callback: Option<String>,
    pub callback_id: Option<String>,
    pub message_id: i32,
    /// Application snapshot captured when dispatching; absent on Telegram updates.
    #[serde(default)]
    pub config_hash: Option<String>,
}

pub fn input(update: &Update) -> Option<Input> {
    let (message, user, text, callback, callback_id) = match &update.kind {
        UpdateKind::Message(m) => {
            let user = m.from.as_ref()?;
            if user.is_bot || m.sender_chat.is_some() {
                return None;
            }
            (m, user, m.text()?.to_owned(), None, None)
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
        callback,
        callback_id,
        message_id: message.id.0,
        config_hash: None,
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
        if text.len() > 45 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let markup = InlineKeyboardMarkup::new(buttons.into_iter().map(|row| {
            row.into_iter()
                .map(|b| InlineKeyboardButton::callback(b.label, b.data))
        }));
        if text.encode_utf16().count() > 3800 {
            let mut request = self.bot.send_document(
                ChatId(chat),
                InputFile::memory(text.as_bytes().to_vec()).file_name("session.md"),
            );
            if topic > 0 {
                request = request.message_thread_id(ThreadId(MessageId(topic)));
            }
            return Ok(self.call(request).await?.id.0);
        }
        let mut request = self
            .bot
            .send_message(ChatId(chat), text)
            .reply_markup(markup);
        if topic > 0 {
            request = request.message_thread_id(ThreadId(MessageId(topic)));
        }
        Ok(self.call(request).await?.id.0)
    }
    pub async fn edit(&self, chat: i64, message: i32, text: &str) -> Result<(), Error> {
        match self
            .call(
                self.bot
                    .edit_message_text(ChatId(chat), MessageId(message), text)
                    .reply_markup(InlineKeyboardMarkup::default()),
            )
            .await
        {
            Ok(_) | Err(Error::NotModified) => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub async fn create_topic(&self, chat: i64, title: &str) -> Result<i32, Error> {
        Ok(self
            .call(self.bot.create_forum_topic(ChatId(chat), title))
            .await?
            .thread_id
            .0
            .0)
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
        tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(safe_error)
    }
}

// teloxide 0.17 predates the private-topic getMe flag. Extend this one response
// with a typed DTO; all message/topic requests retain the SDK's typed API.
#[derive(Deserialize)]
pub(crate) struct Account {
    pub id: u64,
    pub username: Option<String>,
    #[serde(default)]
    pub has_topics_enabled: bool,
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
                r#"{"ok":true,"result":{"id":1,"username":"fixture","has_topics_enabled":true}}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path_regex("(?i).*/getwebhookinfo$")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{"url":"","has_custom_certificate":false,"pending_update_count":0}}"#)).expect(1).mount(&server).await;
        let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
        let identity = api.identity().await.unwrap();
        assert!(identity.topics_enabled);
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
}
