//! Source/phase-aware message presentation. Original inputs are never rewritten.
use super::{Error, Service};
use crate::{api::escape_markdown, ledger::Binding, storage::Store};
use session::{History, Role, TextItem};
use std::collections::HashSet;

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
    if item.phase.as_deref() == Some("error_retry") {
        format!(
            "⚠️ {}\n{}",
            ::i18n::text(locale, "tg.turn.retrying"),
            escape_markdown(&text)
        )
    } else {
        format!("❌\n{}", escape_markdown(&text))
    }
}

pub(super) fn render(item: &TextItem, locale: &str) -> String {
    if item.is_error() {
        return error_text(item, locale);
    }
    if item.role == Role::User {
        return format!("🧑‍💻\n{}", user_text(item));
    }
    let icon = match item.phase.as_deref() {
        Some("final_answer") => "✅",
        Some("commentary" | "partial_answer" | "plan") => "💭",
        _ => "💬",
    };
    format!("{icon}\n{}", item.text)
}

pub(super) fn initial(
    history: &History,
    active: bool,
    own: &HashSet<String>,
    locale: &str,
) -> String {
    let last = history.items.iter().rposition(|i| i.role == Role::User);
    let latest_turn = last
        .map(|i| history.items[i].turn_id.as_str())
        .or_else(|| history.items.last().map(|i| i.turn_id.as_str()));
    history
        .items
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            if item.role == Role::User && item.client_id.as_ref().is_some_and(|id| own.contains(id))
            {
                return false;
            }
            if active {
                return true;
            }
            item.role == Role::Agent
                && Some(item.turn_id.as_str()) == latest_turn
                && last.is_none_or(|n| *index > n)
                && (item.is_error() || item.phase.as_deref() == Some("final_answer"))
        })
        .map(|(_, item)| render(item, locale))
        .collect::<Vec<_>>()
        .join("\n\n")
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
        Ok(initial(history, active, &own, &locale))
    }
}
