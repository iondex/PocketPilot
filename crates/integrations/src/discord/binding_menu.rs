//! Discord native controls for the shared binding inventory.
use super::*;
use crate::bindings::{Change, Manager, Target, Version};

impl Service {
    pub(super) async fn binding_list(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        offset: usize,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        let page = Manager::new(&self.store, self.binding_scope(input))
            .page(&snapshot.config, offset, &locale)
            .await
            .map_err(error)?;
        let mut menu = Menu::new(
            format!(
                "**{}**\n\n{}",
                i18n::text(&locale, "bindings.heading"),
                i18n::Message::new("bindings.count")
                    .arg("count", page.total)
                    .render(&locale)
            ),
            "bindings.choose",
        );
        for (index, entry) in page.items.into_iter().enumerate() {
            menu.choice(
                &format!("{}. {}", page.offset + index + 1, entry.title),
                &entry.description(&locale),
                View::Binding {
                    target: entry.record.target,
                },
                None,
            );
        }
        menu.empty_hint(&locale, "bindings.empty");
        if let Some(offset) = page.previous {
            menu.button("discord.ui.previous", View::Bindings { offset });
        }
        if let Some(offset) = page.next {
            menu.button("discord.ui.next", View::Bindings { offset });
        }
        menu.button(
            "discord.ui.refresh",
            View::Bindings {
                offset: page.offset,
            },
        );
        menu.button("bindings.home", View::Workspaces { offset: 0 });
        Ok(menu)
    }

    pub(super) async fn binding_detail(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        target: Target,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        let scope = self.binding_scope(input);
        let entry = Manager::new(&self.store, scope.clone())
            .entry(&snapshot.config, &target, &locale)
            .await
            .map_err(error)?;
        let mut menu = Menu::new(
            format!(
                "**🔗 {}**\n\n{}\n\n{}",
                ui::quoted(&entry.title),
                crate::discord::render::literal(&entry.details(&locale)),
                i18n::text(&locale, "bindings.pause_hint")
            ),
            "",
        );
        if let Some(url) = entry.jump_url(&scope) {
            menu.link("bindings.open", url);
        }
        let change = entry.observation_change();
        if let Some((label, change)) = change {
            menu.button(
                label,
                View::BindingChange {
                    target: target.clone(),
                    version: entry.version,
                    change,
                },
            );
        }
        if entry.record.active {
            menu.button(
                "bindings.detach",
                View::BindingConfirm {
                    target,
                    version: entry.version,
                },
            );
        }
        menu.button("bindings.back", View::Bindings { offset: 0 });
        Ok(menu)
    }

    pub(super) async fn binding_confirmation(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        target: Target,
        version: Version,
    ) -> Result<Menu> {
        let locale = self.locale(input);
        Manager::new(&self.store, self.binding_scope(input))
            .entry(&snapshot.config, &target, &locale)
            .await
            .map_err(error)?;
        let mut menu = Menu::new(i18n::text(&locale, "bindings.confirm"), "");
        menu.button(
            "bindings.confirm_button",
            View::BindingChange {
                target: target.clone(),
                version,
                change: Change::Detach,
            },
        );
        menu.button("bindings.back", View::Binding { target });
        Ok(menu)
    }
}
