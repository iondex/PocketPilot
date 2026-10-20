use crate::slack::api::Client;
use crate::slack::socket_model::{Ack, Event};
use crate::slack::{Error, Result, api::Api, model::*, runtime::Service};
use futures_util::{SinkExt, StreamExt};
use slack_morphism::{UserCallbackResult, listener::SlackClientEventsUserState, prelude::*};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::watch;

async fn state(states: SlackClientEventsUserState) -> Result<Arc<Service>> {
    states
        .read()
        .await
        .get_user_state::<Arc<Service>>()
        .cloned()
        .ok_or(Error::Request)
}

pub(crate) fn message(
    event: SlackPushEventCallback,
    settings: &crate::slack::config::Config,
    bot: &str,
    direct_channel: &str,
) -> Option<Request> {
    if event.api_app_id.to_string() != settings.app_id
        || event.team_id.to_string() != settings.team_id
    {
        return None;
    }
    let SlackEventCallbackBody::Message(m) = event.event else {
        return None;
    };
    if m.hidden == Some(true)
        || m.sender.bot_id.is_some()
        || !matches!(m.subtype, None | Some(SlackMessageEventType::FileShare))
    {
        return None;
    }
    let user = m.sender.user?.to_string();
    let channel = m.origin.channel?.to_string();
    if user == bot
        || channel != direct_channel
        || m.origin
            .channel_type
            .as_ref()
            .map(|kind| kind.value().as_str())
            != Some("im")
        || !settings.permits(&settings.team_id, &user)
    {
        return None;
    }
    let root = m.origin.thread_ts.map(|root| root.to_string());
    let content = m.content?;
    let uploads = content
        .files
        .unwrap_or_default()
        .into_iter()
        .map(|file| session::files::Incoming {
            name: file.name.unwrap_or_else(|| "file".into()),
            location: file.id.to_string(),
            image: file
                .mimetype
                .as_ref()
                .is_some_and(|m| session::files::image_mime(m.value())),
        })
        .collect::<Vec<_>>();
    let mut text = content.text.unwrap_or_default();
    if text.trim().is_empty() && uploads.is_empty() {
        return None;
    }
    if text.trim().is_empty() {
        text = i18n::text("zh-CN", "common.file.request");
    }
    let date = m.origin.ts.value().split('.').next()?.parse().ok()?;
    Some(Request {
        uploads,
        menu_message: None,
        id: format!("message:{channel}:{}", m.origin.ts),
        channel,
        user,
        command: if root.is_some() {
            Command::Input { text }
        } else {
            Command::Menu
        },
        root,
        binding: None,
        revision: None,
        date,
    })
}

async fn push(
    event: SlackPushEventCallback,
    _client: Arc<Client>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let service = state(states).await?;
    let source = format!("event:{}", event.event_id);
    if let Some(r) = message(
        event,
        &service.settings,
        &service.bot,
        &service.direct_channel,
    ) {
        tokio::time::timeout(Duration::from_secs(2), service.intake(r, &source))
            .await
            .map_err(|_| Error::Request)??;
    }
    Ok(())
}

