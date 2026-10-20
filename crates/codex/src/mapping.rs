//! Convert generated Codex DTOs into platform-independent session models.
use crate::protocol as p;
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
mod tests {
    use super::*;

    #[test]
    fn only_terminal_turn_states_expose_native_errors() {
        for status in ["inProgress", "completed"] {
            let turn = serde_json::from_str(&format!(r#"{{"id":"t","status":"{status}","itemsView":"full","items":[{{"type":"agentMessage","id":"a","text":"normal progress","phase":"commentary"}}],"error":{{"message":"Reconnecting... 1/5"}}}}"#)).unwrap();
            let items = turn_items(turn);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].text, "normal progress");
            assert!(!items[0].is_error());
        }
    }

    #[test]
    fn failed_turns_preserve_primary_errors_without_internal_details() {
        let turn: p::Turn = serde_json::from_str(r#"{"id":"t","status":"failed","itemsView":"full","items":[{"type":"userMessage","id":"u","content":[{"type":"text","text":"question"}]}],"error":{"message":"request failed","additionalDetails":"private transport diagnostics"}}"#).unwrap();
        let items = turn_items(turn);
        assert_eq!(items.len(), 2);
        assert!(items[1].is_error());
        assert_eq!(items[1].text, "request failed");
        assert!(
            !serde_json::to_string(&items)
                .unwrap()
                .contains("private transport")
        );
        for status in ["failed", "interrupted"] {
            let turn = serde_json::from_str(&format!(
                r#"{{"id":"t","status":"{status}","itemsView":"full","items":[]}}"#
            ))
            .unwrap();
            assert!(turn_items(turn)[0].is_error());
        }
    }

    #[test]
    fn user_images_keep_only_metadata_even_without_text() {
        let item = serde_json::from_str(r#"{"type":"userMessage","id":"u","content":[{"type":"localImage","path":"/private/attachments/picture.jpg"},{"type":"image","url":"data:image/png;base64,SECRET"}]}"#).unwrap();
        let item = text_item("t", true, item).unwrap();
        assert!(item.text.is_empty());
        assert_eq!(item.attachments[0].name, "picture.jpg");
        assert_eq!(item.attachments[1].name, "image");
        let encoded = serde_json::to_string(&item).unwrap();
        assert!(!encoded.contains("SECRET") && !encoded.contains("/private"));
    }

    #[test]
    fn schema_decodes_active_flags_and_rejects_unknown_state() {
        let status: p::ThreadStatus =
            serde_json::from_str(r#"{"type":"active","activeFlags":["waitingOnApproval"]}"#)
                .unwrap();
        assert_eq!(runtime(&status).label(), "🟠 等待审批");
        assert!(serde_json::from_str::<p::ThreadStatus>(r#"{"type":"future-state"}"#).is_err());
    }

    #[test]
    fn typed_tool_outputs_are_excluded_from_chat_history() {
        let item: p::ThreadItem =
            serde_json::from_str(r#"{"type":"reasoning","id":"i","summary":["private"]}"#).unwrap();
        assert!(text_item("turn", true, item).is_none());
    }

    #[test]
    fn failed_or_active_turn_text_is_not_mislabeled_final() {
        let partial: p::ThreadItem =
            serde_json::from_str(r#"{"type":"agentMessage","id":"i","text":"partial"}"#).unwrap();
        assert!(!text_item("turn", false, partial).unwrap().complete);
        let final_item: p::ThreadItem = serde_json::from_str(
            r#"{"type":"agentMessage","id":"i","text":"answer","phase":"final_answer"}"#,
        )
        .unwrap();
        assert!(text_item("turn", false, final_item).unwrap().complete);
    }

    #[test]
    fn failed_turn_error_codes_accept_official_string_and_structured_shapes() {
        let string_error = r#"{"message":"fixture failed","codexErrorInfo":"other"}"#;
        let structured_error = r#"{"message":"fixture failed","codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":502}}}"#;
        assert!(
            serde_json::from_str::<p::TurnError>(string_error).is_ok(),
            "official string error shape must not prevent reading history"
        );
        assert!(
            serde_json::from_str::<p::TurnError>(structured_error).is_ok(),
            "official structured error shape must not prevent reading history"
        );
        assert!(
            serde_json::from_str::<p::TurnError>(
                r#"{"message":"fixture","codexErrorInfo":"futureError"}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<p::TurnError>(
                r#"{"message":"fixture","codexErrorInfo":{"futureError":{"code":42}}}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<p::TurnError>(r#"{"message":"fixture","codexErrorInfo":123}"#)
                .is_err()
        );
    }
}
