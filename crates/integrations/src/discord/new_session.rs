//! Native creation form and confirmation; actual writes use the existing operation ledger.
use super::*;
use crate::discord::ui::{Action, NewSessionForm};

impl Service {
    pub(in crate::discord::runtime) async fn prepare_new_session_form(
        &self,
        api: &Api,
        input: &Input,
        token: &str,
    ) -> Result<NewSessionForm> {
        let snapshot = self.config.snapshot();
        self.fresh(api, input, &snapshot, false).await?;
        let Some(View::NewSession { project }) = self
            .store
            .ui_resolve(input, &snapshot.hash, token, None)
            .await
            .map_err(error)?
        else {
            return Err("discord.ui.expired".into());
        };
        let selected = Navigator::new(&snapshot.config)
            .parent(&project)
            .map_err(error)?;
        let title = ui::short(
            &format!(
                "{} · {}",
                i18n::text(&self.locale(input), "discord.ui.new_session"),
                selected.project.name
            ),
            45,
        );
        let token = self
            .store
            .ui_action(input, &snapshot.hash, Action::Draft { project })
            .await
            .map_err(error)?;
        self.fresh(api, input, &snapshot, false).await?;
        Ok(NewSessionForm { token, title })
    }

    pub(super) fn creation_confirmation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        project: &str,
        text: &str,
    ) -> Result<Menu> {
        let selected = Navigator::new(&snapshot.config)
            .parent(project)
            .map_err(error)?;
        let locale = self.locale(input);
        let mut menu = Menu::new(
            format!(
                "**{}**\n{} › {}\n\n{}\n\n{}",
                i18n::text(&locale, "discord.ui.create_confirm"),
                ui::quoted(&selected.host.name),
                ui::quoted(&selected.project.name),
                i18n::text(&locale, "discord.ui.create_notice"),
                crate::discord::render::literal(&session::markdown::preview(text, 1200))
            ),
            "",
        );
        menu.button(
            "discord.ui.create_submit",
            View::Create {
                project: project.into(),
                text: text.into(),
            },
        );
        menu.button(
            "discord.ui.back_sessions",
            View::Sessions {
                project: project.into(),
                cursors: vec![None],
            },
        );
        Ok(menu)
    }

    pub(super) async fn opened_menu(
        &self,
        input: &Input,
        project: &str,
        thread: &str,
        text: String,
        cursors: Vec<Option<String>>,
    ) -> Result<Menu> {
        let binding = self
            .store
            .discord_bindings(input.bot, input.guild)
            .await
            .map_err(error)?
            .into_iter()
            .find(|binding| {
                binding.workspace == project
                    && binding.thread == thread
                    && binding.owner == input.user.to_string()
            })
            .ok_or("discord.error.binding_missing")?;
        let channel = binding
            .channel
            .parse::<u64>()
            .map_err(|_| i18n::Message::new("error.discord.backend"))?;
        let mut menu = Menu::new(text, "");
        menu.link(
            "discord.ui.channel",
            format!("https://discord.com/channels/{}/{channel}", input.guild),
        );
        menu.button(
            "discord.ui.back_sessions",
            View::Sessions {
                project: project.into(),
                cursors,
            },
        );
        Ok(menu)
    }
}
