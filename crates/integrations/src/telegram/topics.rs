//! Management of known Telegram topics, with no agent lifecycle operations.
use super::*;

#[derive(sqlx::FromRow)]
struct KnownTopic {
    topic_id: i32,
    title: String,
}

#[derive(Clone, Copy)]
enum Removal {
    Removed,
    Rejected,
    Unknown,
    Skipped,
}

impl Removal {
    fn state(self) -> &'static str {
        match self {
            Self::Removed => "accepted",
            Self::Rejected => "rejected",
            Self::Unknown => "outcome_unknown",
            Self::Skipped => "skipped",
        }
    }
}

impl Store {
    async fn all_topic_ids(&self, bot: i64, chat: i64) -> Result<Vec<i32>, StoreError> {
        sqlx::query_scalar("SELECT r.topic_id FROM topic_requests r WHERE r.bot_id=? AND r.chat_id=? AND r.state='accepted' AND NOT EXISTS(SELECT 1 FROM management_topics m WHERE m.bot_id=r.bot_id AND m.chat_id=r.chat_id AND m.topic_id=r.topic_id AND m.state='accepted') ORDER BY r.topic_id")
            .bind(bot).bind(chat).fetch_all(&self.pool).await.map_err(|_|StoreError::Database)
    }

    async fn known_topics(
        &self,
        bot: i64,
        chat: i64,
        after: i32,
    ) -> Result<Vec<KnownTopic>, StoreError> {
        sqlx::query_as("SELECT r.topic_id,COALESCE(NULLIF(b.topic_title,''),NULLIF(b.display_name,''),'Topic '||r.topic_id) AS title FROM topic_requests r LEFT JOIN bindings b ON b.bot_id=r.bot_id AND b.chat_id=r.chat_id AND b.topic_id=r.topic_id WHERE r.bot_id=? AND r.chat_id=? AND r.state='accepted' AND r.topic_id>? ORDER BY r.topic_id LIMIT 7")
            .bind(bot).bind(chat).bind(after).fetch_all(&self.pool).await.map_err(|_|StoreError::Database)
    }

    async fn known_topic(
        &self,
        bot: i64,
        chat: i64,
        topic: i32,
    ) -> Result<Option<KnownTopic>, StoreError> {
        sqlx::query_as("SELECT r.topic_id,COALESCE(NULLIF(b.topic_title,''),NULLIF(b.display_name,''),'Topic '||r.topic_id) AS title FROM topic_requests r LEFT JOIN bindings b ON b.bot_id=r.bot_id AND b.chat_id=r.chat_id AND b.topic_id=r.topic_id WHERE r.bot_id=? AND r.chat_id=? AND r.topic_id=? AND r.state='accepted'")
            .bind(bot).bind(chat).bind(topic).fetch_optional(&self.pool).await.map_err(|_|StoreError::Database)
    }

