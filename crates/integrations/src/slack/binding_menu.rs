//! Slack Block Kit views over the shared binding inventory.
use super::*;
use crate::bindings::{Action as BindingAction, Manager, Target, Version};
use crate::slack::render;
use slack_morphism::prelude::*;

impl Service {
    pub(super) async fn binding_list(
        &self,
        r: &Request,
        hash: &str,
        snapshot: &Snapshot,
        offset: usize,
    ) -> Result<()> {
        let navigation = r.navigation();
        let r = &navigation;
        let locale = self.locale();
        let page = Manager::new(&self.store, self.binding_scope())
            .page(&snapshot.config, offset, 8, &locale)
            .await?;
        let mut content = render::message(i18n::text(&locale, "bindings.heading"));
        content.blocks = vec![
            render::header(&content.text),
            render::context(
                &i18n::Message::new("bindings.count")
                    .arg("count", page.total)
                    .render(&locale),
            ),
        ];
        if page.items.is_empty() {
            content
                .blocks
                .push(render::plain(&i18n::text(&locale, "bindings.empty")));
        }
        for (index, entry) in page.items.into_iter().enumerate() {
            let nonce = self
                .store
                .action(
                    r,
                    Action::Run(Command::Binding {
                        target: entry.record.target.clone(),
                    }),
                    hash,
                )
                .await?;
            content.blocks.push(render::section_action(
                &format!("{}. {}", page.offset + index + 1, entry.title),
                &entry.description(&locale),
                &i18n::text(&locale, "bindings.manage"),
                &nonce,
            ));
        }
        if let Some(offset) = page.previous {
            content.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.previous",
                    Action::Run(Command::Bindings { offset }),
                )
                .await?,
            );
        }
        if let Some(offset) = page.next {
            content.blocks.push(
                self.action_button(
                    r,
                    hash,
                    "slack.next",
                    Action::Run(Command::Bindings { offset }),
                )
                .await?,
            );
        }
        content.blocks.push(
            self.action_button(
                r,
                hash,
                "nav.refresh",
                Action::Run(Command::Bindings {
                    offset: page.offset,
                }),
            )
            .await?,
        );
        content.blocks.push(
            self.action_button(r, hash, "bindings.home", Action::Run(Command::Menu))
                .await?,
        );
        self.menu_output(r, content, hash).await
    }

    pub(super) async fn binding_detail(
        &self,
        r: &Request,
        hash: &str,
        snapshot: &Snapshot,
        target: &Target,
    ) -> Result<()> {
        let navigation = r.navigation();
        let r = &navigation;
        let locale = self.locale();
        let scope = self.binding_scope();
        let entry = Manager::new(&self.store, scope.clone())
            .entry(&snapshot.config, target, &locale)
            .await?;
        let mut content = render::message(entry.title.clone());
        content.blocks = vec![
            render::header(&session::markdown::preview(&entry.title, 150)),
            render::plain(&entry.details(&locale)),
            render::context(&i18n::text(&locale, "bindings.pause_hint")),
        ];
        if let Some(url) = entry.jump_url(&scope) {
            content.blocks.push(
                SlackSectionBlock::new()
                    .with_text(md!("<{}|{}>", url, i18n::text(&locale, "bindings.open")))
                    .into(),
            );
        }
        for control in entry.controls(false) {
            let command = match control.action {
                BindingAction::ConfirmDetach { version } => Command::BindingConfirm {
                    target: target.clone(),
                    version,
                },
                BindingAction::Change { version, change } => Command::BindingChange {
                    target: target.clone(),
                    version,
                    change,
                },
            };
            content.blocks.push(
                self.action_button(r, hash, control.label, Action::Run(command))
                    .await?,
            );
        }
        content.blocks.push(
            self.action_button(
                r,
                hash,
                "bindings.back",
                Action::Run(Command::Bindings { offset: 0 }),
            )
            .await?,
        );
        self.menu_output(r, content, hash).await
    }

    pub(super) async fn binding_confirmation(
        &self,
        r: &Request,
        hash: &str,
        target: &Target,
        version: Version,
        snapshot: &Snapshot,
    ) -> Result<()> {
        let navigation = r.navigation();
        let r = &navigation;
        let locale = self.locale();
        let entry = Manager::new(&self.store, self.binding_scope())
            .confirmation(&snapshot.config, target, Some(version), &locale)
            .await?;
        let mut content = render::message(i18n::text(&locale, "bindings.confirm"));
        content.blocks = vec![
            render::plain(&content.text),
            self.action_button(
                r,
                hash,
                "bindings.back",
                Action::Run(Command::Binding {
                    target: target.clone(),
                }),
            )
            .await?,
        ];
        for control in entry.controls(true) {
            if let BindingAction::Change { version, change } = control.action {
                content.blocks.insert(
                    1,
                    self.action_button(
                        r,
                        hash,
                        control.label,
                        Action::Run(Command::BindingChange {
                            target: target.clone(),
                            version,
                            change,
                        }),
                    )
                    .await?,
                );
            }
        }
        self.menu_output(r, content, hash).await
    }
}
