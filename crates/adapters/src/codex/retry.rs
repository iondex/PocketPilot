//! Classify native terminal errors, without scanning conversation bodies.
use super::Codex;
use crate::codex::protocol as p;
use session::{
    ProviderError,
    retry::{RetryReason, RetryableFailure},
};

impl Codex {
    pub(super) async fn latest_retryable_failure(
        &self,
        thread: &str,
    ) -> Result<Option<RetryableFailure>, ProviderError> {
        let metadata = self
            .client
            .call(&p::ThreadReadParams {
                thread_id: thread.into(),
                include_turns: Some(false),
            })
            .await?;
        if !matches!(metadata.thread.status, p::ThreadStatus::Idle) {
            return Ok(None);
        }
        let turn = match metadata.thread.history_mode {
            p::ThreadHistoryMode::Legacy => self
                .client
                .call(&p::ThreadReadParams {
                    thread_id: thread.into(),
                    include_turns: Some(true),
                })
                .await?
                .thread
                .turns
                .into_iter()
                .last(),
            p::ThreadHistoryMode::Paginated => self
                .client
                .call(&p::ThreadTurnsListParams {
                    thread_id: thread.into(),
                    cursor: None,
                    limit: Some(1),
                    sort_direction: Some(p::SortDirection::Desc),
                    items_view: Some(p::TurnItemsView::NotLoaded),
                })
                .await?
                .data
                .into_iter()
                .next(),
        };
        let Some(turn) = turn else {
            return Ok(None);
        };
        if !matches!(turn.status, p::TurnStatus::Failed) {
            return Ok(None);
        }
        let reason = if let Some(error) = &turn.error {
            classify(error)
        } else {
            crate::codex::errors::merge(&self.errors, thread, vec![])
                .iter()
                .find(|item| item.turn_id == turn.id && item.is_error())
                .and_then(|item| classify_message(&item.text))
        };
        Ok(reason.map(|reason| RetryableFailure {
            turn_id: turn.id,
            reason,
        }))
    }
}

fn classify(error: &p::TurnError) -> Option<RetryReason> {
    use p::{CodexErrorInfo as Info, CodexErrorInfoVariant0 as Kind};
    if permanent_message(&error.message.to_ascii_lowercase()) {
        return None;
    }
    match &error.codex_error_info {
        Some(Info::Variant0(kind)) => match kind {
            Kind::ServerOverloaded | Kind::FlexUnavailable => Some(RetryReason::ServerOverloaded),
            Kind::RateLimitExceeded => Some(RetryReason::RateLimit),
            Kind::InternalServerError => Some(RetryReason::ServerError),
            Kind::Other => classify_message(&error.message),
            _ => None,
        },
        Some(Info::Variant1 {
            http_connection_failed,
        }) => http_reason(http_connection_failed.http_status_code),
        Some(Info::Variant2 {
            response_stream_connection_failed,
        }) => http_reason(response_stream_connection_failed.http_status_code),
        Some(Info::Variant3 {
            response_stream_disconnected,
        }) => http_reason(response_stream_disconnected.http_status_code),
        Some(Info::Variant4 {
            response_too_many_failed_attempts,
        }) => http_reason(response_too_many_failed_attempts.http_status_code),
        None => classify_message(&error.message),
        _ => None,
    }
}

fn http_reason(status: Option<u16>) -> Option<RetryReason> {
    match status {
        Some(429) => Some(RetryReason::RateLimit),
        Some(500..=599) => Some(RetryReason::ServerError),
        None | Some(408) => Some(RetryReason::Transport),
        _ => None,
    }
}

fn classify_message(message: &str) -> Option<RetryReason> {
    let message = message.to_ascii_lowercase();
    if permanent_message(&message) {
        return None;
    }
    if message.contains("concurrency limit reached") || message.contains("concurrent request limit")
    {
        Some(RetryReason::ConcurrencyLimit)
    } else if message.contains("our servers are currently overloaded")
        || message.contains("server is overloaded")
    {
        Some(RetryReason::ServerOverloaded)
    } else if message.contains("rate limit reached")
        || message.contains("rate limit exceeded")
        || message.contains("too many requests")
    {
        Some(RetryReason::RateLimit)
    } else if message.contains("internal server error")
        || message.contains("service unavailable")
        || message.contains("bad gateway")
        || message.contains("gateway timeout")
    {
        Some(RetryReason::ServerError)
    } else if message.contains("stream disconnected before completion")
        || message.contains("error sending request")
        || message.contains("connection reset by peer")
    {
        Some(RetryReason::Transport)
    } else {
        None
    }
}

fn permanent_message(message: &str) -> bool {
    [
        "usage limit",
        "quota",
        "billing",
        "context window",
        "session budget",
        "unauthorized",
        "authentication",
        "api key",
        "permission denied",
        "credits",
    ]
    .iter()
    .any(|s| message.contains(s))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
