//! Convert generated Codex DTOs into platform-independent session models.
use crate::codex::protocol as p;
use session::{Role, RuntimeStatus, Session, SessionRef, TextItem};

pub fn runtime(status: &p::ThreadStatus) -> RuntimeStatus {
    match status {
        p::ThreadStatus::NotLoaded => RuntimeStatus::NotLoaded,
        p::ThreadStatus::Idle => RuntimeStatus::Idle,
        p::ThreadStatus::SystemError => RuntimeStatus::SystemError,
        p::ThreadStatus::Active(flags) => RuntimeStatus::Active {
            waiting_on_approval: flags
                .iter()
                .any(|f| matches!(f, p::ThreadActiveFlag::WaitingOnApproval)),
            waiting_on_user_input: flags
                .iter()
                .any(|f| matches!(f, p::ThreadActiveFlag::WaitingOnUserInput)),
        },
    }
}

pub(super) fn summary(instance: &str, thread: p::Thread) -> Session {
    let native_title = thread.name.clone().filter(|name| !name.trim().is_empty());
    let active_turn = thread
        .turns
        .iter()
        .find(|turn| matches!(turn.status, p::TurnStatus::InProgress))
        .map(|t| t.id.clone());
    Session {
        reference: SessionRef {
            instance: instance.into(),
            thread: thread.id,
        },
        title: thread
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if thread.preview.is_empty() {
                    "未命名会话".into()
                } else {
                    thread.preview
                }
            }),
        cwd: thread.cwd.to_string(),
        native_title,
        status: runtime(&thread.status),
        active_turn,
        updated_at: Some(thread.updated_at),
    }
}

fn text_item(turn_id: &str, turn_complete: bool, item: p::ThreadItem) -> Option<TextItem> {
    let item = match item {
        p::ThreadItem::UserMessage {
            client_id,
            content,
            id,
        } => {
            let mut text = Vec::new();
            let mut attachments = Vec::new();
            for input in content {
                let name = match input {
                    p::UserInput::TextUserInput { text: body, .. } => {
                        text.push(body);
                        continue;
                    }
                    p::UserInput::LocalImageUserInput { path, .. }
                    | p::UserInput::LocalAudioUserInput { path, .. } => std::path::Path::new(&path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("attachment")
                        .to_owned(),
                    p::UserInput::ImageUserInput(_) => "image".into(),
                    p::UserInput::AudioUserInput { .. } => "audio".into(),
                    _ => continue,
                };
                attachments.push(session::Attachment { name });
            }
            TextItem {
                turn_id: turn_id.into(),
                item_id: id,
                role: Role::User,
                phase: None,
                text: text.join("\n"),
                complete: true,
                client_id,
                attachments,
            }
        }
        p::ThreadItem::AgentMessage {
            id, phase, text, ..
        } => TextItem {
            turn_id: turn_id.into(),
            item_id: id,
            role: Role::Agent,
            complete: turn_complete || matches!(&phase, Some(p::MessagePhase::FinalAnswer)),
            phase: phase.map(|phase| {
                match phase {
                    p::MessagePhase::Commentary => "commentary",
                    p::MessagePhase::PartialAnswer => "partial_answer",
                    p::MessagePhase::FinalAnswer => "final_answer",
                }
                .into()
            }),
            text,
            client_id: None,
            attachments: Vec::new(),
        },
        p::ThreadItem::Plan { id, text } => TextItem {
            turn_id: turn_id.into(),
            item_id: id,
            role: Role::Agent,
            phase: Some("plan".into()),
            text,
            complete: turn_complete,
            client_id: None,
            attachments: Vec::new(),
        },
        _ => return None,
    };
    (!item.text.is_empty() || !item.attachments.is_empty()).then_some(item)
}

pub(super) fn turn_items(turn: p::Turn) -> Vec<TextItem> {
    let complete = matches!(turn.status, p::TurnStatus::Completed);
    let error = match turn.status {
        p::TurnStatus::Failed | p::TurnStatus::Interrupted => Some(match turn.error {
            Some(error) => ("error", error.message),
            None => (
                if matches!(turn.status, p::TurnStatus::Interrupted) {
                    "turn_interrupted"
                } else {
                    "turn_failed"
                },
                String::new(),
            ),
        }),
        _ => None,
    };
    let mut items = turn
        .items
        .into_iter()
        .filter_map(|item| text_item(&turn.id, complete, item))
        .collect::<Vec<_>>();
    if let Some((phase, text)) = error {
        items.push(TextItem {
            turn_id: turn.id,
            item_id: "pp:turn-error".into(),
            role: Role::Agent,
            phase: Some(phase.into()),
            text,
            complete: true,
            client_id: None,
            attachments: vec![],
        });
    }
    items
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;
