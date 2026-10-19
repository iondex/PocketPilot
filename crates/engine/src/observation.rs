//! Visible message changes, including phases and attachments, with legacy cursor compatibility.
use crate::digest;

pub fn visible_hash(item: &session::TextItem) -> String {
    let mut visible = item.clone();
    // Completion belongs to execution state; a flag flip does not change visible text.
    visible.complete = true;
    format!(
        "v3:{}",
        digest(&serde_json::to_string(&visible).expect("text item serialization"))
    )
}

pub fn unchanged(item: &session::TextItem, previous: Option<&String>) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.starts_with("v3:") {
        return *previous == visible_hash(item);
    }
    if previous.starts_with("v2:") {
        // Upgrade existing observations without re-sending a completion-only change.
        let mut legacy = item.clone();
        return [false, true].into_iter().any(|complete| {
            legacy.complete = complete;
            *previous
                == format!(
                    "v2:{}",
                    digest(&serde_json::to_string(&legacy).expect("text item serialization"))
                )
        });
    }
    *previous == digest(&item.text)
}
