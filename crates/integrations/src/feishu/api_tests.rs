use super::*;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path, query_param},
};

async fn api() -> (MockServer, Api) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .and(body_partial_json(
            json!({"app_id":"cli_example","app_secret":"synthetic-secret"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"code":0,"msg":"success","tenant_access_token":"fixture-token","expire":7200}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let api = Api::configured(&super::super::tests::settings(), &server.uri(), true).unwrap();
    (server, api)
}

#[tokio::test]
async fn sdk_auth_identity_and_thread_reply_use_the_documented_wire_shapes() {
    let (server, api) = api().await;
    Mock::given(method("GET"))
        .and(path("/open-apis/bot/v3/info"))
        .and(header("authorization", "Bearer fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"code":0,"msg":"success","bot":{"open_id":"ou_bot","app_name":"Fixture bot"}}),
        ))
        .mount(&server)
        .await;
    assert_eq!(api.identity().await.unwrap().open_id, "ou_bot");
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages/om_root/reply"))
        .and(header("authorization", "Bearer fixture-token"))
        .and(body_partial_json(
            json!({"msg_type":"interactive","reply_in_thread":true,"uuid":"one-operation"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"code":0,"data":{"message_id":"om_reply","thread_id":"omt_thread"}}),
        ))
        .mount(&server)
        .await;
    let mut card = Card::new("Fixture");
    card.markdown("**hello**".into());
    let reply = api
        .send(
            &Destination {
                chat: "oc_group".into(),
                root: Some("om_root".into()),
            },
            &card,
            "one-operation",
        )
        .await
        .unwrap();
    assert_eq!(reply.thread_id, "omt_thread");
    let requests = server.received_requests().await.unwrap();
    let body: SendBodyOwned = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    let rendered: Card = serde_json::from_str(&body.content).unwrap();
    assert!(rendered.bounded());
}

#[derive(Deserialize)]
struct SendBodyOwned {
    content: String,
}

#[tokio::test]
async fn resource_download_is_bound_to_message_and_file_key() {
    let (server, api) = api().await;
    Mock::given(method("GET"))
        .and(path(
            "/open-apis/im/v1/messages/om_input/resources/file_fixture",
        ))
        .and(query_param("type", "file"))
        .and(header("authorization", "Bearer fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fixture contents"))
        .expect(1)
        .mount(&server)
        .await;
    let file = session::files::Incoming {
        name: "../input.txt".into(),
        location: serde_json::to_string(&Resource {
            message: "om_input".into(),
            key: "file_fixture".into(),
            image: false,
        })
        .unwrap(),
        image: false,
    };
    let downloaded = api.download(&file).await.unwrap();
    assert_eq!(downloaded.name, "input.txt");
    assert_eq!(downloaded.bytes, b"fixture contents");
    let bad = session::files::Incoming {
        location: serde_json::to_string(&Resource {
            message: "../other".into(),
            key: "file_fixture".into(),
            image: false,
        })
        .unwrap(),
        ..file
    };
    assert!(api.download(&bad).await.is_err());
}

#[tokio::test]
async fn uncertain_sends_are_not_retried_by_the_http_client() {
    let (server, api) = api().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .and(query_param("receive_id_type", "chat_id"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        api.send(
            &Destination {
                chat: "oc_group".into(),
                root: None
            },
            &Card::new("Fixture"),
            "uncertain-operation"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn authenticated_downloads_never_follow_redirects() {
    let (server, api) = api().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"must not reach"))
        .expect(0)
        .mount(&other)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/open-apis/im/v1/messages/om_input/resources/file_fixture",
        ))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/leak", other.uri())),
        )
        .mount(&server)
        .await;
    let file = session::files::Incoming {
        name: "input".into(),
        location: serde_json::to_string(&Resource {
            message: "om_input".into(),
            key: "file_fixture".into(),
            image: false,
        })
        .unwrap(),
        image: false,
    };
    assert!(api.download(&file).await.is_err());
}

#[tokio::test]
async fn api_errors_expose_only_numeric_codes() {
    use i18n::LocalizedError;
    let (server, api) = api().await;
    Mock::given(method("GET"))
        .and(path("/open-apis/bot/v3/info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":99991672,"msg":"private diagnostic must not appear"})),
        )
        .mount(&server)
        .await;
    let error = api.identity().await.unwrap_err();
    assert!(matches!(error, Error::Api(99991672)));
    assert!(!error.localized("en").contains("private diagnostic"));
}
