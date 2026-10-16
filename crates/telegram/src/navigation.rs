//! Durable, numbered private management screens. Navigation never grants agent permissions.
use super::*;
use crate::ledger::{Outbound, Scope};
use engine::reload::Snapshot;
use sqlx::Row;

#[derive(Clone, Serialize, Deserialize)]
struct Choice {
    label: String,
    action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
struct Menu {
    heading: String,
    note: String,
    choices: Vec<Choice>,
    back: Action,
    refresh: Action,
    scope: Option<Scope>,
}

struct SavedMenu {
    id: String,
    menu: Menu,
    after: i32,
    message: Option<i32>,
    published: Option<i64>,
    consumed: bool,
}

impl Store {
    async fn navigation(&self, bot: i64, input: &Input) -> Result<Option<SavedMenu>, StoreError> {
        let row=sqlx::query("SELECT id,payload,after_message_id,message_id,published_at,consumed FROM navigation WHERE bot_id=? AND chat_id=? AND topic_id=? AND user_id=?")
            .bind(bot).bind(input.chat).bind(input.topic).bind(input.user as i64).fetch_optional(&self.pool).await.map_err(|_|StoreError::Database)?;
        row.map(|row| {
            Ok(SavedMenu {
                id: row.get("id"),
                menu: serde_json::from_str(row.get("payload")).map_err(|_| StoreError::Database)?,
                after: row.get("after_message_id"),
                message: row.get("message_id"),
                published: row.get("published_at"),
                consumed: row.get("consumed"),
            })
        })
        .transpose()
    }

    async fn save_navigation(
        &self,
        bot: i64,
        input: &Input,
        id: &str,
        menu: &Menu,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO navigation(bot_id,chat_id,topic_id,user_id,id,payload,after_message_id) VALUES(?,?,?,?,?,?,?) ON CONFLICT(bot_id,chat_id,topic_id) DO UPDATE SET user_id=excluded.user_id,id=excluded.id,payload=excluded.payload,after_message_id=excluded.after_message_id,published_at=NULL,consumed=0")
            .bind(bot).bind(input.chat).bind(input.topic).bind(input.user as i64).bind(id)
            .bind(serde_json::to_string(menu).map_err(|_|StoreError::Database)?).bind(input.message_id)
            .execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }

    pub async fn navigation_receipt(
        &self,
        id: &str,
        message: i32,
        time: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE navigation SET message_id=?,published_at=? WHERE id=?")
            .bind(message)
            .bind(time)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|_| StoreError::Database)?;
        Ok(())
    }

    async fn claim_navigation(&self, id: &str) -> Result<bool, StoreError> {
        Ok(
            sqlx::query("UPDATE navigation SET consumed=1 WHERE id=? AND consumed=0")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|_| StoreError::Database)?
                .rows_affected()
                == 1,
        )
    }
}

fn md(text: &str) -> String {
    crate::api::escape_markdown(text)
}

impl Menu {
    fn render(&self) -> String {
        let mut text = format!("*{}*\n", md(&self.heading));
        if !self.note.is_empty() {
            text.push_str(&format!("\n{}\n", md(&self.note)));
        }
        for (index, choice) in self.choices.iter().enumerate() {
            text.push_str(&format!("\n*{}*  {}", index + 1, md(&choice.label)));
        }
        text.push_str("\n\n回复序号选择 · `0` 返回\n`/start` 首页 · `/refresh` 刷新");
        text
    }
}

