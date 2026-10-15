//! Private management topic routing. This topic never owns an agent session.
use super::*;
use sqlx::Row;

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
    pub async fn probe_threads(
        &self,
        instance: &str,
    ) -> Result<std::collections::HashSet<String>, StoreError> {
        let receipts=sqlx::query("SELECT receipt FROM operations WHERE instance_id=? AND source_key LIKE 'probe:%:create' AND state='accepted'")
            .bind(instance).fetch_all(&self.pool).await.map_err(|_|StoreError::Database)?;
        let mut threads = std::collections::HashSet::new();
        for row in receipts {
            let Some(raw) = row.get::<Option<String>, _>("receipt") else {
                continue;
            };
            let receipt: Receipt = serde_json::from_str(&raw).map_err(|_| StoreError::Database)?;
            threads.insert(receipt.thread);
        }
        Ok(threads)
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
            .connections
            .actor(
                "__telegram_management",
                &format!("{}:{}", self.bot, input.chat),
            )
            .await;
        let _guard = actor.lock().await;
        let topic = match self.store.management_topic(self.bot, input.chat).await? {
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
                    .create_topic(input.chat, "PocketPilot · 管理")
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
                            telegram::Error::Rejected
                                | telegram::Error::Authentication
                                | telegram::Error::NoRecipient
                                | telegram::Error::RetryAfter(_)
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
        if topic != input.topic
            && self
                .store
                .binding(self.bot, input.chat, input.topic)
                .await?
                .is_none()
        {
            let body = Outbound::text(
                "管理消息已转到「PocketPilot · 管理」topic。这里未关联 agent，普通消息不会发送给 Codex；请到管理 topic 添加会话。",
            );
            self.respond(input, &format!("management-route:{}", input.topic), body)
                .await?;
        }
        Ok(routed)
    }
    pub(super) async fn default_message(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let routed = self.management_input(input, snapshot).await?;
        let text = format!(
            "这是管理 topic，不关联 Codex 会话。\n这条消息未发送给 agent：\n\n{}\n\n使用下方菜单添加会话，再到该会话 topic 发送消息。",
            input.text
        );
        self.workspaces_text(&routed, snapshot, &text).await
    }
    pub(super) async fn menu_response(
        &self,
        input: &Input,
        key: &str,
        mut body: Outbound,
    ) -> Result<(), Error> {
        if input.callback.is_some() {
            body.edit_message = Some(input.message_id);
        }
        self.respond(input, key, body).await
    }
    pub(super) async fn opened(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
        topic: i32,
    ) -> Result<(), Error> {
        let mut body = Outbound::text(format!(
            "已添加会话到独立 topic（编号 {topic}）。\n请在该会话 topic 发送任务；此处只用于管理。"
        ));
        body.buttons = vec![
            vec![
                self.button(
                    input,
                    snapshot,
                    "返回会话列表",
                    Action::Workspace {
                        id: workspace.id.clone(),
                        cursor: None,
                    },
                )
                .await?,
            ],
            vec![
                self.button(input, snapshot, "返回工作区", Action::Home)
                    .await?,
            ],
        ];
        self.menu_response(input, &format!("opened:{}", input.event_key), body)
            .await
    }
}
