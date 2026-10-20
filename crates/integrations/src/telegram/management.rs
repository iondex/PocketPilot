//! Private management topic routing. This topic never owns an agent session.
use super::*;
use crate::telegram::ledger::Outbound;
use crate::telegram::presentation::{Appearance, MANAGEMENT_ICON, MANAGEMENT_TITLE};
use engine::reload::Snapshot;

impl Store {
    pub async fn management_topic(&self, bot: i64, chat: i64) -> Result<Option<i32>, StoreError> {
        sqlx::query_scalar("SELECT topic_id FROM management_topics WHERE bot_id=? AND chat_id=? AND state='accepted'")
            .bind(bot).bind(chat).fetch_optional(&self.pool).await.map_err(|_|StoreError::Database)
    }

    async fn begin_management(&self, bot: i64, chat: i64) -> Result<bool, StoreError> {
        let n=sqlx::query("INSERT INTO management_topics(bot_id,chat_id,state) VALUES(?,?,'dispatching') ON CONFLICT(bot_id,chat_id) DO UPDATE SET state='dispatching' WHERE management_topics.state='rejected'")
            .bind(bot).bind(chat).execute(&self.pool).await.map_err(|_|StoreError::Database)?.rows_affected();
        Ok(n == 1)
    }

    async fn end_management(
        &self,
        bot: i64,
        chat: i64,
        state: &str,
        topic: Option<i32>,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE management_topics SET state=?,topic_id=? WHERE bot_id=? AND chat_id=? AND state='dispatching'")
            .bind(state).bind(topic).bind(bot).bind(chat).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }
}

impl Service {
    pub(super) async fn management_input(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<Input, Error> {
        self.authorize(input, &snapshot.hash).await?;
        let actor = self
            .runtimes
            .actor(
                "__telegram_management",
                &format!("{}:{}", self.bot, input.chat),
            )
            .await;
        let _guard = actor.lock().await;
        let mut existing = self.store.management_topic(self.bot, input.chat).await?;
        let start = input
            .text
            .split_whitespace()
            .next()
            .is_some_and(|command| command.split('@').next() == Some("/start"));
        if start && let Some(topic) = existing {
            // Validate this address and repair titles written by older versions.
            match self
                .api
                .set_appearance(
                    input.chat,
                    topic,
                    &Appearance {
                        title: crate::telegram::i18n::tr(input, MANAGEMENT_TITLE),
                        emoji: MANAGEMENT_ICON.into(),
                    },
                    None,
                )
                .await
            {
                Ok(()) => {}
                Err(crate::telegram::Error::IconUnavailable(seconds)) => {
                    tracing::debug!(
                        retry_after_seconds = seconds,
                        "management icon repair deferred; navigation remains available"
                    );
                }
                Err(crate::telegram::Error::TopicUnavailable) => {
                    self.store
                        .unavailable_topic(self.bot, input.chat, topic)
                        .await?;
                    existing = None;
                    tracing::info!(
                        bot_id = self.bot,
                        topic_id = topic,
                        "missing management topic confirmed; restoring on explicit start"
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
        let topic = match existing {
            Some(topic) => topic,
            None => {
                if !self.topics_enabled {
                    return Err(Error::TopicsDisabled);
                }
                if !self.store.begin_management(self.bot, input.chat).await? {
                    return Err(Error::Unknown);
                }
                match self
                    .api
                    .create_topic_with_icon(
                        input.chat,
                        &crate::telegram::i18n::tr(input, MANAGEMENT_TITLE),
                        MANAGEMENT_ICON,
                    )
                    .await
                {
                    Ok(topic) => {
                        self.store
                            .end_management(self.bot, input.chat, "accepted", Some(topic))
                            .await?;
                        topic
                    }
                    Err(error) => {
                        let state = if matches!(
                            error,
                            crate::telegram::Error::Rejected
                                | crate::telegram::Error::Authentication
                                | crate::telegram::Error::NoRecipient
                                | crate::telegram::Error::RetryAfter(_)
                        ) {
                            "rejected"
                        } else {
                            "outcome_unknown"
                        };
                        self.store
                            .end_management(self.bot, input.chat, state, None)
                            .await?;
                        return Err(error.into());
                    }
                }
            }
        };
        let mut routed = input.clone();
        routed.topic = topic;
        // A callback can only edit the actual message whose nonce it owns.
        if input.callback.is_some() && topic != input.topic {
            return Err(Error::Expired);
        }
        if topic != input.topic {
            let body = Outbound::ui("tg.management.routed");
            // A different command needs its own visible reply in the origin.
            // Re-delivery of the same input still shares one durable source key.
            self.respond(
                input,
                &format!("management-route:{}", input.event_key),
                body,
            )
            .await?;
            tracing::info!(
                bot_id = self.bot,
                update_id = input.update_id,
                origin_topic = input.topic,
                management_topic = topic,
                "Telegram input routed to management"
            );
        }
        Ok(routed)
    }

    pub(super) async fn default_message(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let routed = self.management_input(input, snapshot).await?;
        if input.topic != routed.topic {
            return self
                .respond(
                    &routed,
                    &format!("management-only:{}", input.event_key),
                    Outbound::ui("tg.management.reject_unbound"),
                )
                .await;
        }
        self.select_navigation(&routed, snapshot).await
    }

    pub(super) async fn opened(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
        topic: i32,
    ) -> Result<(), Error> {
        let _ = topic;
        self.opened_navigation(input, snapshot, workspace).await
    }
}