impl Service {
    async fn show_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        menu: Menu,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        let prior = self.store.navigation(self.bot, input).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut body = Outbound::text(menu.render());
        body.markdown = true;
        body.navigation_id = Some(id.clone());
        body.scope = menu.scope.clone();
        // A locally cleared bot message may still be editable but invisible.
        // Explicit /start restores the screen; numbered steps edit that post.
        let start = input
            .text
            .split_whitespace()
            .next()
            .is_some_and(|command| command.split('@').next() == Some("/start"));
        body.edit_message = if start {
            None
        } else {
            prior.and_then(|p| p.message)
        };
        self.store
            .save_navigation(self.bot, input, &id, &menu)
            .await?;
        // Pairing is checked again by the sender. Scope is checked when selecting
        // a choice, so display/rate changes do not expire a harmless menu.
        self.store
            .enqueue(
                self.bot,
                input.chat,
                input.topic,
                &format!("menu:{}", input.event_key),
                &body,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn workspaces(&self, input: &Input, snapshot: &Snapshot) -> Result<(), Error> {
        let choices = snapshot
            .config
            .hosts
            .iter()
            .map(|host| Choice {
                label: format!(
                    "{} {}",
                    if matches!(host.transport, engine::config::TransportKind::SSHProxy) {
                        "🌐"
                    } else {
                        "💻"
                    },
                    host.name
                ),
                action: Action::Host {
                    id: host.id.clone(),
                },
            })
            .collect();
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: "主机".into(),
                note: "选择要管理的主机。这里仅接受序号和管理命令。".into(),
                choices,
                back: Action::Home,
                refresh: Action::Home,
                scope: None,
            },
        )
        .await
    }

    pub(super) async fn host_workspaces(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
    ) -> Result<(), Error> {
        let Some(host) = snapshot.config.hosts.iter().find(|h| h.id == id) else {
            return self.workspaces(input, snapshot).await;
        };
        let choices = snapshot
            .config
            .workspaces
            .iter()
            .filter(|w| {
                snapshot
                    .config
                    .providers
                    .iter()
                    .any(|p| p.id == w.provider_id && p.host_id == host.id)
            })
            .map(|w| Choice {
                label: format!("📁 {}", w.name),
                action: Action::Workspace {
                    id: w.id.clone(),
                    cursor: None,
                },
            })
            .collect();
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!("{} › 工作区", host.name),
                note: "添加项目：/add /绝对路径 可选名称\n移除入口：/remove 名称".into(),
                choices,
                back: Action::Home,
                refresh: Action::Host { id: id.into() },
                scope: None,
            },
        )
        .await
    }

    pub(super) async fn sessions(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        cursor: Option<String>,
    ) -> Result<(), Error> {
        let Ok(workspace) = self.workspace(&snapshot.config, id) else {
            return self.workspaces(input, snapshot).await;
        };
        let provider = self.connections.get(&snapshot.config, workspace).await?;
        let page =
            connections::list_picker(&self.store, provider.as_ref(), workspace, cursor.clone(), 6)
                .await?;
        let mut choices = page
            .data
            .into_iter()
            .map(|s| Choice {
                label: format!(
                    "{} · {}",
                    s.status.label(),
                    session::delivery::short_title(&s.title, 40)
                ),
                action: Action::Open {
                    workspace: workspace.id.clone(),
                    thread: s.reference.thread,
                },
            })
            .collect::<Vec<_>>();
        choices.push(Choice {
            label: "＋ 新建会话".into(),
            action: Action::NewPrompt {
                workspace: workspace.id.clone(),
            },
        });
        if let Some(cursor) = page.next_cursor {
            choices.push(Choice {
                label: "下一页 →".into(),
                action: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: Some(cursor),
                },
            });
        }
        let host = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .map(|p| p.host_id.clone())
            .ok_or(connections::Error::Selection)?;
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!("{} › 会话", workspace.name),
                note: String::new(),
                choices,
                back: Action::Host { id: host },
                refresh: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor,
                },
                scope: Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?),
            },
        )
        .await
    }

    pub(super) async fn confirm_creation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        text: &str,
    ) -> Result<(), Error> {
        let workspace = self.workspace(&snapshot.config, id)?;
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: format!("{} › 新建会话", workspace.name),
                note: format!("首条任务\n{text}\n\n回复 1 创建并发送，回复 0 返回。"),
                choices: vec![Choice {
                    label: "确认创建并发送".into(),
                    action: Action::Create {
                        workspace: workspace.id.clone(),
                        text: text.into(),
                    },
                }],
                back: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: None,
                },
                refresh: Action::NewPrompt {
                    workspace: workspace.id.clone(),
                },
                scope: Some(self.output_scope(&snapshot.config, workspace, &workspace.cwd)?),
            },
        )
        .await
    }

    pub(super) async fn opened_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &Workspace,
    ) -> Result<(), Error> {
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: "会话已添加".into(),
                note: "到对应会话 topic 发送任务。".into(),
                choices: vec![Choice {
                    label: "返回会话列表".into(),
                    action: Action::Workspace {
                        id: workspace.id.clone(),
                        cursor: None,
                    },
                }],
                back: Action::Home,
                refresh: Action::Workspace {
                    id: workspace.id.clone(),
                    cursor: None,
                },
                scope: None,
            },
        )
        .await
    }

    async fn show_action(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        action: Action,
    ) -> Result<(), Error> {
        match action {
            Action::Home => self.workspaces(input, snapshot).await,
            Action::Host { id } => self.host_workspaces(input, snapshot, &id).await,
            Action::Workspace { id, cursor } => self.sessions(input, snapshot, &id, cursor).await,
            Action::NewPrompt { workspace } => {
                let Ok(ws) = self.workspace(&snapshot.config, &workspace) else {
                    return self.workspaces(input, snapshot).await;
                };
                self.show_navigation(
                    input,
                    snapshot,
                    Menu {
                        heading: format!("{} › 新建会话", ws.name),
                        note: format!(
                            "使用管理命令填写首条任务：\n/new {} 任务内容",
                            ws.short_name
                        ),
                        choices: Vec::new(),
                        back: Action::Workspace {
                            id: ws.id.clone(),
                            cursor: None,
                        },
                        refresh: Action::NewPrompt {
                            workspace: ws.id.clone(),
                        },
                        scope: None,
                    },
                )
                .await
            }
            _ => self.workspaces(input, snapshot).await,
        }
    }

    pub(super) async fn select_navigation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
    ) -> Result<(), Error> {
        let text = input.text.trim();
        let number = if matches!(text, "/back" | "返回") {
            Some(0)
        } else {
            text.parse::<usize>().ok()
        };
        if number.is_none() && !matches!(text, "/refresh" | "刷新") {
            return self
                .respond(
                    input,
                    &format!("management-reject:{}", input.event_key),
                    Outbound::text("这里只接受菜单序号和管理命令。任务请发到对应会话 topic。"),
                )
                .await;
        }
        let Some(saved) = self.store.navigation(self.bot, input).await? else {
            return self.workspaces(input, snapshot).await;
        };
        if matches!(text, "/refresh" | "刷新") {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        let Some(number) = number else {
            return self
                .respond(
                    input,
                    &format!("management-reject:{}", input.event_key),
                    Outbound::text("这里只接受菜单序号和管理命令。任务请发到对应会话 topic。"),
                )
                .await;
        };
        if input
            .reply_to_message_id
            .is_some_and(|id| Some(id) != saved.message)
            || input.message_id <= saved.after
            || saved.consumed
        {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        if saved.published.is_none() {
            return self
                .respond(
                    input,
                    &format!("menu-loading:{}", input.event_key),
                    Outbound::text("菜单正在更新，请稍后回复序号。"),
                )
                .await;
        }
        let action = if number == 0 {
            saved.menu.back.clone()
        } else if let Some(choice) = saved.menu.choices.get(number - 1) {
            choice.action.clone()
        } else {
            return self
                .respond(
                    input,
                    &format!("menu-range:{}", input.event_key),
                    Outbound::text("请回复菜单中的序号，或回复 0 返回。"),
                )
                .await;
        };
        if number != 0
            && saved
                .menu
                .scope
                .as_ref()
                .is_some_and(|scope| !self.scope_allowed(&snapshot.config, scope))
        {
            return self.show_action(input, snapshot, saved.menu.refresh).await;
        }
        if let Action::Create { workspace, text } = &action
            && (self.delayed(input) || saved.published.is_some_and(|at| input.date < at))
        {
            return self
                .confirm_creation(input, snapshot, workspace, text)
                .await;
        }
        if !self.store.claim_navigation(&saved.id).await? {
            return Ok(());
        }
        match action {
            Action::Open { workspace, thread } => {
                self.open(input, snapshot, &workspace, &thread).await
            }
            Action::Create { workspace, text } => {
                self.create(input, snapshot, &workspace, &text).await
            }
            action => self.show_action(input, snapshot, action).await,
        }
    }
}

impl Service {
    pub(super) async fn add_workspace(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        args: &str,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let Some(menu) = self.store.navigation(self.bot, input).await? else {
            return self.workspaces(input, snapshot).await;
        };
        let Action::Host { id: host_id } = menu.menu.refresh else {
            return self
                .respond(
                    input,
                    &input.event_key,
                    Outbound::text("请先选择主机，再用 /add /绝对路径 可选名称 添加工作区。"),
                )
                .await;
        };
        let providers: Vec<_> = snapshot
            .config
            .providers
            .iter()
            .filter(|p| p.host_id == host_id)
            .collect();
        if providers.len() != 1 {
            return self
                .respond(
                    input,
                    &input.event_key,
                    Outbound::text(if providers.is_empty() {
                        "尚未发现此主机的 Codex 共享服务，请稍后刷新。"
                    } else {
                        "此主机有多个执行实例，请在本机用 workspaces add --provider 指定实例。"
                    }),
                )
                .await;
        }
        let (cwd, name) = args
            .split_once(char::is_whitespace)
            .map_or((args, None), |(p, n)| (p, Some(n.trim())));
        let catalog = engine::catalog::Catalog::new(self.store.pool().clone());
        catalog
            .add_workspace(&snapshot.config, &providers[0].id, cwd, name)
            .await?;
        let mut config = self.config.snapshot().config.clone();
        catalog.load(&mut config).await?;
        self.config.publish_catalog(config)?;
        let next = self.config.snapshot();
        let mut routed = input.clone();
        routed.config_hash = Some(next.hash.clone());
        self.host_workspaces(&routed, &next, &host_id).await
    }

    pub(super) async fn remove_workspace(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
    ) -> Result<(), Error> {
        self.authorize(input, &snapshot.hash).await?;
        if self.delayed(input) {
            return Err(Error::Stale);
        }
        let workspace = self.workspace(&snapshot.config, id)?;
        let catalog = engine::catalog::Catalog::new(self.store.pool().clone());
        catalog.remove_workspace(&workspace.id).await?;
        let mut config = self.config.snapshot().config.clone();
        catalog.load(&mut config).await?;
        self.config.publish_catalog(config)?;
        let next = self.config.snapshot();
        let mut routed = input.clone();
        routed.config_hash = Some(next.hash.clone());
        self.workspaces(&routed, &next).await
    }
}
