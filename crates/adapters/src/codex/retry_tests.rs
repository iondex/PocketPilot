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
                crate::codex::Client::spawn(
                    Box::new(crate::codex::transport::JsonLines::new(read, write, None)),
                    std::time::Duration::from_secs(2),
                ),
            );
            let server = tokio::spawn(async move {
                let mut stream = Framed::new(remote, LinesCodec::new());
                let frame: Frame =
                    serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                assert_eq!(frame.method, "thread/read");
                let params: p::ThreadReadParams = serde_json::from_str(frame.params.get()).unwrap();
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

#[test]
fn transport_codes_use_the_structured_status_and_preserve_permanent_error_exclusions() {
    for code in [
        "httpConnectionFailed",
        "responseStreamConnectionFailed",
        "responseStreamDisconnected",
        "responseTooManyFailedAttempts",
    ] {
        for (status, reason) in [
            ("null", Some(RetryReason::Transport)),
            ("408", Some(RetryReason::Transport)),
            ("429", Some(RetryReason::RateLimit)),
            ("500", Some(RetryReason::ServerError)),
            ("503", Some(RetryReason::ServerError)),
            ("599", Some(RetryReason::ServerError)),
            ("400", None),
            ("401", None),
            ("403", None),
            ("404", None),
        ] {
            let error = serde_json::from_str(&format!(
                r#"{{"message":"opaque failure","codexErrorInfo":{{"{code}":{{"httpStatusCode":{status}}}}}}}"#
            ))
            .unwrap();
            assert_eq!(classify(&error), reason, "{code}: {status}");
        }
    }
    for info in [r#""serverOverloaded""#, r#""rateLimitExceeded""#] {
        let error = serde_json::from_str(&format!(
            r#"{{"message":"insufficient quota","codexErrorInfo":{info}}}"#
        ))
        .unwrap();
        assert_eq!(classify(&error), None);
    }
}