    async fn begin_topic_removal(
        &self,
        bot: i64,
        input: &Input,
        topic: i32,
    ) -> Result<bool, StoreError> {
        if !crate::telegram::authorized(input.user, input.chat, true) {
            return Err(StoreError::Unauthorized);
        }
        let mut tx = self.pool.begin().await.map_err(|_| StoreError::Database)?;
        let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM topic_requests r WHERE r.bot_id=? AND r.chat_id=? AND r.topic_id=? AND r.state='accepted' AND NOT EXISTS(SELECT 1 FROM management_topics m WHERE m.bot_id=r.bot_id AND m.chat_id=r.chat_id AND m.topic_id=r.topic_id AND m.state='accepted'))")
            .bind(bot).bind(input.chat).bind(topic).fetch_one(&mut *tx).await.map_err(|_|StoreError::Database)?;
        if !allowed {
            return Ok(false);
        }
        let claimed = sqlx::query("INSERT INTO topic_removals(bot_id,chat_id,topic_id,request_key,state) VALUES(?,?,?,?,'dispatching') ON CONFLICT(bot_id,chat_id,topic_id) DO UPDATE SET request_key=excluded.request_key,state='dispatching' WHERE topic_removals.state IN ('rejected','outcome_unknown') AND topic_removals.request_key<>excluded.request_key")
            .bind(bot).bind(input.chat).bind(topic).bind(&input.event_key).execute(&mut *tx).await.map_err(|_|StoreError::Database)?.rows_affected();
        if claimed != 1 {
            return Ok(false);
        }
        // Invalidate local send permissions before the remote deletion. Never
        // cancel a turn or let pending input revive a removed topic afterwards.
        sqlx::query("UPDATE bindings SET active=0,revision=revision+1 WHERE bot_id=? AND chat_id=? AND topic_id=?")
            .bind(bot).bind(input.chat).bind(topic).execute(&mut *tx).await.map_err(|_|StoreError::Database)?;
        sqlx::query("UPDATE deliveries SET state='rejected' WHERE bot_id=? AND chat_id=? AND topic_id=? AND state='pending'")
            .bind(bot).bind(input.chat).bind(topic).execute(&mut *tx).await.map_err(|_|StoreError::Database)?;
        sqlx::query("UPDATE inbound_updates SET processed=1 WHERE bot_id=? AND chat_id=? AND topic_id=? AND processed=0")
            .bind(bot).bind(input.chat).bind(topic).execute(&mut *tx).await.map_err(|_|StoreError::Database)?;
        sqlx::query("UPDATE callbacks SET consumed=1 WHERE bot_id=? AND chat_id=? AND topic_id=?")
            .bind(bot)
            .bind(input.chat)
            .bind(topic)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Database)?;
        tx.commit().await.map_err(|_| StoreError::Database)?;
        Ok(true)
    }

    async fn end_topic_removal(
        &self,
        bot: i64,
        input: &Input,
        topic: i32,
        state: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE topic_removals SET state=? WHERE bot_id=? AND chat_id=? AND topic_id=? AND request_key=? AND state='dispatching'")
            .bind(state).bind(bot).bind(input.chat).bind(topic).bind(&input.event_key).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }
}

