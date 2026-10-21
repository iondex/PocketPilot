//! Kept inside the RPC actor so event loss cannot leave an answerable stale dialog.
use super::{Error, protocol as p};
use session::questions::{Dialog, OptionLabel, Question, Submission};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Questions {
    pending: BTreeMap<String, Pending>,
    retired: BTreeSet<String>,
}

struct Pending {
    id: p::RequestId,
    thread: String,
    dialog: Dialog,
}

impl Questions {
    pub fn receive(
        &mut self,
        id: p::RequestId,
        params: p::ToolRequestUserInputParams,
    ) -> Result<(), Error> {
        let key = serde_json::to_string(&id).map_err(|_| Error::Protocol)?;
        if key.len() > 1024 {
            return Err(Error::Protocol);
        }
        if self.retired.contains(&key) || self.pending.contains_key(&key) {
            return Ok(());
        }
        if self.pending.len() >= session::questions::MAX_PENDING || self.retired.len() >= 4096 {
            return Err(Error::Protocol);
        }
        let dialog = Dialog {
            token: uuid::Uuid::new_v4().simple().to_string(),
            turn: Some(params.turn_id),
            questions: params
                .questions
                .into_iter()
                .map(|q| Question {
                    id: q.id,
                    header: q.header,
                    text: q.question,
                    options: q
                        .options
                        .unwrap_or_default()
                        .into_iter()
                        .map(|o| OptionLabel {
                            label: o.label,
                            description: o.description,
                        })
                        .collect(),
                    multiple: false,
                    other: q.is_other,
                    secret: q.is_secret,
                })
                .collect(),
        };
        if !dialog.valid() {
            return Err(Error::Protocol);
        }
        self.pending.insert(
            key,
            Pending {
                id,
                thread: params.thread_id,
                dialog,
            },
        );
        Ok(())
    }

    pub fn list(&self, thread: &str) -> Vec<Dialog> {
        self.pending
            .values()
            .filter(|p| p.thread == thread)
            .map(|p| p.dialog.clone())
            .collect()
    }

    pub fn resolved(&mut self, notification: p::ServerRequestResolvedNotification) {
        if let Ok(key) = serde_json::to_string(&notification.request_id)
            && self
                .pending
                .get(&key)
                .is_some_and(|p| p.thread == notification.thread_id)
        {
            self.pending.remove(&key);
            self.retired.insert(key);
        }
    }

    pub fn completed(&mut self, thread: &str, turn: &str) {
        let keys: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.thread == thread && p.dialog.turn.as_deref() == Some(turn))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            self.pending.remove(&key);
            self.retired.insert(key);
        }
    }

    /// Consume before writing. An uncertain send must never be replayed under another operation.
    pub fn answer(
        &mut self,
        thread: &str,
        submission: Submission,
    ) -> Result<(String, Option<String>), Error> {
        if self.retired.len() >= 4096 {
            return Err(Error::Protocol);
        }
        let (key, pending) = self
            .pending
            .iter()
            .find(|(_, p)| p.thread == thread && p.dialog.token == submission.token)
            .ok_or(Error::TurnChanged)?;
        pending
            .dialog
            .validate(&submission)
            .map_err(|_| Error::TurnChanged)?;
        let result = p::ToolRequestUserInputResponse {
            answers: submission
                .answers
                .into_iter()
                .map(|a| {
                    (
                        a.question,
                        p::ToolRequestUserInputAnswer { answers: a.values },
                    )
                })
                .collect(),
        };
        #[derive(serde::Serialize)]
        struct Response<'a> {
            id: &'a p::RequestId,
            result: p::ToolRequestUserInputResponse,
        }
        let message = serde_json::to_string(&Response {
            id: &pending.id,
            result,
        })
        .map_err(|_| Error::Protocol)?;
        let turn = pending.dialog.turn.clone();
        let key = key.clone();
        self.pending.remove(&key);
        self.retired.insert(key);
        Ok((message, turn))
    }
}