async fn interaction(
    event: SlackInteractionEvent,
    client: Arc<Client>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<SlackInteractionResponse> {
    let service = state(states).await?;
    tokio::time::timeout(
        Duration::from_millis(2400),
        handle_interaction(&service, &client, event),
    )
    .await
    .map_err(|_| Error::Request)?
    .map_err(Into::into)
}

fn action_nonce(action: &SlackInteractionActionInfo) -> Option<&str> {
    let id = action.action_id.value();
    let value = action.value.as_deref().or_else(|| {
        action
            .selected_option
            .as_ref()
            .map(|option| option.value.as_str())
    })?;
    if id == "pp" || id.strip_prefix("pp:") == Some(value) {
        Some(value)
    } else {
        None
    }
}

async fn handle_interaction(
    service: &Service,
    client: &Client,
    event: SlackInteractionEvent,
) -> Result<SlackInteractionResponse> {
    match event {
        SlackInteractionEvent::BlockActions(event) => {
            if event.team.id.to_string() != service.settings.team_id
                || event.api_app_id.to_string() != service.settings.app_id
            {
                return Ok(SlackInteractionResponse::Empty);
            }
            let Some(user) = event.user else {
                return Ok(SlackInteractionResponse::Empty);
            };
            let Some(channel) = event.channel else {
                return Ok(SlackInteractionResponse::Empty);
            };
            let Some(actions) = event.actions else {
                return Ok(SlackInteractionResponse::Empty);
            };
            let Some(action) = actions.first() else {
                return Ok(SlackInteractionResponse::Empty);
            };
            let Some(nonce) = action_nonce(action) else {
                return Ok(SlackInteractionResponse::Empty);
            };
            if let Some((r, action)) = service
                .click(
                    nonce,
                    Some(channel.id.value()),
                    user.id.value(),
                    None,
                    InteractionSource {
                        id: Some(event.trigger_id.value()),
                        message: match &event.container {
                            SlackInteractionActionContainer::Message(container) => {
                                Some(container.message_ts.value().as_str())
                            }
                            _ => None,
                        },
                    },
                )
                .await?
            {
                match action {
                    Action::Run(_) => {}
                    Action::NewModal { workspace } => {
                        let hash = service.context.config.snapshot().hash.clone();
                        let nonce = service
                            .store
                            .action(&r, Action::NewSubmit { workspace }, &hash)
                            .await?;
                        let input = SlackInputBlock::new(
                            pt!(i18n::text(&service.locale(), "slack.title")),
                            SlackBlockPlainTextInputElement::new()
                                .with_action_id("title".into())
                                .with_max_length(120)
                                .into(),
                        )
                        .with_block_id("title".into());
                        let view = SlackModalView::new(
                            pt!(i18n::text(&service.locale(), "slack.new")),
                            vec![input.into()],
                        )
                        .with_submit(pt!(i18n::text(&service.locale(), "slack.create")))
                        .with_callback_id("pp-new".into())
                        .with_private_metadata(nonce);
                        let token = SlackApiToken::new(service.settings.bot_token.clone().into())
                            .with_team_id(service.settings.team_id.clone().into());
                        client
                            .open_session(&token)
                            .views_open(&SlackApiViewsOpenRequest::new(
                                event.trigger_id,
                                SlackView::Modal(view),
                            ))
                            .await
                            .map_err(crate::slack::api::map_error)?;
                    }
                    _ => {}
                }
            }
        }
        SlackInteractionEvent::ViewSubmission(event) => {
            if event.team.id.to_string() != service.settings.team_id {
                return Ok(SlackInteractionResponse::Empty);
            }
            let SlackView::Modal(view) = event.view.view else {
                return Ok(SlackInteractionResponse::Empty);
            };
            if view
                .callback_id
                .as_ref()
                .map(ToString::to_string)
                .as_deref()
                != Some("pp-new")
            {
                return Ok(SlackInteractionResponse::Empty);
            }
            let Some(nonce) = view.private_metadata else {
                return Ok(SlackInteractionResponse::Empty);
            };
            let title = event
                .view
                .state_params
                .state
                .as_ref()
                .and_then(|s| s.values.get(&SlackBlockId("title".into())))
                .and_then(|b| b.get(&SlackActionId("title".into())))
                .and_then(|a| a.value.clone())
                .unwrap_or_default();
            if title.trim().is_empty() || title.chars().count() > 120 {
                return Ok(SlackViewSubmissionResponse::Errors(
                    SlackViewSubmissionErrorsResponse::new(std::collections::HashMap::from([(
                        "title".into(),
                        i18n::text(&service.locale(), "slack.error.title"),
                    )])),
                )
                .into());
            }
            if let Some((r, Action::NewSubmit { workspace })) = service
                .click(
                    &nonce,
                    None,
                    event.user.id.value(),
                    Some(&title),
                    InteractionSource::default(),
                )
                .await?
            {
                let _ = (r, workspace);
            }
        }
        _ => {}
    }
    Ok(SlackInteractionResponse::Empty)
}

/// Socket transport uses the same explicit proxy as the API connector. All envelopes
/// carry SDK payload types; acknowledgements follow durable intake, including duplicates.
pub async fn run(
    service: Arc<Service>,
    api: Arc<Api>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let states =
        slack_morphism::listener::SlackClientEventsListenerEnvironment::new(api.client.clone())
            .with_user_state(service.clone())
            .user_state;
    let token = SlackApiToken::new(service.settings.app_token.clone().into());
    let mut backoff = 2;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        service.verified.store(false, Ordering::SeqCst);
        let connection = async {
            let opened = crate::slack::api::bounded(
                api.client
                    .open_session(&token)
                    .apps_connections_open(&SlackApiAppsConnectionOpenRequest::new()),
            )
            .await?;
            service
                .settings
                .proxy
                .websocket(opened.url.value().as_str())
                .await
                .map_err(|_| Error::Request)
        };
        let connection = tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = connection => result,
        };
        if let Ok(mut socket) = connection {
            let mut hello = false;
            let mut deadline = tokio::time::Instant::now() + Duration::from_secs(45);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
            heartbeat.tick().await;
            loop {
                let frame = tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    _ = heartbeat.tick(), if hello => {
                        if tokio::time::timeout(Duration::from_secs(5), socket.send(tokio_tungstenite::tungstenite::Message::Ping(Vec::new().into())))
                            .await.map_or(true, |r| r.is_err()) { break; }
                        continue;
                    }
                    result = tokio::time::timeout_at(deadline, socket.next()) => result,
                };
                let Ok(Some(Ok(frame))) = frame else { break };
                if hello {
                    deadline = tokio::time::Instant::now() + Duration::from_secs(90);
                }
                match frame {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        let result = tokio::select! {
                            _ = stop.changed() => return Ok(()),
                            result = tokio::time::timeout(Duration::from_millis(if hello { 2500 } else { 45000 }), dispatch(&service, &api, states.clone(), &text)) => result,
                        };
                        match result {
                            Ok(Ok(SocketReply::Hello)) => {
                                hello = true;
                                backoff = 2;
                                deadline = tokio::time::Instant::now() + Duration::from_secs(90);
                            }
                            Ok(Ok(SocketReply::Reconnect)) => break,
                            Ok(Ok(SocketReply::Ack(ack))) => {
                                if tokio::time::timeout(
                                    Duration::from_secs(5),
                                    socket.send(tokio_tungstenite::tungstenite::Message::Text(
                                        ack.into(),
                                    )),
                                )
                                .await
                                .map_or(true, |r| r.is_err())
                                {
                                    break;
                                }
                            }
                            Ok(Ok(SocketReply::Ignore)) => {}
                            _ => {
                                tracing::warn!(
                                    "Slack callback failed; envelope not acknowledged (payload redacted)"
                                );
                                if !hello {
                                    break;
                                }
                            }
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(_) => {
                        if tokio::time::timeout(Duration::from_secs(5), socket.flush())
                            .await
                            .map_or(true, |r| r.is_err())
                        {
                            break;
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    _ => {}
                }
            }
        }
        service.verified.store(false, Ordering::SeqCst);
        tracing::warn!(
            retry_seconds = backoff,
            "Slack socket disconnected; reconnecting (details redacted)"
        );
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
        }
        backoff = (backoff * 2).min(120);
    }
}

