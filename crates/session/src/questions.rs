//! Runtime-owned dialogs. Tokens are opaque, short-lived handles, never native request IDs.
use crate::ProviderError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_PENDING: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
pub struct Dialog {
    pub token: String,
    pub turn: Option<String>,
    pub questions: Vec<Question>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub text: String,
    pub options: Vec<OptionLabel>,
    pub multiple: bool,
    pub other: bool,
    pub secret: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OptionLabel {
    pub label: String,
    pub description: String,
}

// Deliberately omit Debug: question text and answers may contain private content.
#[derive(Clone, Serialize, Deserialize)]
pub struct Submission {
    pub token: String,
    pub answers: Vec<Answer>,
}

impl std::fmt::Debug for Submission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Submission").finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Answer {
    pub question: String,
    pub values: Vec<String>,
}

impl Dialog {
    /// Refuse malformed/oversized native dialogs instead of partially answering them.
    pub fn valid(&self) -> bool {
        let mut ids = HashSet::new();
        let size: usize = self
            .questions
            .iter()
            .map(|q| {
                q.id.len()
                    + q.header.len()
                    + q.text.len()
                    + q.options
                        .iter()
                        .map(|o| o.label.len() + o.description.len())
                        .sum::<usize>()
            })
            .sum();
        size <= 65536
            && !self.token.is_empty()
            && (1..=16).contains(&self.questions.len())
            && self.questions.iter().all(|q| {
                let mut labels = HashSet::new();
                !q.id.is_empty()
                    && ids.insert(&q.id)
                    && q.text.len() <= 16_384
                    && q.header.len() <= 512
                    && q.options.len() <= 32
                    && q.options.iter().all(|o| {
                        !o.label.trim().is_empty()
                            && o.label.len() <= 2048
                            && o.description.len() <= 4096
                            && labels.insert(&o.label)
                    })
            })
    }

    /// All fields must be explicitly answered once. No defaults, secret answers or approvals.
    pub fn validate(&self, submission: &Submission) -> Result<(), ProviderError> {
        if !self.valid()
            || submission.token != self.token
            || submission.answers.len() != self.questions.len()
            || self.questions.iter().any(|q| q.secret)
        {
            return Err(ProviderError::Conflict);
        }
        let mut ids = HashSet::new();
        for answer in &submission.answers {
            let question = self
                .questions
                .iter()
                .find(|q| q.id == answer.question)
                .ok_or(ProviderError::Conflict)?;
            let mut values = HashSet::new();
            if !ids.insert(&answer.question)
                || answer.values.is_empty()
                || answer.values.len() > 32
                || (!question.multiple && answer.values.len() != 1)
                || answer.values.iter().any(|value| {
                    value.trim().is_empty()
                        || value.len() > 8192
                        || !values.insert(value)
                        || (!question.other
                            && !question.options.is_empty()
                            && !question.options.iter().any(|o| o.label == *value))
                })
            {
                return Err(ProviderError::Conflict);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;
