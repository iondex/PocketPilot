use super::{Result, Service, error};
use crate::{
    Api, Command, Input,
    ui::{self, Action, Screen, View},
};
use engine::{
    navigation::{self, Navigator},
    reload::Snapshot,
};
use serenity::all::{CreateButton, CreateSelectMenuOption};

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
        let screen = match view {
            View::Workspaces { offset } => self.workspace_screen(&input, &snapshot, offset).await?,
            View::Projects { workspace, offset } => {
                self.project_screen(&input, &snapshot, &workspace, offset)
                    .await?
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
                let mut screen = Screen::new(text);
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
                let open = CreateButton::new_link(format!(
                    "https://discord.com/channels/{}/{channel}",
                    input.guild
                ))
                .label(i18n::text(&self.locale(&input), "discord.ui.channel"));
                let back = self
                    .menu_button(
                        &input,
                        &snapshot,
                        "discord.ui.back_sessions",
                        View::Sessions { project, cursors },
                    )
                    .await?;
                screen.buttons(vec![open, back]);
                screen
            }
        };
        self.fresh(api, &input, &snapshot, false).await?;
        Ok(screen)
    }

    async fn menu_button(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        id: &str,
        view: View,
    ) -> Result<CreateButton> {
        let token = self
            .store
            .ui_action(input, &snapshot.hash, Action::Navigate(view))
            .await
            .map_err(error)?;
        Ok(ui::button(&token, &i18n::text(&self.locale(input), id)))
    }

    async fn menu_select(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        screen: &mut Screen,
        id: &str,
        choices: Vec<View>,
        options: Vec<CreateSelectMenuOption>,
    ) -> Result<()> {
        if !choices.is_empty() {
            let token = self
                .store
                .ui_action(input, &snapshot.hash, Action::Select(choices))
                .await
                .map_err(error)?;
            screen.select(&token, &i18n::text(&self.locale(input), id), options);
        }
        Ok(())
    }

    async fn workspace_screen(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        offset: usize,
    ) -> Result<Screen> {
        let locale = self.locale(input);
        let page = navigation::page(
            Navigator::new(&snapshot.config).workspaces(),
            offset,
            PAGE_SIZE,
        );
        let mut screen = Screen::new(format!(
            "**🌐 {}**\n{}",
            i18n::text(&locale, "discord.ui.workspaces"),
            i18n::text(&locale, "discord.ui.workspace_hint")
        ));
        let mut options = Vec::new();
        let mut choices = Vec::new();
        for (index, view) in page.items.into_iter().enumerate() {
            options.push(ui::option(
                &view.host.name,
                &i18n::t!(
                    "discord.ui.project_count",
                    locale = &locale,
                    count = view.projects.len()
                ),
                index,
            ));
            choices.push(View::Projects {
                workspace: view.host.id.clone(),
                offset: 0,
            });
        }
        if choices.is_empty() {
            screen.text.push_str(&format!(
                "\n\n{}",
                i18n::text(&locale, "discord.ui.no_workspaces")
            ));
        }
        self.menu_select(
            input,
            snapshot,
            &mut screen,
            "discord.ui.choose_workspace",
            choices,
            options,
        )
        .await?;
        let mut buttons = Vec::new();
        if let Some(offset) = page.previous {
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.previous",
                    View::Workspaces { offset },
                )
                .await?,
            );
        }
        if let Some(offset) = page.next {
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.next",
                    View::Workspaces { offset },
                )
                .await?,
            );
        }
        buttons.push(
            self.menu_button(
                input,
                snapshot,
                "discord.ui.refresh",
                View::Workspaces {
                    offset: page.offset,
                },
            )
            .await?,
        );
        screen.buttons(buttons);
        Ok(screen)
    }

    async fn project_screen(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        workspace: &str,
        offset: usize,
    ) -> Result<Screen> {
        let locale = self.locale(input);
        let view = Navigator::new(&snapshot.config)
            .workspace(workspace)
            .map_err(error)?;
        let page = navigation::page(view.projects, offset, PAGE_SIZE);
        let mut screen = Screen::new(format!(
            "**📁 {}**\n{}",
            i18n::text(&locale, "discord.ui.projects"),
            ui::quoted(&view.host.name)
        ));
        let mut options = Vec::new();
        let mut choices = Vec::new();
        for (index, project) in page.items.into_iter().enumerate() {
            options.push(ui::option(&project.name, &directory(&project.cwd), index));
            choices.push(View::Sessions {
                project: project.id.clone(),
                cursors: vec![None],
            });
        }
        if choices.is_empty() {
            screen.text.push_str(&format!(
                "\n\n{}",
                i18n::text(&locale, "discord.projects.empty")
            ));
        } else {
            screen.text.push_str(&format!(
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
        self.menu_select(
            input,
            snapshot,
            &mut screen,
            "discord.ui.choose_project",
            choices,
            options,
        )
        .await?;
        let mut buttons = vec![
            self.menu_button(
                input,
                snapshot,
                "discord.ui.back_workspaces",
                View::Workspaces { offset: 0 },
            )
            .await?,
        ];
        if let Some(offset) = page.previous {
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.previous",
                    View::Projects {
                        workspace: workspace.into(),
                        offset,
                    },
                )
                .await?,
            );
        }
        if let Some(offset) = page.next {
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.next",
                    View::Projects {
                        workspace: workspace.into(),
                        offset,
                    },
                )
                .await?,
            );
        }
        buttons.push(
            self.menu_button(
                input,
                snapshot,
                "discord.ui.refresh",
                View::Projects {
                    workspace: workspace.into(),
                    offset: page.offset,
                },
            )
            .await?,
        );
        screen.buttons(buttons);
        Ok(screen)
    }

    async fn session_screen(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        project: &str,
        mut cursors: Vec<Option<String>>,
    ) -> Result<Screen> {
        let locale = self.locale(input);
        let selected = Navigator::new(&snapshot.config)
            .parent(project)
            .map_err(error)?;
        let ws = selected.project;
        if cursors.is_empty() {
            cursors.push(None);
        }
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        let page = engine::connections::list_picker(
            &self.store,
            provider.as_ref(),
            ws,
            cursors.last().cloned().flatten(),
            8,
        )
        .await
        .map_err(error)?;
        let mut screen = Screen::new(format!(
            "**💬 {}**\n{} › {}\n{}",
            i18n::text(&locale, "discord.ui.sessions"),
            ui::quoted(&selected.host.name),
            ui::quoted(&ws.name),
            i18n::text(&locale, "discord.ui.session_hint")
        ));
        let mut options = Vec::new();
        let mut choices = Vec::new();
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
            options.push(ui::option(&title, &detail, index).emoji(
                serenity::all::ReactionType::Unicode(ui::status_icon(&session.status).into()),
            ));
            choices.push(View::Open {
                project: ws.id.clone(),
                thread: session.reference.thread,
                cursors: cursors.clone(),
            });
        }
        if choices.is_empty() {
            screen.text.push_str(&format!(
                "\n\n{}",
                i18n::text(&locale, "discord.sessions.empty")
            ));
        }
        self.menu_select(
            input,
            snapshot,
            &mut screen,
            "discord.ui.choose_session",
            choices,
            options,
        )
        .await?;
        let project_offset = Navigator::new(&snapshot.config)
            .workspace(&selected.host.id)
            .map_err(error)?
            .projects
            .iter()
            .position(|p| p.id == project)
            .unwrap_or(0)
            / PAGE_SIZE
            * PAGE_SIZE;
        let mut buttons = vec![
            self.menu_button(
                input,
                snapshot,
                "discord.ui.back_projects",
                View::Projects {
                    workspace: selected.host.id.clone(),
                    offset: project_offset,
                },
            )
            .await?,
        ];
        if cursors.len() > 1 {
            let mut previous = cursors.clone();
            previous.pop();
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.previous",
                    View::Sessions {
                        project: project.into(),
                        cursors: previous,
                    },
                )
                .await?,
            );
        }
        if let Some(cursor) = page.next_cursor
            && cursors.len() < 100
        {
            let mut next = cursors.clone();
            next.push(Some(cursor));
            buttons.push(
                self.menu_button(
                    input,
                    snapshot,
                    "discord.ui.next",
                    View::Sessions {
                        project: project.into(),
                        cursors: next,
                    },
                )
                .await?,
            );
        }
        buttons.push(
            self.menu_button(
                input,
                snapshot,
                "discord.ui.refresh",
                View::Sessions {
                    project: project.into(),
                    cursors: vec![None],
                },
            )
            .await?,
        );
        screen.buttons(buttons);
        Ok(screen)
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
