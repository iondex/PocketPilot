//! Native topic navigation over the shared binding lifecycle.
use super::*;
use crate::bindings::{Change, Manager, Target};

impl Service {
    pub(in crate::telegram::runtime) async fn binding_list(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        offset: usize,
    ) -> Result<(), Error> {
        let locale = crate::telegram::i18n::lang(input);
        let page = Manager::new(&self.store, self.binding_scope(input.chat))
            .page(&snapshot.config, offset, locale)
            .await?;
        let mut choices: Vec<_> = page
            .items
            .iter()
            .map(|entry| Choice {
                label: format!("{} · {}", entry.title, entry.state.text(locale)).into(),
                action: Action::Binding {
                    target: entry.record.target.clone(),
                },
            })
            .collect();
        if let Some(offset) = page.next {
            choices.push(Choice {
                label: UiText::id("nav.next"),
                action: Action::Bindings { offset },
            });
        }
        let note = if page.total == 0 {
            ::i18n::text(locale, "bindings.empty")
        } else {
            navigation_model::page_counter(&page, locale)
        };
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: UiText::id("bindings.heading"),
                note: note.into(),
                choices,
                back: page
                    .previous
                    .map_or(Action::Home, |offset| Action::Bindings { offset }),
                refresh: Action::Bindings {
                    offset: page.offset,
                },
                scope: None,
            },
        )
        .await
    }

    pub(in crate::telegram::runtime) async fn binding_detail(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        target: Target,
        confirm: bool,
    ) -> Result<(), Error> {
        let locale = crate::telegram::i18n::lang(input);
        let entry = Manager::new(&self.store, self.binding_scope(input.chat))
            .entry(&snapshot.config, &target, locale)
            .await?;
        let mut choices = Vec::new();
        if confirm && entry.record.active {
            choices.push(Choice {
                label: UiText::id("bindings.confirm_button"),
                action: Action::BindingChange {
                    target: target.clone(),
                    version: entry.version,
                    change: Change::Detach,
                },
            });
        } else {
            let change = entry.observation_change();
            if let Some((label, change)) = change {
                choices.push(Choice {
                    label: UiText::id(label),
                    action: Action::BindingChange {
                        target: target.clone(),
                        version: entry.version,
                        change,
                    },
                });
            }
            if entry.record.active {
                choices.push(Choice {
                    label: UiText::id("bindings.detach"),
                    action: Action::BindingConfirm {
                        target: target.clone(),
                    },
                });
            }
        }
        let note = if confirm {
            ::i18n::text(locale, "bindings.confirm")
        } else {
            format!(
                "{}\n\n{}\n\n{}",
                entry.details(locale),
                ::i18n::text(locale, "bindings.topic_hint"),
                ::i18n::text(locale, "bindings.pause_hint")
            )
        };
        self.show_navigation(
            input,
            snapshot,
            Menu {
                heading: entry.title.into(),
                note: note.into(),
                choices,
                back: if confirm {
                    Action::Binding {
                        target: target.clone(),
                    }
                } else {
                    Action::Bindings { offset: 0 }
                },
                refresh: Action::Binding { target },
                scope: None,
            },
        )
        .await
    }
}
