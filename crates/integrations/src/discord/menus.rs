use super::{Result, Service, error};
use crate::discord::{
    Api, Command, Input,
    ui::{self, Screen, View},
};
use engine::{
    navigation::{self, Navigator},
    reload::Snapshot,
};

#[path = "menu_model.rs"]
mod model;

use model::Menu;

const PAGE_SIZE: usize = 20;

impl Service {
    pub(super) async fn menu_screen(
        &self,
        api: &Api,
        mut input: Input,
        token: Option<String>,
        value: Option<String>,
    ) -> Result<Screen> {
        let snapshot = self.config.snapshot();
        self.fresh(api, &input, &snapshot, false).await?;
        let view = if let Some(token) = token {
            self.store
                .ui_resolve(&input, &snapshot.hash, &token, value.as_deref())
                .await
                .map_err(error)?
                .ok_or("discord.ui.expired")?
        } else {
            match &input.command {
                Command::Workspaces => View::Workspaces { offset: 0 },
                Command::Projects { workspace } => {
                    let host = Navigator::new(&snapshot.config)
                        .workspace(workspace)
                        .map_err(error)?
                        .host;
                    View::Projects {
                        workspace: host.id.clone(),
                        offset: 0,
                    }
                }
                Command::Sessions {
                    workspace,
                    project,
                    cursor,
                } => {
                    let ws = self.project(&snapshot.config, workspace, project)?;
                    View::Sessions {
                        project: ws.id.clone(),
                        cursors: vec![cursor.clone()],
                    }
                }
                _ => return Err("discord.ui.expired".into()),
            }
        };
        let menu = match view {
            View::Workspaces { offset } => self.workspace_screen(&input, &snapshot, offset),
            View::Projects { workspace, offset } => {
                self.project_screen(&input, &snapshot, &workspace, offset)?
            }
            View::Sessions { project, cursors } => {
                self.session_screen(&input, &snapshot, &project, cursors)
                    .await?
            }
            View::Open {
                project,
                thread,
                cursors,
            } => {
                let selected = Navigator::new(&snapshot.config)
                    .parent(&project)
                    .map_err(error)?;
                input.command = Command::Open {
                    workspace: selected.host.id.clone(),
                    project: project.clone(),
                    thread: thread.clone(),
                };
                let text = self.open(api, &input, &snapshot, &project, &thread).await?;
                // The only raw platform markup in this response is the generated numeric channel link.
                let mut menu = Menu::new(text, "");
                let b = self
                    .store
                    .discord_bindings(input.bot, input.guild)
                    .await
                    .map_err(error)?
                    .into_iter()
                    .find(|b| b.workspace == project && b.thread == thread)
                    .ok_or("discord.error.binding_missing")?;
                let channel = b
                    .channel
                    .parse::<u64>()
                    .map_err(|_| i18n::Message::new("error.discord.backend"))?;
                menu.link(
                    "discord.ui.channel",
                    format!("https://discord.com/channels/{}/{channel}", input.guild),
                );
                menu.button(
                    "discord.ui.back_sessions",
                    View::Sessions { project, cursors },
                );
                menu
            }
        };
        let screen = menu
            .bind(&self.store, &input, &snapshot.hash, &self.locale(&input))
            .await?;
        self.fresh(api, &input, &snapshot, false).await?;
        Ok(screen)
    }

    fn workspace_screen(&self, input: &Input, snapshot: &Snapshot, offset: usize) -> Menu {
        let locale = self.locale(input);
        let page = navigation::page(
            Navigator::new(&snapshot.config).workspaces(),
            offset,
            PAGE_SIZE,
        );
        let mut menu = Menu::new(
            format!(
                "**🌐 {}**\n{}",
                i18n::text(&locale, "discord.ui.workspaces"),
                i18n::text(&locale, "discord.ui.workspace_hint")
            ),
            "discord.ui.choose_workspace",
        );
        for view in page.items {
            menu.choice(
                &view.host.name,
                &i18n::t!(
                    "discord.ui.project_count",
                    locale = &locale,
                    count = view.projects.len()
                ),
                View::Projects {
                    workspace: view.host.id.clone(),
                    offset: 0,
                },
                None,
            );
        }
        menu.empty_hint(&locale, "discord.ui.no_workspaces");
        if let Some(offset) = page.previous {
            menu.button("discord.ui.previous", View::Workspaces { offset });
        }
        if let Some(offset) = page.next {
            menu.button("discord.ui.next", View::Workspaces { offset });
        }
        menu.button(
            "discord.ui.refresh",
            View::Workspaces {
                offset: page.offset,
            },
        );
        menu
    }

