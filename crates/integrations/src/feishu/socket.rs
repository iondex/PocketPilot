//! SDK callbacks acknowledge only after a scoped input is durably enqueued.
use super::{
    Error, Result,
    config::identifier,
    model::{Action, Command, Input, Resource},
    runtime::Service,
};
use openlark_client::ws_client::{
    CallbackEventHandler, EventDispatcherHandler, EventHandler, LarkWsClient,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

const MAX_EVENT: usize = 512 * 1024;

type CallbackError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Deserialize)]
struct Envelope {
    header: Header,
    event: Box<serde_json::value::RawValue>,
}

#[derive(Deserialize)]
struct Header {
    event_id: String,
    event_type: String,
    app_id: String,
    tenant_key: String,
    create_time: String,
}

#[derive(Deserialize)]
struct MessageEvent {
    sender: Sender,
    message: Message,
}

#[derive(Deserialize)]
struct Sender {
    sender_id: SenderId,
    sender_type: String,
    tenant_key: String,
}

#[derive(Deserialize)]
struct SenderId {
    open_id: String,
}

#[derive(Deserialize)]
struct Message {
    message_id: String,
    chat_id: String,
    chat_type: String,
    message_type: String,
    content: String,
    create_time: String,
    #[serde(default)]
    root_id: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    mentions: Vec<Mention>,
}

#[derive(Deserialize)]
struct Mention {
    key: String,
    id: SenderId,
}

#[derive(Deserialize)]
struct TextContent {
    text: String,
}

#[derive(Deserialize)]
struct FileContent {
    file_key: String,
    file_name: String,
}

#[derive(Deserialize)]
struct ImageContent {
    image_key: String,
}

#[derive(Deserialize)]
struct PostContent {
    #[serde(default)]
    title: String,
    content: Vec<Vec<PostNode>>,
}

#[derive(Deserialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
enum PostNode {
    Text {
        text: String,
    },
    A {
        text: String,
        href: String,
    },
    At {
        user_id: String,
    },
    Img {
        image_key: String,
    },
    Md {
        text: String,
    },
    CodeBlock {
        text: String,
    },
    #[serde(other)]
    Other,
}

fn post(
    content: &str,
    message: &str,
    mentions: &[Mention],
    bot: &str,
) -> Result<Option<(String, Vec<session::files::Incoming>, bool)>> {
    let content: PostContent = serde_json::from_str(content)?;
    let mut lines = vec![content.title];
    let mut files = Vec::new();
    let mut mentioned = false;
    for row in content.content {
        let mut text = String::new();
        for node in row {
            match node {
                PostNode::Text { text: value }
                | PostNode::Md { text: value }
                | PostNode::CodeBlock { text: value } => text.push_str(&value),
                PostNode::A { text: value, href } => {
                    text.push_str(&value);
                    if let Some(link) = session::markdown::safe_link(&href) {
                        text.push_str(&format!(" ({link})"));
                    }
                }
                PostNode::At { user_id } => {
                    if mentions
                        .iter()
                        .any(|m| m.key == user_id && m.id.open_id == bot)
                    {
                        mentioned = true;
                    } else {
                        text.push_str(&user_id);
                    }
                }
                PostNode::Img { image_key } if identifier(&image_key) => {
                    files.push(session::files::Incoming {
                        name: "image".into(),
                        location: serde_json::to_string(&Resource {
                            message: message.into(),
                            key: image_key,
                            image: true,
                        })?,
                        image: true,
                    });
                    if files.len() > session::files::MAX_FILES {
                        return Ok(None);
                    }
                }
                _ => return Ok(None),
            }
        }
        lines.push(text);
    }
    Ok(Some((lines.join("\n"), files, mentioned)))
}

#[derive(Deserialize)]
struct CardEvent {
    operator: Operator,
    context: CardContext,
    action: CardAction,
}

#[derive(Deserialize)]
struct Operator {
    open_id: String,
    tenant_key: String,
}

#[derive(Deserialize)]
struct CardContext {
    open_message_id: String,
    open_chat_id: String,
}

#[derive(Deserialize)]
struct CardAction {
    value: super::render::ActionValue,
    #[serde(default)]
    form_value: Option<Form>,
}

#[derive(Default, Deserialize)]
struct Form {
    title: Option<String>,
}

