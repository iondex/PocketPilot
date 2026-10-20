//! Classify native terminal errors, without scanning conversation bodies.
use super::Codex;
use crate::protocol as p;
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
            crate::errors::merge(&self.errors, thread, vec![])
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
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Frame {
        id: p::RequestId,
        method: String,
        params: Box<serde_json::value::RawValue>,
    }

    #[tokio::test]
    async fn only_the_latest_failed_turn_is_retryable_and_observation_emits_no_writes() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_util::codec::{Framed, LinesCodec};
        for mode in ["legacy", "paginated"] {
            for status in ["failed", "completed", "interrupted", "inProgress"] {
                let (local, remote) = tokio::io::duplex(16384);
                let (read, write) = tokio::io::split(local);
                let provider = Codex::new(
                    "instance".into(),
                    crate::Client::spawn(
                        Box::new(crate::transport::JsonLines::new(read, write, None)),
                        std::time::Duration::from_secs(2),
                    ),
                );
                let server = tokio::spawn(async move {
                    let mut stream = Framed::new(remote, LinesCodec::new());
                    let frame: Frame =
                        serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                    assert_eq!(frame.method, "thread/read");
                    let params: p::ThreadReadParams =
                        serde_json::from_str(frame.params.get()).unwrap();
                    assert_eq!(params.include_turns, Some(false));
                    let metadata = format!(
                        r#"{{"id":"thread","sessionId":"thread","cliVersion":"0.162.1","projectId":null,"preview":"fixture","cwd":"/projects/example","ephemeral":false,"source":"appServer","modelProvider":"openai","createdAt":100,"updatedAt":100,"status":{{"type":"idle"}},"historyMode":"{mode}","turns":[]}}"#
                    );
                    let mut thread: p::Thread = serde_json::from_str(&metadata).unwrap();
                    stream
                        .send(format!(
                            r#"{{"id":{},"result":{{"thread":{metadata}}}}}"#,
                            frame.id
                        ))
                        .await
                        .unwrap();
                    let latest: p::Turn = serde_json::from_str(&format!(r#"{{"id":"latest","status":"{status}","items":[],"error":{{"message":"Concurrency limit reached"}}}}"#)).unwrap();
                    let frame: Frame =
                        serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                    if mode == "legacy" {
                        assert_eq!(frame.method, "thread/read");
                        let params: p::ThreadReadParams =
                            serde_json::from_str(frame.params.get()).unwrap();
                        assert_eq!(params.include_turns, Some(true));
                        let old = serde_json::from_str(r#"{"id":"old","status":"failed","items":[],"error":{"message":"Concurrency limit reached"}}"#).unwrap();
                        thread.turns = vec![old, latest];
                        stream
                            .send(format!(
                                r#"{{"id":{},"result":{}}}"#,
                                frame.id,
                                serde_json::to_string(&p::ThreadReadResponse { thread }).unwrap()
                            ))
                            .await
                            .unwrap();
                    } else {
                        assert_eq!(frame.method, "thread/turns/list");
                        let params: p::ThreadTurnsListParams =
                            serde_json::from_str(frame.params.get()).unwrap();
                        assert_eq!(params.limit, Some(1));
                        assert!(matches!(
                            params.items_view,
                            Some(p::TurnItemsView::NotLoaded)
                        ));
                        stream
                            .send(format!(
                                r#"{{"id":{},"result":{{"data":[{}],"nextCursor":null}}}}"#,
                                frame.id,
                                serde_json::to_string(&latest).unwrap()
                            ))
                            .await
                            .unwrap();
                    }
                    assert!(
                        stream.next().await.is_none(),
                        "retry observation emitted a remote write"
                    );
                });
                let failure = provider.latest_retryable_failure("thread").await.unwrap();
                assert_eq!(
                    failure.map(|f| f.turn_id),
                    (status == "failed").then(|| "latest".into())
                );
                drop(provider);
                tokio::time::timeout(std::time::Duration::from_secs(2), server)
                    .await
                    .unwrap()
                    .unwrap();
            }
        }
    }

    #[test]
    fn known_messages_and_native_codes_are_classified() {
        for (message, reason) in [
            ("Concurrency limit reached", RetryReason::ConcurrencyLimit),
            (
                "Our servers are currently overloaded. Please try again later.",
                RetryReason::ServerOverloaded,
            ),
            ("Rate limit reached for model", RetryReason::RateLimit),
            (
                "unexpected status 502 Bad Gateway",
                RetryReason::ServerError,
            ),
            (
                "stream disconnected before completion",
                RetryReason::Transport,
            ),
        ] {
            assert_eq!(classify_message(message), Some(reason));
        }
        for (info, reason) in [
            (r#""serverOverloaded""#, RetryReason::ServerOverloaded),
            (r#""rateLimitExceeded""#, RetryReason::RateLimit),
            (r#""internalServerError""#, RetryReason::ServerError),
            (
                r#"{"httpConnectionFailed":{"httpStatusCode":503}}"#,
                RetryReason::ServerError,
            ),
            (
                r#"{"responseStreamDisconnected":{"httpStatusCode":null}}"#,
                RetryReason::Transport,
            ),
        ] {
            let error = serde_json::from_str(&format!(
                r#"{{"message":"opaque error","codexErrorInfo":{info}}}"#
            ))
            .unwrap();
            assert_eq!(classify(&error), Some(reason));
        }
    }

    #[test]
    fn permanent_errors_override_retry_sounding_messages() {
        for info in [
            r#""unauthorized""#,
            r#""usageLimitExceeded""#,
            r#""contextWindowExceeded""#,
            r#""badRequest""#,
            r#""sandboxError""#,
            r#""sessionBudgetExceeded""#,
            r#"{"httpConnectionFailed":{"httpStatusCode":401}}"#,
            r#"{"httpConnectionFailed":{"httpStatusCode":403}}"#,
            r#"{"httpConnectionFailed":{"httpStatusCode":400}}"#,
        ] {
            let error = serde_json::from_str(&format!(
                r#"{{"message":"Concurrency limit reached","codexErrorInfo":{info}}}"#
            ))
            .unwrap();
            assert_eq!(classify(&error), None);
        }
        for message in [
            "You've hit your usage limit. Try again later.",
            "insufficient_quota: too many requests",
            "context window exceeded",
            "invalid API key",
            "Please try again later",
            "unknown error",
        ] {
            assert_eq!(classify_message(message), None);
        }
    }
}
