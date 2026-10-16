use crate::{History, Role, TextItem};
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

pub fn short_title(title: &str, max_graphemes: usize) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut result = String::new();
    for grapheme in normalized.graphemes(true).take(max_graphemes) {
        if result.chars().count() + grapheme.chars().count() > 96 {
            break;
        }
        result.push_str(grapheme);
    }
    if result.is_empty() {
        "会话".into()
    } else {
        result
    }
}

pub fn full_history(history: &History) -> String {
    full_history_in(history, "zh-CN")
}

pub fn full_history_in(history: &History, locale: &str) -> String {
    let mut result = format!("{}\n\n", i18n::text(locale, "common.history.heading"));
    for item in &history.items {
        let role = match item.role {
            Role::User => "common.role.user",
            Role::Agent => "common.role.agent",
        };
        let role = i18n::text(locale, role);
        result.push_str(&format!(
            "{role}{}\n{}\n\n",
            if item.complete {
                String::new()
            } else {
                i18n::text(locale, "common.history.incomplete_item")
            },
            item.text
        ));
    }
    if !history.all_pages_read {
        result.push_str(&i18n::text(locale, "common.history.incomplete"));
        result.push('\n');
    }
    result
}

pub struct Interaction<'a> {
    pub user: &'a TextItem,
    pub answers: Vec<&'a TextItem>,
    pub errors: Vec<&'a TextItem>,
}

pub fn last_interaction(history: &History) -> Option<Interaction<'_>> {
    if !history.all_pages_read {
        return None;
    }
    let user_index = history.items.iter().rposition(|i| i.role == Role::User)?;
    let user = &history.items[user_index];
    let answers = history
        .items
        .iter()
        .skip(user_index + 1)
        .filter(|i| {
            i.turn_id == user.turn_id
                && i.role == Role::Agent
                && i.complete
                && i.phase.as_deref() == Some("final_answer")
                && !i.text.trim().is_empty()
        })
        .collect();
    let errors = history
        .items
        .iter()
        .skip(user_index + 1)
        .filter(|i| i.turn_id == user.turn_id && i.is_error())
        .collect();
    Some(Interaction {
        user,
        answers,
        errors,
    })
}

pub fn latest_interaction(history: &History) -> String {
    latest_interaction_in(history, "zh-CN")
}

pub fn latest_interaction_in(history: &History, locale: &str) -> String {
    if !history.all_pages_read {
        return i18n::text(locale, "common.history.incomplete");
    }
    let Some(interaction) = last_interaction(history) else {
        return i18n::text(locale, "common.history.no_input");
    };
    let answer = if interaction.answers.is_empty() {
        i18n::text(locale, "common.history.no_final")
    } else {
        interaction
            .answers
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    i18n::t!(
        "common.history.latest",
        locale = locale,
        user = &interaction.user.text,
        answer = answer
    )
}

/// Telegram is an assistant conversation: user inputs are already visible there.
pub fn agent_history(history: &History) -> String {
    agent_history_in(history, "zh-CN")
}

pub fn agent_history_in(history: &History, locale: &str) -> String {
    let mut text = history
        .items
        .iter()
        .filter(|i| i.role == Role::Agent)
        .map(|i| i.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !history.all_pages_read {
        text.push_str("\n\n");
        text.push_str(&i18n::text(locale, "common.history.partial"));
    }
    text
}

pub fn latest_agent_answer(history: &History) -> String {
    let Some(user) = history.items.iter().rposition(|i| i.role == Role::User) else {
        return agent_history(history);
    };
    history
        .items
        .iter()
        .skip(user + 1)
        .filter(|i| {
            i.role == Role::Agent
                && i.turn_id == history.items[user].turn_id
                && i.phase.as_deref() == Some("final_answer")
        })
        .map(|i| i.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub struct Batcher {
    window: Duration,
    max_delay: Duration,
    threshold: usize,
    opened: Option<Instant>,
    items: Vec<String>,
}

impl Batcher {
    pub fn new(window: Duration, max_delay: Duration, threshold: usize) -> Self {
        Self {
            window,
            max_delay: max_delay.max(window),
            threshold: threshold.max(1),
            opened: None,
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, now: Instant, text: String) {
        self.opened.get_or_insert(now);
        self.items.push(text);
    }

    pub fn take_ready(&mut self, now: Instant, force: bool) -> Option<String> {
        let opened = self.opened?;
        let elapsed = now.saturating_duration_since(opened);
        let due = if self.items.len() >= self.threshold {
            self.max_delay
        } else {
            self.window
        };
        if !force && elapsed < due {
            return None;
        }
        self.opened = None;
        Some(std::mem::take(&mut self.items).join("\n\n────────\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(turn: &str, role: Role, phase: Option<&str>, text: &str) -> TextItem {
        TextItem {
            turn_id: turn.into(),
            item_id: text.into(),
            role,
            phase: phase.map(str::to_owned),
            text: text.into(),
            complete: true,
            client_id: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn last_interaction_excludes_incomplete_finals_and_does_not_guess_from_truncated_history() {
        let mut partial = item("t", Role::Agent, Some("final_answer"), "still incomplete");
        partial.complete = false;
        let mut history = History {
            all_pages_read: true,
            items: vec![item("t", Role::User, None, "latest question"), partial],
        };
        let interaction = last_interaction(&history).unwrap();
        assert_eq!(interaction.user.text, "latest question");
        assert!(interaction.answers.is_empty());
        history.all_pages_read = false;
        assert!(last_interaction(&history).is_none());
    }

    #[test]
    fn never_pairs_a_new_user_input_with_an_old_answer() {
        let history = History {
            all_pages_read: true,
            items: vec![
                item("old", Role::User, None, "old question"),
                item("old", Role::Agent, Some("final_answer"), "old answer"),
                item("new", Role::User, None, "new question"),
                item("new", Role::Agent, Some("commentary"), "working"),
            ],
        };
        let text = latest_interaction(&history);
        assert!(text.contains("new question"));
        assert!(!text.contains("old answer"));
        assert!(!text.contains("working"));
    }

    #[test]
    fn sustained_messages_have_a_fixed_flush_deadline() {
        let now = Instant::now();
        let mut batch = Batcher::new(Duration::from_secs(2), Duration::from_secs(5), 3);
        batch.push(now, "one".into());
        batch.push(now + Duration::from_secs(1), "two".into());
        batch.push(now + Duration::from_secs(2), "three".into());
        assert!(
            batch
                .take_ready(now + Duration::from_secs(4), false)
                .is_none()
        );
        batch.push(now + Duration::from_secs(4), "four".into());
        assert!(
            batch
                .take_ready(now + Duration::from_secs(5), false)
                .unwrap()
                .contains("four")
        );
    }

    #[test]
    fn a_late_steer_does_not_inherit_an_earlier_final_in_the_same_turn() {
        let history = History {
            all_pages_read: true,
            items: vec![
                item("t", Role::User, None, "question"),
                item("t", Role::Agent, Some("final_answer"), "earlier answer"),
                item("t", Role::User, None, "late steer"),
            ],
        };
        let output = latest_interaction(&history);
        assert!(output.contains("late steer"));
        assert!(!output.contains("earlier answer"));
    }
}
