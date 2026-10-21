use super::{Result, Service, error};
use crate::discord::{
    Api, Command, Input,
    ui::{self, Screen, View},
};
use crate::navigation::Navigator;
use engine::reload::Snapshot;

#[path = "menu_model.rs"]
mod model;

use model::Menu;

#[path = "binding_menu.rs"]
mod bindings;

#[path = "new_session.rs"]
mod creation;

const PAGE_SIZE: usize = 25;

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
                Command::Bindings => View::Bindings { offset: 0 },
                Command::Unbind => {
                    let binding = self.bound(&input).await?;
                    let target = crate::bindings::Target::Discord {
                        channel: binding.channel,
                    };
                    let entry =
                        crate::bindings::Manager::new(&self.store, self.binding_scope(&input))
                            .entry(&snapshot.config, &target, &self.locale(&input))
                            .await
                            .map_err(error)?;
                    View::BindingConfirm {
                        target,
                        version: entry.version,
                    }
                }
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
            View::NewSession { .. } => return Err("discord.ui.expired".into()),
            View::ConfirmCreate { project, text } => {
                self.creation_confirmation(&input, &snapshot, &project, &text)?
            }
            View::Create { project, text } => {
                let selected = Navigator::new(&snapshot.config)
                    .parent(&project)
                    .map_err(error)?;
                input.command = Command::New {
                    workspace: selected.host.id.clone(),
                    project: project.clone(),
                    text,
                    confirmed: true,
                };
                let thread = self.create_session(api, &input, &snapshot).await?;
                let text = self.open(api, &input, &snapshot, &project, &thread).await?;
                self.opened_menu(&input, &project, &thread, text, vec![None])
                    .await?
            }
            View::Bindings { offset } => self.binding_list(&input, &snapshot, offset).await?,
            View::Binding { target } => self.binding_detail(&input, &snapshot, target).await?,
            View::BindingConfirm { target, version } => {
                self.binding_confirmation(&input, &snapshot, target, version)
                    .await?
            }
            View::BindingChange {
                target,
                version,
                change,
            } => {
                self.fresh(api, &input, &snapshot, true).await?;
                crate::bindings::Manager::new(&self.store, self.binding_scope(&input))
                    .change(&target, version, change)
                    .await
                    .map_err(error)?;
                self.binding_detail(&input, &snapshot, target).await?
            }
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
                self.opened_menu(&input, &project, &thread, text, cursors)
                    .await?
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
        let page = Navigator::new(&snapshot.config).workspaces_page(offset, PAGE_SIZE);
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
        menu.button("bindings.heading", View::Bindings { offset: 0 });
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
            .projects_page(workspace, offset, PAGE_SIZE)
            .map_err(error)?;
        let page = view.projects;
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
        if cursors.is_empty() {
            cursors.push(None);
        }
        let loaded = Navigator::new(&snapshot.config)
            .sessions_page(
                &self.store,
                &self.runtimes,
                project,
                cursors.last().cloned().flatten(),
                PAGE_SIZE as u32,
            )
            .await
            .map_err(error)?;
        self.session_page(input, snapshot, loaded, cursors)
    }

    fn session_page(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        loaded: crate::navigation::SessionPage<'_>,
        mut cursors: Vec<Option<String>>,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        let selected = loaded.selection;
        let ws = selected.project;
        let project = ws.id.as_str();
        let page = loaded.sessions;
        // The picker may restart an expired snapshot at page one. Drop the old
        // breadcrumb stack so stale pages cannot invent a Previous button.
        if page.offset == 0 {
            cursors = vec![None];
        }
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
                    page.offset + index + 1
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
        if page.total > 0 {
            menu.text.push_str(&format!(
                "\n{}",
                i18n::Message::new("discord.ui.range")
                    .arg("first", page.offset + 1)
                    .arg("last", (page.offset + PAGE_SIZE).min(page.total))
                    .arg("total", page.total)
                    .render(&locale)
            ));
        }
        menu.form("discord.ui.new_session", project.into());
        let project_offset = Navigator::new(&snapshot.config)
            .project_offset(&ws.id, PAGE_SIZE)
            .map_err(error)?;
        menu.button(
            "discord.ui.back_projects",
            View::Projects {
                workspace: selected.host.id.clone(),
                offset: project_offset,
            },
        );
        if page.offset > 0 && cursors.len() > 1 {
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
            && page.total > PAGE_SIZE
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
