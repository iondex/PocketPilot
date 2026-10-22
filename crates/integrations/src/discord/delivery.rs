//! Deliver persisted Discord output without replaying messages whose outcomes are unknown.

use super::Service;
use crate::discord::Api;
use crate::discord::Command;
use crate::discord::Input;
use engine::now;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

impl Service {
    pub(super) async fn deliver(
        self: Arc<Self>,
        api: Api,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), crate::discord::Error> {
        loop {
            let rows = self
                .store
                .discord_pending(api.bot, api.guild, now())
                .await
                .map_err(|_| crate::discord::Error::Backend)?;
            for row in rows.into_iter().take(1) {
                let mut input = Input {
                    uploads: Vec::new(),
                    locale: None,
                    id: 0,
                    bot: api.bot,
                    guild: api.guild,
                    channel: row
                        .channel
                        .parse()
                        .map_err(|_| crate::discord::Error::Backend)?,
                    user: 0,
                    date: now(),
                    command: Command::Status,
                };
                let snapshot = self.config.snapshot();
                let binding = self
                    .store
                    .discord_binding(api.bot, api.guild, input.channel)
                    .await
                    .map_err(|_| crate::discord::Error::Backend)?;
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
                        .map_err(|_| crate::discord::Error::Backend)?;
                    continue;
                }
                match api.authorize(&input).await {
                    Ok(()) => {}
                    Err(crate::discord::Error::Denied) => {
                        self.store
                            .discord_delivery_state(&row.id, "pending", "rejected")
                            .await
                            .map_err(|_| crate::discord::Error::Backend)?;
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
                    .map_err(|_| crate::discord::Error::Backend)?;
                if current.is_none_or(|b| b.revision != row.revision) {
                    self.store
                        .discord_delivery_state(&row.id, "pending", "rejected")
                        .await
                        .map_err(|_| crate::discord::Error::Backend)?;
                    continue;
                }
                let parts: Vec<crate::discord::native::Part> =
                    if let Some(saved) = &row.rendered_parts {
                        serde_json::from_str(saved).map_err(|_| crate::discord::Error::Backend)?
                    } else {
                        let batch = crate::rendering::Renderer::render(
                            &crate::discord::render::ComponentRenderer,
                            crate::rendering::RenderRequest {
                                source: &row.payload,
                                format: session::markdown::Format::Markdown,
                                heading: None,
                            },
                        )
                        .map_err(|_| crate::discord::Error::TooLarge)?;
                        let parts = batch
                            .parts
                            .into_iter()
                            .map(crate::discord::native::Part::V2)
                            .collect::<Vec<_>>();
                        self.store
                            .prepare_parts(&row, &parts)
                            .await
                            .map_err(|_| crate::discord::Error::Backend)?;
                        parts
                    };
                self.store
                    .discord_delivery_state(&row.id, "pending", "dispatching")
                    .await
                    .map_err(|_| crate::discord::Error::Backend)?;
                let Some(part) = usize::try_from(row.next_part)
                    .ok()
                    .and_then(|i| parts.get(i))
                else {
                    self.store
                        .discord_delivery_state(&row.id, "dispatching", "outcome_unknown")
                        .await
                        .map_err(|_| crate::discord::Error::Backend)?;
                    continue;
                };
                match api.send_part(input.channel, part).await {
                    Ok(_) => self
                        .store
                        .acknowledge_part(&row, row.next_part as usize + 1 < parts.len())
                        .await
                        .map_err(|_| crate::discord::Error::Backend)?,
                    Err(_) => self
                        .store
                        .discord_delivery_state(&row.id, "dispatching", "outcome_unknown")
                        .await
                        .map_err(|_| crate::discord::Error::Backend)?,
                }
            }
            tokio::select! {
                _ = stop.changed() => break,
                _ = tokio::time::sleep(Duration::from_secs(4)) => {},
            }
        }
        Ok(())
    }
}