    fn project_screen(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &str,
        offset: usize,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        let view = Navigator::new(&snapshot.config)
            .workspace(workspace)
            .map_err(error)?;
        let page = navigation::page(view.projects, offset, PAGE_SIZE);
        let mut menu = Menu::new(
            format!(
                "**📁 {}**\n{}",
                i18n::text(&locale, "discord.ui.projects"),
                ui::quoted(&view.host.name)
            ),
            "discord.ui.choose_project",
        );
        for project in page.items {
            menu.choice(
                &project.name,
                &directory(&project.cwd),
                View::Sessions {
                    project: project.id.clone(),
                    cursors: vec![None],
                },
                None,
            );
        }
        if page.total == 0 {
            menu.empty_hint(&locale, "discord.projects.empty");
        } else {
            menu.text.push_str(&format!(
                "\n{}",
                i18n::t!(
                    "discord.ui.range",
                    locale = &locale,
                    first = page.offset + 1,
                    last = (page.offset + PAGE_SIZE).min(page.total),
                    total = page.total
                )
            ));
        }
        menu.button("discord.ui.back_workspaces", View::Workspaces { offset: 0 });
        if let Some(offset) = page.previous {
            menu.button(
                "discord.ui.previous",
                View::Projects {
                    workspace: workspace.into(),
                    offset,
                },
            );
        }
        if let Some(offset) = page.next {
            menu.button(
                "discord.ui.next",
                View::Projects {
                    workspace: workspace.into(),
                    offset,
                },
            );
        }
        menu.button(
            "discord.ui.refresh",
            View::Projects {
                workspace: workspace.into(),
                offset: page.offset,
            },
        );
        Ok(menu)
    }

    async fn session_screen(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        project: &str,
        mut cursors: Vec<Option<String>>,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        let selected = Navigator::new(&snapshot.config)
            .parent(project)
            .map_err(error)?;
        let ws = selected.project;
        if cursors.is_empty() {
            cursors.push(None);
        }
        let provider = self
            .runtimes
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        let page = engine::runtimes::list_picker(
            &self.store,
            provider.as_ref(),
            ws,
            cursors.last().cloned().flatten(),
            8,
        )
        .await
        .map_err(error)?;
        let mut menu = Menu::new(
            format!(
                "**💬 {}**\n{} › {}\n{}",
                i18n::text(&locale, "discord.ui.sessions"),
                ui::quoted(&selected.host.name),
                ui::quoted(&ws.name),
                i18n::text(&locale, "discord.ui.session_hint")
            ),
            "discord.ui.choose_session",
        );
        let labels = page
            .data
            .iter()
            .map(|session| ui::short(&session.title, 90))
            .collect::<Vec<_>>();
        for (index, session) in page.data.into_iter().enumerate() {
            let state = if session.status == session::RuntimeStatus::NotLoaded {
                i18n::text(&locale, "discord.ui.not_loaded")
            } else {
                session.status.text_in(&locale)
            };
            let detail = format!("{} · {}", state, ui::updated(session.updated_at, &locale));
            let title = if labels
                .iter()
                .filter(|label| **label == labels[index])
                .count()
                > 1
            {
                format!(
                    "{} · {}",
                    ui::short(&session.title, 80),
                    (cursors.len() - 1) * 8 + index + 1
                )
            } else {
                labels[index].clone()
            };
            menu.choice(
                &title,
                &detail,
                View::Open {
                    project: ws.id.clone(),
                    thread: session.reference.thread,
                    cursors: cursors.clone(),
                },
                Some(ui::status_icon(&session.status)),
            );
        }
        menu.empty_hint(&locale, "discord.sessions.empty");
        let project_offset = Navigator::new(&snapshot.config)
            .workspace(&selected.host.id)
            .map_err(error)?
            .projects
            .iter()
            .position(|p| p.id == project)
            .unwrap_or(0)
            / PAGE_SIZE
            * PAGE_SIZE;
        menu.button(
            "discord.ui.back_projects",
            View::Projects {
                workspace: selected.host.id.clone(),
                offset: project_offset,
            },
        );
        if cursors.len() > 1 {
            let mut previous = cursors.clone();
            previous.pop();
            menu.button(
                "discord.ui.previous",
                View::Sessions {
                    project: project.into(),
                    cursors: previous,
                },
            );
        }
        if let Some(cursor) = page.next_cursor
            && cursors.len() < 100
        {
            let mut next = cursors.clone();
            next.push(Some(cursor));
            menu.button(
                "discord.ui.next",
                View::Sessions {
                    project: project.into(),
                    cursors: next,
                },
            );
        }
        menu.button(
            "discord.ui.refresh",
            View::Sessions {
                project: project.into(),
                cursors: vec![None],
            },
        );
        Ok(menu)
    }
}

/// Keep the useful end of long paths in the native option's description.
fn directory(path: &str) -> String {
    if session::markdown::fits(path, 95) {
        return ui::short(path, 95);
    }
    let end = path
        .chars()
        .rev()
        .take(90)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("…{}", ui::short(&end, 94))
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
