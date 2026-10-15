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
    let mut result = String::from("会话历史（按已读取记录；非原子快照）\n\n");
    for item in &history.items {
        let role = match item.role {
            Role::User => "用户",
            Role::Agent => "Agent",
        };
        result.push_str(&format!(
            "{role}{}\n{}\n\n",
            if item.complete { "" } else { "（未完成）" },
            item.text
        ));
    }
    if !history.all_pages_read {
        result.push_str("注意：历史未完整读取。\n");
    }
    result
}

pub fn latest_interaction(history: &History) -> String {
    let Some(user_index) = history.items.iter().rposition(|i| i.role == Role::User) else {
        return "尚无用户输入。".into();
    };
    let user = &history.items[user_index];
    let finals: Vec<&TextItem> = history
        .items
        .iter()
        .skip(user_index + 1)
        .filter(|i| {
            i.turn_id == user.turn_id
                && i.role == Role::Agent
                && i.phase.as_deref() == Some("final_answer")
        })
        .collect();
    let answer = if finals.is_empty() {
        "这次输入尚无可确认的最终输出。".into()
    } else {
        finals
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    format!("最后用户输入\n{}\n\n最终输出\n{}", user.text, answer)
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
        }
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
