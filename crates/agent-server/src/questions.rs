//! Claude-specific question framing. Opaque extensions remain at this protocol boundary.
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use session::{
    ProviderError,
    questions::{Dialog, OptionLabel, Question, Submission},
    server::Failure,
};
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
pub struct Requests {
    pending: BTreeMap<String, Pending>,
    retired: HashSet<String>,
}

struct Pending {
    question: bool,
    dialog: Option<(Dialog, NativeInput)>,
}

#[derive(Deserialize)]
pub struct Request {
    pub subtype: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    input: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
pub struct Snapshot {
    pub request_id: String,
    pub request: Request,
}

type NativeInput = BTreeMap<String, Box<RawValue>>;

#[derive(Deserialize)]
struct AskInput {
    questions: Vec<NativeQuestion>,
}

#[derive(Deserialize)]
struct NativeQuestion {
    question: String,
    header: String,
    options: Vec<OptionLabel>,
    #[serde(rename = "multiSelect")]
    multiple: bool,
}

#[derive(Serialize)]
pub struct Response {
    subtype: &'static str,
    request_id: String,
    response: AllowedQuestion,
}

#[derive(Serialize)]
struct AllowedQuestion {
    behavior: &'static str,
    #[serde(rename = "updatedInput")]
    input: UpdatedInput,
}

#[derive(Serialize)]
struct UpdatedInput {
    #[serde(flatten)]
    original: NativeInput,
    answers: BTreeMap<String, String>,
}

impl Requests {
    pub fn receive(
        &mut self,
        id: String,
        request: Request,
        turn: Option<String>,
    ) -> Result<bool, Failure> {
        if id.len() > 512 {
            return Err(Failure::Invalid);
        }
        if self.retired.contains(&id) || self.pending.contains_key(&id) {
            return Ok(false);
        }
        if self.pending.len() >= session::questions::MAX_PENDING || self.retired.len() >= 4096 {
            return Err(Failure::Busy);
        }
        let question = request.subtype == "can_use_tool"
            && request.tool_name.as_deref() == Some("AskUserQuestion");
        let dialog = if question {
            request.input.and_then(|raw| native(&raw, turn))
        } else {
            None
        };
        self.pending.insert(id, Pending { question, dialog });
        Ok(true)
    }

    pub fn list(&self) -> Vec<Dialog> {
        self.pending
            .values()
            .filter_map(|p| p.dialog.as_ref().map(|(d, _)| d.clone()))
            .collect()
    }

    pub fn waiting(&self) -> (bool, bool) {
        (
            self.pending.values().any(|p| !p.question),
            self.pending.values().any(|p| p.question),
        )
    }

    pub fn cancel(&mut self, id: &str) -> Result<bool, Failure> {
        if self.retired.len() >= 4096 {
            return Err(Failure::Busy);
        }
        let existed = self.pending.remove(id).is_some();
        self.retired.insert(id.into());
        Ok(existed)
    }

    pub fn finish(&mut self) -> Result<(), Failure> {
        let ids: Vec<_> = self.pending.keys().cloned().collect();
        for id in ids {
            self.cancel(&id)?;
        }
        Ok(())
    }

    /// Only a matching AskUserQuestion can ever construct an allow response.
    pub fn answer(
        &mut self,
        submission: Submission,
    ) -> Result<(Response, Option<String>), Failure> {
        let (id, pending) = self
            .pending
            .iter()
            .find(|(_, p)| {
                p.dialog
                    .as_ref()
                    .is_some_and(|(d, _)| d.token == submission.token)
            })
            .ok_or(Failure::Conflict)?;
        let (dialog, _) = pending.dialog.as_ref().ok_or(Failure::Conflict)?;
        dialog.validate(&submission).map_err(|error| match error {
            ProviderError::Unsupported => Failure::Unsupported,
            _ => Failure::Conflict,
        })?;
        if self.retired.len() >= 4096 {
            return Err(Failure::Busy);
        }
        let id = id.clone();
        let pending = self.pending.remove(&id).ok_or(Failure::Conflict)?;
        self.retired.insert(id.clone());
        let (dialog, input) = pending.dialog.ok_or(Failure::Conflict)?;
        let answers = submission
            .answers
            .into_iter()
            .map(|a| (a.question, a.values.join(", ")))
            .collect();
        Ok((
            Response {
                subtype: "success",
                request_id: id,
                response: AllowedQuestion {
                    behavior: "allow",
                    input: UpdatedInput {
                        original: input,
                        answers,
                    },
                },
            },
            dialog.turn,
        ))
    }
}

fn native(raw: &RawValue, turn: Option<String>) -> Option<(Dialog, NativeInput)> {
    if raw.get().len() > 65536 {
        return None;
    }
    let input: AskInput = serde_json::from_str(raw.get()).ok()?;
    let original: NativeInput = serde_json::from_str(raw.get()).ok()?;
    // Replayed inputs (including an explicit null) cannot become fresh dialogs.
    if original.contains_key("answers") {
        return None;
    }
    let questions = input.questions;
    let dialog = Dialog {
        token: uuid::Uuid::new_v4().simple().to_string(),
        turn,
        questions: questions
            .into_iter()
            .map(|q| Question {
                id: q.question.clone(),
                header: q.header,
                text: q.question,
                options: q.options,
                multiple: q.multiple,
                other: true,
                secret: false,
            })
            .collect(),
    };
    dialog.valid().then_some((dialog, original))
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;