#[derive(Serialize)]
struct ToastResponse {
    toast: Toast,
}

#[derive(Serialize)]
struct Toast {
    #[serde(rename = "type")]
    kind: &'static str,
    content: String,
}

pub(super) struct Handler {
    pub service: Arc<Service>,
    pub handle: tokio::runtime::Handle,
}

impl Handler {
    fn dispatch(&self, payload: &[u8]) -> std::result::Result<bool, CallbackError> {
        let result = self.handle.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), self.service.receive(payload)).await
        });
        result.map_err(|_| Error::Request)?.map_err(Into::into)
    }
}

impl EventHandler for Handler {
    fn handle(&self, payload: &[u8]) -> std::result::Result<(), CallbackError> {
        self.dispatch(payload).map(|_| ())
    }
}

impl CallbackEventHandler for Handler {
    fn handle(
        &self,
        payload: &[u8],
    ) -> std::result::Result<Option<serde_json::Value>, CallbackError> {
        let received = self.dispatch(payload)?;
        // Value is confined to this SDK boundary; the response is a named DTO.
        Ok(Some(serde_json::to_value(ToastResponse {
            toast: Toast {
                kind: "info",
                content: self.service.text(if received {
                    "feishu.received"
                } else {
                    "feishu.expired"
                }),
            },
        })?))
    }
}

fn seconds(raw: &str) -> Option<i64> {
    let number = raw.parse::<i64>().ok()?;
    Some(if number > 100_000_000_000_000 {
        number / 1_000_000
    } else if number > 100_000_000_000 {
        number / 1000
    } else {
        number
    })
}

