//! Discord Gateway connection, proxy relay and shutdown lifecycle.
use crate::discord::{Api, Backend, Error};
use secrecy::{ExposeSecret, SecretString};
use serenity::all::GatewayIntents;
use std::sync::Arc;
use tokio::sync::watch;

#[path = "command_menu.rs"]
mod command_menu;

#[path = "interactions.rs"]
mod interactions;

use interactions::Handler;

/// Register guild commands and run the Gateway until shutdown is requested.
pub async fn run(
    token: SecretString,
    api: Api,
    backend: Arc<dyn Backend>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let guild = api.guild;
    let bot = api.bot;
    command_menu::register(&api).await?;
    let proxy = api.proxy.clone();
    let mut client = serenity::client::ClientBuilder::new_with_http(
        crate::discord::api::http(token.expose_secret(), &api.proxy)?,
        GatewayIntents::GUILDS,
    )
    .event_handler(Handler { backend, api })
    .await
    .map_err(|_| Error::Request)?;
    let _relay = if proxy.enabled() {
        let upstream = client.ws_url.lock().await.clone();
        let relay = crate::network::Relay::start(proxy, upstream)
            .await
            .map_err(|_| Error::Request)?;
        *client.ws_url.lock().await = relay.endpoint.clone();
        Some(relay)
    } else {
        None
    };
    tracing::info!(
        guild_id = guild,
        bot_id = bot,
        "Discord slash-command prototype ready"
    );
    let manager = client.shard_manager.clone();
    let result = tokio::select! { result=client.start()=>result.map_err(|_|Error::Request), _=stop.changed()=>Ok(()) };
    manager.shutdown_all().await;
    result
}
