//! Socket envelopes use SDK payload DTOs. The SDK's envelope module is private.
use serde::{Deserialize, Serialize};
use slack_morphism::prelude::*;

#[derive(Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    #[serde(rename = "hello")]
    Hello { connection_info: ConnectionInfo },
    #[serde(rename = "disconnect")]
    Disconnect,
    #[serde(rename = "events_api")]
    EventsApi {
        #[serde(flatten)]
        envelope: Envelope,
        payload: SlackPushEventCallback,
    },
    #[serde(rename = "interactive")]
    Interactive {
        #[serde(flatten)]
        envelope: Envelope,
        payload: SlackInteractionEvent,
    },
}

#[derive(Deserialize)]
pub struct ConnectionInfo {
    pub app_id: SlackAppId,
}

#[derive(Deserialize)]
pub struct Envelope {
    pub envelope_id: String,
    #[serde(default)]
    pub accepts_response_payload: bool,
}

#[derive(Serialize)]
pub struct Ack<T = ()> {
    pub envelope_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<T>,
}

impl Ack<()> {
    pub fn bare(envelope_id: String) -> Self {
        Self {
            envelope_id,
            payload: None,
        }
    }

    pub fn unknown(text: &str) -> Result<Self, serde_json::Error> {
        let envelope: Envelope = serde_json::from_str(text)?;
        Ok(Self::bare(envelope.envelope_id))
    }
}

impl<T> Ack<T> {
    pub fn new(envelope: Envelope, payload: T) -> Self {
        Self {
            envelope_id: envelope.envelope_id,
            payload: envelope.accepts_response_payload.then_some(payload),
        }
    }
}