enum SocketReply {
    Hello,
    Reconnect,
    Ack(String),
    Ignore,
}

async fn dispatch(
    service: &Service,
    api: &Api,
    states: SlackClientEventsUserState,
    text: &str,
) -> Result<SocketReply> {
    let event = match serde_json::from_str::<Event>(text) {
        Ok(event) => event,
        Err(_) => {
            return Ok(match Ack::unknown(text) {
                Ok(ack) => SocketReply::Ack(serde_json::to_string(&ack)?),
                Err(_) => SocketReply::Ignore,
            });
        }
    };
    let ack = match event {
        Event::Hello { connection_info } => {
            if connection_info.app_id.to_string() != service.settings.app_id {
                return Err(Error::Ui("slack.error.identity"));
            }
            service.connected().await?;
            service.verified.store(true, Ordering::SeqCst);
            tracing::info!("Slack Socket Mode connected");
            return Ok(SocketReply::Hello);
        }
        Event::Disconnect => return Ok(SocketReply::Reconnect),
        Event::EventsApi { envelope, payload } => {
            push(payload, api.client.clone(), states)
                .await
                .map_err(|_| Error::Request)?;
            serde_json::to_string(&Ack::bare(envelope.envelope_id))?
        }
        Event::Interactive {
            mut envelope,
            payload,
        } => {
            let reply = interaction(payload, api.client.clone(), states)
                .await
                .map_err(|_| Error::Request)?;
            envelope.accepts_response_payload &= !matches!(reply, SlackInteractionResponse::Empty);
            serde_json::to_string(&Ack::new(envelope, reply))?
        }
    };
    Ok(SocketReply::Ack(ack))
}
