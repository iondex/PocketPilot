use crate::{Error, Result, api::Api, model::*, runtime::Service};
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
    settings: &crate::config::Config,
    bot: &str,
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
    if user == bot || !settings.permits(&settings.team_id, &user, &channel) {
        return None;
    }
    let root = m.origin.thread_ts?.to_string();
    let text = m.content?.text?;
    if text.trim().is_empty() {
        return None;
    }
    let date = m.origin.ts.value().split('.').next()?.parse().ok()?;
    Some(Request {
        id: format!("message:{channel}:{}", m.origin.ts),
        channel,
        user,
        root: Some(root),
        binding: None,
        revision: None,
        date,
        command: Command::Input { text },
    })
}

async fn push(
    event: SlackPushEventCallback,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let service = state(states).await?;
    let source = format!("event:{}", event.event_id);
    if let Some(r) = message(event, &service.settings, &service.bot) {
        tokio::time::timeout(Duration::from_secs(2), service.intake(r, &source))
            .await
            .map_err(|_| Error::Request)??;
    }
    Ok(())
}

async fn command(
    event: SlackCommandEvent,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<SlackCommandEventResponse> {
    let service = state(states).await?;
    let allowed = service.settings.permits(
        event.team_id.as_ref(),
        event.user_id.as_ref(),
        event.channel_id.as_ref(),
    ) && event.command.to_string() == "/pp";
    let text = if allowed {
        let id = format!("command:{}", event.trigger_id);
        let r = Request {
            id: id.clone(),
            channel: event.channel_id.to_string(),
            user: event.user_id.to_string(),
            root: None,
            binding: None,
            revision: None,
            date: engine::now(),
            command: Command::Menu,
        };
        tokio::time::timeout(Duration::from_secs(2), service.intake(r, &id))
            .await
            .map_err(|_| Error::Request)??;
        i18n::text(&service.locale(), "slack.loading")
    } else {
        i18n::text(&service.locale(), "slack.error.scope")
    };
    Ok(
        SlackCommandEventResponse::new(SlackMessageContent::new().with_text(text))
            .with_response_type(SlackMessageResponseType::Ephemeral),
    )
}

async fn interaction(
    event: SlackInteractionEvent,
    client: Arc<SlackHyperClient>,
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

async fn handle_interaction(
    service: &Service,
    client: &SlackHyperClient,
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
            if action.action_id.to_string() != "pp" {
                return Ok(SlackInteractionResponse::Empty);
            }
            let Some(nonce) = &action.value else {
                return Ok(SlackInteractionResponse::Empty);
            };
            if let Some((r, action)) = service
                .click(
                    nonce,
                    Some(channel.id.value()),
                    user.id.value(),
                    None,
                    Some(event.trigger_id.value()),
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
                            .map_err(crate::api::map_error)?;
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
                .click(&nonce, None, event.user.id.value(), Some(&title), None)
                .await?
            {
                let _ = (r, workspace);
            }
        }
        _ => {}
    }
    Ok(SlackInteractionResponse::Empty)
}

pub async fn run(
    service: Arc<Service>,
    api: Arc<Api>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let env = Arc::new(
        SlackClientEventsListenerEnvironment::new(api.client.clone())
            .with_user_state(service.clone())
            .with_error_handler(|_, _, _| {
                tracing::warn!("Slack SDK callback failed (payload redacted)");
                http::StatusCode::SERVICE_UNAVAILABLE
            }),
    );
    let callbacks = SlackSocketModeListenerCallbacks::new()
        .with_hello_events(|event, _client, states| async move {
            if let Ok(service) = state(states).await {
                let verified = event.connection_info.app_id.to_string() == service.settings.app_id;
                service.verified.store(false, Ordering::SeqCst);
                let connected = service.connected().await.is_ok();
                service
                    .verified
                    .store(verified && connected, Ordering::SeqCst);
                if verified && connected {
                    tracing::info!("Slack Socket Mode connected");
                } else {
                    tracing::warn!("Slack identity or recovery initialization failed");
                }
            }
        })
        .with_push_events(push)
        .with_command_events(command)
        .with_interaction_events(interaction);
    let listener = SlackClientSocketModeListener::new(
        &SlackClientSocketModeConfig::new().with_max_connections_count(1),
        env,
        callbacks,
    );
    let token = SlackApiToken::new(service.settings.app_token.clone().into());
    crate::api::bounded(listener.listen_for(&token)).await?;
    listener.start().await;
    let mut ready_stop = stop.clone();
    tokio::select! {
        _=stop.changed()=>{},
        result=async {
            let deadline=tokio::time::Instant::now()+Duration::from_secs(45);
            while !service.verified.load(Ordering::SeqCst) {
                if tokio::time::Instant::now()>deadline {return Err(Error::Ui("slack.error.identity"));}
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            ready_stop.changed().await.map_err(|_|Error::Request)
        }=>{if let Err(error)=result {listener.shutdown().await;return Err(error);}},
    }
    listener.shutdown().await;
    Ok(())
}
