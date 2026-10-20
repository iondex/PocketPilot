//! Source/phase-aware message presentation. Original inputs are never rewritten.
use super::{Error, Service};
use crate::telegram::{api::escape_markdown, ledger::Binding, storage::Store};
use session::{History, Role, TextItem};
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

impl Store {
    pub async fn own_client_ids(
        &self,
        binding: &Binding,
    ) -> Result<HashSet<String>, engine::storage::StoreError> {
        let prefix = format!("telegram:{}:", binding.bot_id);
        let direct = format!("{prefix}message:{}:%:input", binding.chat_id);
        let keys:Vec<String>=sqlx::query_scalar("SELECT source_key FROM operations o WHERE instance_id=? AND thread_id=? AND state IN ('dispatching','accepted','outcome_unknown') AND (source_key LIKE ? OR EXISTS(SELECT 1 FROM inbound_updates i WHERE i.bot_id=? AND i.chat_id=? AND o.source_key=?||i.event_key||':input'))")
            .bind(&binding.instance_id).bind(&binding.thread_id).bind(direct).bind(binding.bot_id).bind(binding.chat_id).bind(&prefix)
            .fetch_all(&self.pool).await.map_err(|_|engine::storage::StoreError::Database)?;
        Ok(keys
            .into_iter()
            .filter_map(|key| {
                Some(format!(
                    "pp:{}:{}",
                    binding.bot_id,
                    key.strip_prefix(&prefix)?.strip_suffix(":input")?
                ))
            })
            .collect())
    }
}

pub(super) fn user_text(item: &TextItem) -> String {
    let content = session::content::user_content(&item.text, &item.attachments);
    let mut text = escape_markdown(&content.text);
    for attachment in content.attachments {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("📎 {}", escape_markdown(&attachment.name)));
    }
    text
}

pub(super) fn error_text(item: &TextItem, locale: &str) -> String {
    if item.is_transient_error() {
        return String::new();
    }
    let text = if item.text.trim().is_empty() {
        ::i18n::text(
            locale,
            if item.phase.as_deref() == Some("turn_interrupted") {
                "tg.turn.interrupted"
            } else {
                "tg.turn.failed"
            },
        )
    } else {
        item.text.clone()
    };
    format!(
        "{}\n\n{}",
        session::presentation::message_heading(item.role, item.phase.as_deref(), locale),
        escape_markdown(&text),
    )
}

pub(super) fn render(item: &TextItem, locale: &str) -> String {
    if item.is_transient_error() {
        return String::new();
    }
    if item.is_error() {
        return error_text(item, locale);
    }
    let body = if item.role == Role::User {
        user_text(item)
    } else {
        item.text.trim_start_matches(['\r', '\n']).to_owned()
    };
    format!(
        "{}\n\n{body}",
        session::presentation::message_heading(item.role, item.phase.as_deref(), locale),
    )
}

