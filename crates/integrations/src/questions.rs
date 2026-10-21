//! One explicit answer grammar for all chats; ordinary messages always remain normal input.
use session::{
    AgentProvider, History, ProviderError, Role, TextItem,
    provider::Mutation,
    questions::{Answer, Dialog, Submission},
};

/// A question is transient runtime state, projected only into the chat observation.
/// Never persist native request IDs or reconstruct a dialog from displayed history.
pub async fn history(
    provider: &dyn AgentProvider,
    thread: &str,
    locale: &str,
) -> Result<History, ProviderError> {
    let mut history = provider.history(thread).await?;
    for dialog in provider.questions(thread).await? {
        let turn = dialog
            .turn
            .clone()
            .or_else(|| history.items.last().map(|i| i.turn_id.clone()))
            .unwrap_or_default();
        history.items.push(TextItem {
            turn_id: turn,
            item_id: format!("question:{}", dialog.token),
            role: Role::Agent,
            phase: Some("question".into()),
            text: render(&dialog, locale),
            complete: false,
            client_id: None,
            attachments: vec![],
        });
    }
    Ok(history)
}

/// Existing platform code has already checked actor, binding, cwd and freshness.
/// The provider rechecks the opaque handle atomically with native dispatch.
pub async fn prepare(
    provider: &dyn AgentProvider,
    thread: &str,
    text: &str,
    has_files: bool,
) -> Result<Option<Mutation>, ProviderError> {
    let text = text.trim();
    let Some(rest) = text
        .strip_prefix("!answer")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    else {
        return Ok(None);
    };
    if has_files || rest.len() > 64 * 1024 {
        return Err(ProviderError::InvalidAnswer);
    }
    let (token, fields) = rest
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(ProviderError::InvalidAnswer)?;
    let dialogs = provider.questions(thread).await?;
    let dialog = dialogs
        .iter()
        .find(|d| d.token == token)
        .ok_or(ProviderError::Conflict)?;
    let mut answers = Vec::new();
    for line in fields.lines().filter(|line| !line.trim().is_empty()) {
        let (number, value) = line.split_once(':').ok_or(ProviderError::InvalidAnswer)?;
        let index = number
            .trim()
            .parse::<usize>()
            .map_err(|_| ProviderError::InvalidAnswer)?;
        let question = index
            .checked_sub(1)
            .and_then(|n| dialog.questions.get(n))
            .ok_or(ProviderError::InvalidAnswer)?;
        let value = value.trim();
        let values = if value.starts_with('[') {
            serde_json::from_str::<Vec<String>>(value).map_err(|_| ProviderError::InvalidAnswer)?
        } else {
            vec![value.into()]
        };
        answers.push(Answer {
            question: question.id.clone(),
            values,
        });
    }
    let submission = Submission {
        token: token.into(),
        answers,
    };
    dialog
        .validate(&submission)
        .map_err(|_| ProviderError::InvalidAnswer)?;
    Ok(Some(Mutation::Answer {
        thread: thread.into(),
        submission,
    }))
}

fn render(dialog: &Dialog, locale: &str) -> String {
    if dialog.questions.iter().any(|q| q.secret) {
        return i18n::text(locale, "common.question.secret");
    }
    let mut text = format!("❓ {}\n\n", i18n::text(locale, "common.question.title"));
    for (index, q) in dialog.questions.iter().enumerate() {
        text.push_str(&format!("{}. {}\n\n", index + 1, q.text));
        for option in &q.options {
            text.push_str(&format!("- {} — {}\n", option.label, option.description));
        }
        text.push('\n');
        if q.multiple {
            text.push_str(&i18n::text(locale, "common.question.multiple"));
            text.push('\n');
        }
    }
    text.push_str(&i18n::text(locale, "common.question.instructions"));
    // Four-backtick fencing contains model-authored backticks in option examples.
    text.push_str(&format!("\n\n````text\n!answer {}\n", dialog.token));
    for (index, _) in dialog.questions.iter().enumerate() {
        text.push_str(&format!("{}: …\n", index + 1));
    }
    text.push_str("````");
    text
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;