impl Service {
    pub(super) async fn receive(&self, payload: &[u8]) -> Result<bool> {
        if payload.len() > MAX_EVENT {
            return Err(Error::Ui("feishu.error.large"));
        }
        let envelope: Envelope = serde_json::from_slice(payload)?;
        let h = &envelope.header;
        if h.app_id != self.settings.app_id
            || h.tenant_key != self.settings.tenant_key
            || !identifier(&h.event_id)
        {
            return Ok(false);
        }
        let Some(date) = seconds(&h.create_time) else {
            return Ok(false);
        };
        if !self.date_valid(date) {
            return Ok(false);
        }
        let snapshot = self.context.config.snapshot();
        match h.event_type.as_str() {
            "im.message.receive_v1" => {
                let event: MessageEvent = serde_json::from_str(envelope.event.get())?;
                if event.sender.sender_type != "user"
                    || !self
                        .settings
                        .permits(&event.sender.tenant_key, &event.sender.sender_id.open_id)
                {
                    return Ok(false);
                }
                let m = event.message;
                if !identifier(&m.message_id)
                    || !identifier(&m.chat_id)
                    || m.content.len() > 256 * 1024
                {
                    return Ok(false);
                }
                let Some(date) = seconds(&m.create_time) else {
                    return Ok(false);
                };
                if !self.date_valid(date) {
                    return Ok(false);
                }
                let direct = m.chat_type == "p2p";
                if !direct && (m.chat_type != "group" || !self.settings.group(&m.chat_id)) {
                    return Ok(false);
                }
                let mut input = Input {
                    id: format!("message:{}", m.message_id),
                    chat: m.chat_id,
                    message: m.message_id.clone(),
                    root: m.root_id.filter(|v| !v.is_empty()),
                    thread: m.thread_id.filter(|v| !v.is_empty()),
                    date,
                    command: Command::Menu(Action::Home { offset: 0 }),
                    files: vec![],
                };
                if direct {
                    // A private chat is the directory, never an implicit current session.
                    return self.store.enqueue(&input, &snapshot.hash).await;
                }
                if input.root.as_deref().is_none_or(|v| !identifier(v))
                    || input.thread.as_deref().is_none_or(|v| !identifier(v))
                {
                    return Ok(false);
                }
                // Files have no caption/mention field in some clients. Only the configured
                // owner in an already bound group thread may submit those independently.
                match m.message_type.as_str() {
                    "text" => {
                        let mut content: TextContent = serde_json::from_str(&m.content)?;
                        let mut mentioned = false;
                        for mention in m.mentions.iter().filter(|m| m.id.open_id == self.bot) {
                            mentioned = true;
                            content.text = content.text.replace(&mention.key, "");
                        }
                        if !mentioned {
                            return Ok(false);
                        }
                        input.command = Command::Text(content.text.trim().into());
                    }
                    "post" => {
                        if !m.mentions.iter().any(|m| m.id.open_id == self.bot) {
                            return Ok(false);
                        }
                        match post(&m.content, &m.message_id, &m.mentions, &self.bot)? {
                            Some((text, files, mentioned)) => {
                                if !mentioned {
                                    return Ok(false);
                                }
                                input.command = Command::Text(text.trim().into());
                                input.files = files;
                            }
                            None => input.command = Command::Unsupported,
                        }
                    }
                    "file" => {
                        let file: FileContent = serde_json::from_str(&m.content)?;
                        if !identifier(&file.file_key) {
                            return Ok(false);
                        }
                        input.files.push(session::files::Incoming {
                            name: session::files::name(&file.file_name),
                            location: serde_json::to_string(&Resource {
                                message: m.message_id,
                                key: file.file_key,
                                image: false,
                            })?,
                            image: false,
                        });
                        input.command = Command::Text(String::new());
                    }
                    "image" => {
                        let file: ImageContent = serde_json::from_str(&m.content)?;
                        if !identifier(&file.image_key) {
                            return Ok(false);
                        }
                        input.files.push(session::files::Incoming {
                            name: "image".into(),
                            location: serde_json::to_string(&Resource {
                                message: m.message_id,
                                key: file.image_key,
                                image: true,
                            })?,
                            image: true,
                        });
                        input.command = Command::Text(String::new());
                    }
                    _ => return Ok(false),
                }
                match self
                    .store
                    .bound(
                        &input.chat,
                        input.thread.as_deref().unwrap_or_default(),
                        input.root.as_deref().unwrap_or_default(),
                    )
                    .await
                {
                    Ok(_) => {}
                    Err(Error::Ui("feishu.error.thread")) => return Ok(false),
                    Err(error) => return Err(error),
                }
                self.store.enqueue(&input, &snapshot.hash).await
            }
            "card.action.trigger" => {
                let event: CardEvent = serde_json::from_str(envelope.event.get())?;
                if !self
                    .settings
                    .permits(&event.operator.tenant_key, &event.operator.open_id)
                    || !identifier(&event.context.open_message_id)
                    || !identifier(&event.context.open_chat_id)
                {
                    return Ok(false);
                }
                let input = Input {
                    id: format!("card:{}", h.event_id),
                    chat: event.context.open_chat_id,
                    message: event.context.open_message_id.clone(),
                    root: None,
                    thread: None,
                    date,
                    command: Command::Menu(Action::Home { offset: 0 }),
                    files: vec![],
                };
                self.store
                    .callback(
                        &event.action.value.nonce,
                        &event.context.open_message_id,
                        &input,
                        &snapshot.hash,
                        event.action.form_value.and_then(|f| f.title),
                    )
                    .await
            }
            _ => Ok(false),
        }
    }
}

pub(super) async fn run(service: Arc<Service>, api: Arc<super::api::Api>) -> Result<()> {
    let mut stop = service.stop.clone();
    let mut failures = 0u32;
    loop {
        let handler = || Handler {
            service: service.clone(),
            handle: tokio::runtime::Handle::current(),
        };
        let dispatcher = EventDispatcherHandler::builder()
            .register_raw("im.message.receive_v1", handler())
            .map_err(|_| Error::Request)?
            .register_callback("card.action.trigger", handler())
            .map_err(|_| Error::Request)?
            .build();
        let connected = tokio::time::Instant::now();
        // The SDK dials directly; no relay or environment proxy is involved.
        let result = tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = LarkWsClient::open(api.config.clone(), dispatcher) => result,
        };
        if *stop.borrow() {
            return Ok(());
        }
        service
            .cutoff
            .fetch_max(engine::now().saturating_sub(300), Ordering::SeqCst);
        failures = if connected.elapsed() > Duration::from_secs(120) {
            0
        } else {
            failures.saturating_add(1)
        };
        let seconds = (1u64 << failures.min(6)).min(60);
        let _ = result;
        tracing::warn!(
            platform = service.region.as_str(),
            retry_seconds = seconds,
            "Chat connection closed; authorized writes will not be replayed"
        );
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => {},
        }
    }
}