pub(super) fn initial(
    history: &History,
    active: bool,
    own: &HashSet<String>,
    locale: &str,
    limit: usize,
) -> String {
    if !history.all_pages_read {
        return ::i18n::text(locale, "tg.history.preview_unavailable");
    }
    let last = history
        .items
        .iter()
        .rposition(|item| item.role == Role::User);
    let turn = last
        .map(|i| history.items[i].turn_id.as_str())
        .or_else(|| history.items.last().map(|i| i.turn_id.as_str()));
    let candidates = history
        .items
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            item.role == Role::Agent
                && !item.is_transient_error()
                && Some(item.turn_id.as_str()) == turn
                && last.is_none_or(|user| *index > user)
        })
        .collect::<Vec<_>>();
    let answer = candidates
        .iter()
        .rev()
        .find(|(_, item)| item.phase.as_deref() == Some("final_answer"))
        .or_else(|| {
            active
                .then(|| candidates.iter().rev().find(|(_, item)| !item.is_error()))
                .flatten()
        });
    let error = candidates.iter().rev().find(|(_, item)| item.is_error());
    let mut selected = Vec::new();
    if let Some(index) = last {
        let item = &history.items[index];
        if !item.client_id.as_ref().is_some_and(|id| own.contains(id)) {
            selected.push((index, 1));
        }
    }
    if let Some((index, _)) = answer {
        selected.push((*index, 3));
    }
    if let Some((index, _)) = error {
        selected.push((*index, 1));
    }
    if selected.is_empty() {
        return String::new();
    }
    let weights = selected.iter().map(|(_, weight)| weight).sum::<usize>();
    let mut clipped = false;
    let mut parts = Vec::new();
    for (index, weight) in &selected {
        let text = render(&history.items[*index], locale);
        let budget = (limit * weight / weights).saturating_sub(8);
        let (preview, shortened) = preview_text(&text, budget);
        clipped |= shortened;
        parts.push(preview);
    }
    let visible = history
        .items
        .iter()
        .filter(|item| {
            !item.is_transient_error()
                && (item.role != Role::User
                    || !item.client_id.as_ref().is_some_and(|id| own.contains(id)))
        })
        .count();
    if clipped || visible > selected.len() {
        parts.push(escape_markdown(&::i18n::text(
            locale,
            "tg.history.preview_hint",
        )));
    }
    parts.join("\n\n")
}

fn preview_text(markdown: &str, budget: usize) -> (String, bool) {
    if markdown.encode_utf16().count() <= budget {
        return (markdown.into(), false);
    }
    let plain = crate::telegram::markdown::render(markdown).plain;
    let mut preview = String::new();
    let mut units = 0;
    for grapheme in plain.graphemes(true) {
        let escaped = escape_markdown(grapheme);
        let size = escaped.encode_utf16().count();
        if units + size > budget.saturating_sub(1) {
            break;
        }
        preview.push_str(&escaped);
        units += size;
    }
    preview.push('…');
    (preview, true)
}

impl Service {
    pub(super) async fn initial_messages(
        &self,
        binding: &Binding,
        history: &History,
        active: bool,
    ) -> Result<String, Error> {
        let own = self.store.own_client_ids(binding).await?;
        let locale = self.ui_locale(binding.chat_id).await;
        Ok(initial(
            history,
            active,
            &own,
            &locale,
            self.config.snapshot().config.delivery.history_preview_chars,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_preview_is_bounded_for_unicode_and_escaped_markdown_at_the_configured_limit() {
        let history = History {
            all_pages_read: true,
            items: vec![TextItem {
                turn_id: "t".into(),
                item_id: "a".into(),
                role: Role::Agent,
                phase: Some("final_answer".into()),
                text: "👩🏽‍💻 **Synthetic result** <value> ".repeat(1000),
                complete: true,
                client_id: None,
                attachments: vec![],
            }],
        };
        for limit in [256, 1200, 2800] {
            let preview = initial(&history, true, &HashSet::new(), "en", limit);
            assert!(preview.encode_utf16().count() < 3800);
            assert!(
                preview.contains('…')
                    && preview.contains("/history")
                    && !preview.contains("/history export")
            );
        }
    }

    #[test]
    fn entry_preview_uses_the_latest_turn_without_pairing_an_old_answer() {
        let item = |turn: &str, role, phase: Option<&str>, text: &str| TextItem {
            turn_id: turn.into(),
            item_id: text.into(),
            role,
            phase: phase.map(str::to_owned),
            text: text.into(),
            complete: true,
            client_id: None,
            attachments: vec![],
        };
        let history = History {
            all_pages_read: true,
            items: vec![
                item("old", Role::User, None, "Earlier request"),
                item("old", Role::Agent, Some("final_answer"), "Earlier final"),
                item("new", Role::User, None, "Current request"),
                item("new", Role::Agent, Some("commentary"), "Current progress"),
            ],
        };
        let preview = initial(&history, true, &HashSet::new(), "en", 1200);
        assert!(
            preview.contains("Current request")
                && preview.contains("🤖 **Agent** · 💭\n\nCurrent progress")
        );
        assert!(!preview.contains("Earlier"));
    }
}