impl Service {
    pub(in crate::telegram::runtime) async fn topics(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        after: i32,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let mut topics = self.store.known_topics(self.bot, input.chat, after).await?;
        let more = topics.len() > 6;
        topics.truncate(6);
        let last = topics.last().map_or(after, |t| t.topic_id);
        let mut choices = Vec::new();
        for topic in topics {
            // A no-op edit validates a known ID without overwriting user edits.
            // Only a definitive missing-topic response invalidates our records.
            let icon = match self.api.check_topic(input.chat, topic.topic_id).await {
                Ok(()) => "💬",
                Err(crate::telegram::Error::TopicUnavailable) => {
                    self.store
                        .unavailable_topic(self.bot, input.chat, topic.topic_id)
                        .await?;
                    tracing::info!(
                        topic_id = topic.topic_id,
                        "manually deleted Telegram topic synchronized"
                    );
                    continue;
                }
                Err(crate::telegram::Error::TopicClosed) => "🔒",
                Err(crate::telegram::Error::RetryAfter(seconds)) => {
                    return Err(crate::telegram::Error::RetryAfter(seconds).into());
                }
                Err(error) => {
                    tracing::debug!(%error,topic_id=topic.topic_id,"Telegram topic existence unconfirmed; retaining association");
                    "❔"
                }
            };
            choices.push(Choice {
                label: format!(
                    "{icon} {}",
                    crate::telegram::presentation::title(&topic.title)
                )
                .into(),
                action: Action::ConfirmTopicRemoval {
                    topic: topic.topic_id,
                },
            });
        }
        if more {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: Action::Topics { after: last },
            });
        }
        if !choices.is_empty() {
            choices.push(Choice {
                label: UiText::id("tg.topics.clear_heading"),
                action: Action::ConfirmClearTopics,
            });
        }
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.topics.heading"),
                note: UiText::id(if choices.is_empty() {
                    "tg.topics.empty"
                } else {
                    "tg.topics.note"
                }),
                choices,
                back: Action::Home,
                refresh: Action::Topics { after },
                scope: None,
            },
        )
        .await
    }

    pub(in crate::telegram::runtime) async fn confirm_topic_removal(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        topic: i32,
    ) -> Result<(), Error> {
        let Some(record) = self.store.known_topic(self.bot, input.chat, topic).await? else {
            return self.topics(input, snapshot, 0).await;
        };
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.topics.remove_heading"),
                note: ::i18n::t!(
                    "tg.topics.remove_note",
                    locale = crate::telegram::i18n::lang(input),
                    title = record.title
                )
                .into(),
                choices: vec![Choice {
                    label: UiText::id("tg.topics.remove_action"),
                    action: Action::RemoveTopic { topic },
                }],
                back: Action::Topics { after: 0 },
                refresh: Action::Topics { after: 0 },
                scope: None,
            },
        )
        .await
    }

    async fn remove_one_topic(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        topic: i32,
    ) -> Result<Removal, Error> {
        self.authorize(input, &snapshot.hash).await?;
        if input.topic == topic {
            return Err(Error::Stale);
        }
        if !self
            .store
            .begin_topic_removal(self.bot, input, topic)
            .await?
        {
            return Ok(Removal::Skipped);
        }
        tracing::info!(
            topic_id = topic,
            "authorized Telegram topic removal dispatched"
        );
        let result = self.api.delete_topic(input.chat, topic).await;
        let outcome = match &result {
            Ok(()) | Err(crate::telegram::Error::TopicUnavailable) => Removal::Removed,
            Err(
                crate::telegram::Error::Rejected
                | crate::telegram::Error::Authentication
                | crate::telegram::Error::NoRecipient
                | crate::telegram::Error::TopicClosed
                | crate::telegram::Error::RetryAfter(_),
            ) => Removal::Rejected,
            _ => Removal::Unknown,
        };
        if matches!(outcome, Removal::Removed) {
            self.store
                .unavailable_topic(self.bot, input.chat, topic)
                .await?;
        }
        self.store
            .end_topic_removal(self.bot, input, topic, outcome.state())
            .await?;
        tracing::info!(
            topic_id = topic,
            state = outcome.state(),
            "Telegram topic removal completed; agent lifecycle untouched"
        );
        Ok(outcome)
    }

    pub(in crate::telegram::runtime) async fn remove_topic(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        topic: i32,
    ) -> Result<(), Error> {
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let outcome = self.remove_one_topic(input, snapshot, topic).await?;
        let text = match outcome {
            Removal::Removed => "tg.topics.removed",
            Removal::Rejected => "tg.topics.rejected",
            Removal::Unknown => "tg.topics.unknown",
            Removal::Skipped => return self.topics(input, snapshot, 0).await,
        };
        self.respond(
            input,
            &format!("topic-removal:{}", input.event_key),
            Outbound::ui(text),
        )
        .await?;
        self.topics(input, snapshot, 0).await
    }

    pub(in crate::telegram::runtime) async fn confirm_clear_topics(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let topics = self.store.all_topic_ids(self.bot, input.chat).await?;
        if topics.is_empty() {
            return self.topics(input, snapshot, 0).await;
        }
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("tg.topics.clear_heading"),
                note: ::i18n::t!(
                    "tg.topics.clear_note",
                    locale = crate::telegram::i18n::lang(input),
                    count = topics.len()
                )
                .into(),
                choices: vec![Choice {
                    label: UiText::id("tg.topics.clear_action"),
                    action: Action::ClearTopics { topics },
                }],
                back: Action::Topics { after: 0 },
                refresh: Action::Topics { after: 0 },
                scope: None,
            },
        )
        .await
    }

    pub(in crate::telegram::runtime) async fn clear_topics(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        topics: Vec<i32>,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let total = topics.len();
        let mut removed = 0;
        let mut unconfirmed = 0;
        for (index, topic) in topics.into_iter().enumerate() {
            if index > 0 {
                // Destructive calls are serialized with a conservative per-chat
                // budget. A failure stops this batch rather than retrying it.
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            }
            match self.remove_one_topic(input, snapshot, topic).await? {
                Removal::Removed => removed += 1,
                Removal::Skipped => {
                    if self
                        .store
                        .known_topic(self.bot, input.chat, topic)
                        .await?
                        .is_none()
                    {
                        removed += 1;
                    }
                }
                Removal::Rejected | Removal::Unknown => {
                    unconfirmed += 1;
                    break;
                }
            }
        }
        tracing::info!(
            removed,
            total,
            unconfirmed,
            "Telegram topic cleanup finished; no agent operation requested"
        );
        self.respond(
            input,
            &format!("topics-cleared:{}", input.event_key),
            Outbound::text(
                ::i18n::t!(
                    if removed == total {
                        "tg.topics.clear_done"
                    } else {
                        "tg.topics.clear_result"
                    },
                    locale = crate::telegram::i18n::lang(input),
                    removed = removed,
                    total = total,
                    remaining = total - removed
                )
                .to_string(),
            ),
        )
        .await?;
        self.topics(input, snapshot, 0).await
    }
}
