use super::Service;
use crate::Api;
use crate::Command;
use crate::Input;
use engine::now;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

impl Service {
    pub(super) async fn deliver(
        self: Arc<Self>,
        api: Api,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), crate::Error> {
        loop {
            let rows = self
                .store
                .discord_pending(api.bot, api.guild, now())
                .await
                .map_err(|_| crate::Error::Backend)?;
            for row in rows.into_iter().take(1) {
                let mut input = Input {
                    locale: None,
                    id: 0,
                    bot: api.bot,
                    guild: api.guild,
                    channel: row.channel.parse().map_err(|_| crate::Error::Backend)?,
                    user: 0,
                    date: now(),
                    command: Command::Status,
                };
                let snapshot = self.config.snapshot();
                let binding = self
                    .store
                    .discord_binding(api.bot, api.guild, input.channel)
                    .await
                    .map_err(|_| crate::Error::Backend)?;
                let valid = if let Some(b) = binding {
                    input.user = b.owner.parse().unwrap_or(0);
                    b.revision == row.revision
                        && self.validate_binding(&snapshot.config, &b).is_ok()
                } else {
                    false
                };
                if !valid {
                    self.store
                        .discord_delivery_state(&row.id, "pending", "rejected")
                        .await
                        .map_err(|_| crate::Error::Backend)?;
                    continue;
                }
                match api.authorize(&input).await {
                    Ok(()) => {}
                    Err(crate::Error::Denied) => {
                        self.store
                            .discord_delivery_state(&row.id, "pending", "rejected")
                            .await
                            .map_err(|_| crate::Error::Backend)?;
                        continue;
                    }
                    Err(_) => {
                        // A failed read of ownership is not a revocation. Retain
                        // the unsent bundle and recheck when Discord is reachable.
                        tracing::warn!(
                            "Discord owner verification unavailable; output remains pending"
                        );
                        continue;
                    }
                }
                if *self.stop.borrow() {
                    return Ok(());
                }
                if self.config.snapshot().hash != snapshot.hash {
                    continue;
                }
                let current = self
                    .store
                    .discord_binding(api.bot, api.guild, input.channel)
                    .await
                    .map_err(|_| crate::Error::Backend)?;
                if current.is_none_or(|b| b.revision != row.revision) {
                    self.store
                        .discord_delivery_state(&row.id, "pending", "rejected")
                        .await
                        .map_err(|_| crate::Error::Backend)?;
                    continue;
                }
                self.store
                    .discord_delivery_state(&row.id, "pending", "dispatching")
                    .await
                    .map_err(|_| crate::Error::Backend)?;
                let sent = api
                    .send_in(input.channel, &row.payload, &snapshot.config.language)
                    .await
                    .is_ok();
                self.store
                    .discord_delivery_state(
                        &row.id,
                        "dispatching",
                        if sent { "delivered" } else { "outcome_unknown" },
                    )
                    .await
                    .map_err(|_| crate::Error::Backend)?;
            }
            tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_secs(4))=>{}}
        }
        Ok(())
    }
}
