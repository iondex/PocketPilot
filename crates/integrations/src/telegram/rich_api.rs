//! RichMessage endpoints not yet exposed by the pinned teloxide version.
use super::{Api, Button};
use crate::telegram::{Error, rich};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

#[derive(Serialize)]
struct Send<'a> {
    chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<i32>,
    rich_message: &'a rich::Message,
    reply_markup: InlineKeyboardMarkup,
}

#[derive(Serialize)]
struct Edit<'a> {
    chat_id: i64,
    message_id: i32,
    rich_message: &'a rich::Message,
    reply_markup: InlineKeyboardMarkup,
}

#[derive(Deserialize)]
struct Reply<T> {
    ok: bool,
    result: Option<T>,
    parameters: Option<Parameters>,
}

#[derive(Deserialize)]
struct Parameters {
    retry_after: Option<u64>,
}

#[derive(Deserialize)]
struct Receipt {
    message_id: i32,
}

fn markup(buttons: Vec<Vec<Button>>) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(buttons.into_iter().map(|row| {
        row.into_iter()
            .map(|button| InlineKeyboardButton::callback(button.label, button.data))
    }))
}

impl Api {
    pub async fn send_rich(
        &self,
        chat: i64,
        topic: i32,
        message: &rich::Message,
        buttons: Vec<Vec<Button>>,
    ) -> Result<i32, Error> {
        let receipt: Receipt = self
            .rich_request(
                "sendRichMessage",
                &Send {
                    chat_id: chat,
                    message_thread_id: (topic > 0).then_some(topic),
                    rich_message: message,
                    reply_markup: markup(buttons),
                },
            )
            .await?;
        Ok(receipt.message_id)
    }

    pub async fn edit_rich(
        &self,
        chat: i64,
        message_id: i32,
        message: &rich::Message,
        buttons: Vec<Vec<Button>>,
    ) -> Result<(), Error> {
        let _: Receipt = self
            .rich_request(
                "editMessageText",
                &Edit {
                    chat_id: chat,
                    message_id,
                    rich_message: message,
                    reply_markup: markup(buttons),
                },
            )
            .await?;
        Ok(())
    }

    async fn rich_request<T: DeserializeOwned>(
        &self,
        method: &'static str,
        payload: &impl Serialize,
    ) -> Result<T, Error> {
        let url = format!(
            "{}/bot{}/{method}",
            self.bot.api_url().as_str().trim_end_matches('/'),
            self.bot.token()
        );
        let mut response = self
            .bot
            .client()
            .post(url)
            .json(payload)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout
                } else {
                    Error::Network
                }
            })?;
        // Decode only a small receipt, never retain or log returned message contents or API URLs.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
            if bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
                return Err(Error::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        let reply: Reply<T> = serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?;
        if let Some(delay) = reply
            .parameters
            .and_then(|parameters| parameters.retry_after)
        {
            return Err(Error::RetryAfter(delay.clamp(1, 3600)));
        }
        if !reply.ok {
            return Err(Error::Request);
        }
        reply.result.ok_or(Error::Protocol)
    }
}
