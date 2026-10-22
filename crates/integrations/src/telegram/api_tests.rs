use super::*;

#[tokio::test]
async fn icon_catalog_failure_is_not_cached_or_reported_as_applied() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let calls = attempts.clone();
    Mock::given(path_regex("(?i).*/getforumtopiciconstickers$"))
        .respond_with(move |_: &wiremock::Request| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_body_string(r#"{"ok":false,"error_code":500,"description":"Internal Server Error"}"#)
            } else {
                ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[{"file_id":"f","file_unique_id":"u","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"💬","custom_emoji_id":"idle"}]}"#)
            }
        }).mount(&server).await;
    Mock::given(path_regex("(?i).*/editforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#))
        .expect(1)
        .mount(&server)
        .await;
    let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
    let appearance = crate::telegram::presentation::Appearance {
        title: "Synthetic title".into(),
        emoji: "💬".into(),
    };
    assert!(
        api.set_appearance(10, 11, &appearance, Some(&appearance.title))
            .await
            .is_err(),
        "an unavailable icon must not produce a success receipt"
    );
    assert!(matches!(
        api.set_appearance(10, 11, &appearance, Some(&appearance.title))
            .await,
        Err(Error::IconUnavailable(_))
    ));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "catalog failure must back off"
    );
    api.icons.lock().await.retry_at = None;
    api.set_appearance(10, 11, &appearance, Some(&appearance.title))
        .await
        .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    #[derive(Deserialize)]
    struct Edit {
        icon_custom_emoji_id: Option<String>,
    }
    let requests = server.received_requests().await.unwrap();
    let edit = requests
        .iter()
        .find(|r| r.url.path().to_lowercase().ends_with("/editforumtopic"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Edit>(&edit.body)
            .unwrap()
            .icon_custom_emoji_id
            .as_deref(),
        Some("idle")
    );
}

#[tokio::test]
async fn missing_icons_and_catalog_rate_limits_never_send_an_empty_icon_edit() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
    for response in [
        r#"{"ok":true,"result":[{"file_id":"f","file_unique_id":"u","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"📁","custom_emoji_id":"folder"}]}"#,
        r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 20","parameters":{"retry_after":20}}"#,
    ] {
        let server = MockServer::start().await;
        Mock::given(path_regex("(?i).*/getforumtopiciconstickers$"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path_regex("(?i).*/editforumtopic$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#),
            )
            .expect(0)
            .mount(&server)
            .await;
        let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
        let appearance = crate::telegram::presentation::Appearance {
            title: "Synthetic title".into(),
            emoji: "💬".into(),
        };
        for _ in 0..2 {
            let result = api
                .set_appearance(10, 11, &appearance, Some(&appearance.title))
                .await;
            if response.contains("429") {
                assert!(matches!(result, Err(Error::RetryAfter(seconds)) if seconds >= 20));
            } else {
                assert!(matches!(result, Err(Error::IconUnavailable(_))));
            }
        }
    }
}

#[tokio::test]
async fn production_poll_budget_survives_a_response_slower_than_the_sdk_default() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
    let server = MockServer::start().await;
    Mock::given(path_regex("(?i).*/getupdates$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"ok":true,"result":[]}"#)
                .set_delay(Duration::from_secs(18)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut api = Api::with_builder(
        "1:abcdefghijklmnopqrstuvwxyz".into(),
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    api.bot = api.bot.set_api_url(url::Url::parse(&server.uri()).unwrap());
    assert!(api.updates(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn identity_extension_keeps_the_token_in_the_url_path_and_reads_private_topics() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
    let server = MockServer::start().await;
    Mock::given(path_regex("(?i).*/getme$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"ok":true,"result":{"id":1,"username":"fixture","has_topics_enabled":true,"allows_users_to_create_topics":true}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path_regex("(?i).*/getwebhookinfo$")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{"url":"","has_custom_certificate":false,"pending_update_count":0}}"#)).expect(1).mount(&server).await;
    let api = Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &server.uri()).unwrap();
    let identity = api.identity().await.unwrap();
    assert!(identity.topics_enabled);
    assert!(identity.user_topic_creation_enabled);
    assert!(!identity.webhook_enabled);
    assert_eq!(identity.bot_id, 1);
}

#[test]
fn sdk_preserves_private_topic_and_denies_group_sender() {
    let private = r#"{"update_id":1,"message":{"message_id":7,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"User"},"chat":{"id":10,"type":"private","first_name":"User"},"text":"hello","is_topic_message":true}}"#;
    let update: Update = serde_json::from_str(private).unwrap();
    let event = input(&update, crate::telegram::Owner::new(10).unwrap()).unwrap();
    assert_eq!(event.topic, 11);
    assert_eq!(event.event_key, "message:10:7");
    let group = private.replace(
        "\"id\":10,\"type\":\"private\",\"first_name\":\"User\"",
        "\"id\":-100,\"type\":\"supergroup\",\"title\":\"Group\"",
    );
    assert!(
        input(
            &serde_json::from_str(&group).unwrap(),
            crate::telegram::Owner::new(10).unwrap()
        )
        .is_none()
    );
}

#[test]
fn media_inputs_keep_caption_and_durable_file_references_without_network() {
    let update = r#"{"update_id":1,"message":{"message_id":7,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"User"},"chat":{"id":10,"type":"private","first_name":"User"},"photo":[{"file_id":"private-file","file_unique_id":"unique","width":640,"height":480}],"caption":"read this caption"}}"#;
    let event = input(
        &serde_json::from_str(update).unwrap(),
        crate::telegram::Owner::new(10).unwrap(),
    )
    .unwrap();
    assert_eq!(event.text, "read this caption");
    assert_eq!(event.topic, 11);
    assert_eq!(event.attachments[0].name, "image");
    assert_eq!(event.uploads[0].location, "private-file");
    assert!(event.uploads[0].image);
    assert!(!format!("{event:?}").contains("private-file"));
    let photo_only = update.replace(",\"caption\":\"read this caption\"", "");
    assert!(
        input(
            &serde_json::from_str(&photo_only).unwrap(),
            crate::telegram::Owner::new(10).unwrap()
        )
        .unwrap()
        .text
        .is_empty()
    );
    assert!(no_preview().is_disabled);
}
