use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

struct MenuBackend(AtomicUsize);

#[async_trait]
impl Backend for MenuBackend {
    async fn handle(&self, _: &Api, _: Input) -> Result<String, i18n::Message> {
        panic!("navigation reached a native operation")
    }

    async fn new_session_form(
        &self,
        _: &Api,
        input: Input,
        token: &str,
    ) -> Result<crate::discord::ui::NewSessionForm, i18n::Message> {
        assert_eq!(input.user, 10);
        assert_eq!(token, "fixture-form");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(crate::discord::ui::NewSessionForm {
            token: "fixture-draft".into(),
            title: "New session".into(),
        })
    }

    async fn menu(
        &self,
        _: &Api,
        input: Input,
        token: Option<String>,
        _: Option<String>,
    ) -> Result<crate::discord::ui::Screen, i18n::Message> {
        assert_eq!(input.user, 10);
        assert_eq!(input.channel, 7);
        self.0.fetch_add(1, Ordering::SeqCst);
        let mut screen = crate::discord::ui::Screen::new(
            if token.is_some() {
                "**📁 选择项目**"
            } else {
                "**🌐 选择工作空间**"
            }
            .into(),
        );
        screen.buttons(vec![crate::discord::ui::button("fixture-handle", "刷新")]);
        Ok(screen)
    }
}

fn message() -> Message {
    let mut message = Message::default();
    message.id = MessageId::new(30);
    message.channel_id = ChannelId::new(7);
    message.author.id = UserId::new(1);
    message.flags = Some(MessageFlags::EPHEMERAL);
    message
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path_regex(".*/guilds/2$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"2","name":"Fixture","owner_id":"10","verification_level":0,"default_message_notifications":0,"explicit_content_filter":0,"roles":[],"emojis":[],"features":[],"mfa_level":0,"system_channel_flags":0,"premium_tier":0,"preferred_locale":"en-US","nsfw_level":0,"stickers":[],"premium_progress_bar_enabled":false}"#))
        .mount(&server).await;
    Mock::given(method("POST"))
        .and(path_regex(".*/interactions/.*/fixture-token/callback$"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path_regex(
            ".*/webhooks/1/fixture-token/messages/@original$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(message()))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn the_short_pp_command_opens_one_private_native_menu() {
    let server = server().await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    api.http.set_application_id(ApplicationId::new(1));
    let backend = Arc::new(MenuBackend(AtomicUsize::new(0)));
    let handler = Handler {
        api,
        backend: backend.clone(),
    };
    let command: CommandInteraction = serde_json::from_value(serde_json::json!({
        "id":"100", "application_id":"1", "guild_id":"2", "channel_id":"7", "token":"fixture-token", "version":1,
        "data":{"id":"90","name":"pp","type":1}, "user":{"id":"10","username":"Fixture","discriminator":"0000","avatar":null},
        "locale":"zh-CN", "entitlements":[], "attachment_size_limit":20971520
    })).unwrap();
    handler.command(&handler.api.http, command).await;
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let ack: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(ack["type"], 5);
    assert_eq!(ack["data"]["flags"], 64);
    let update: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(
        update["components"][0]["components"][0]["content"]
            .as_str()
            .unwrap()
            .contains("🌐")
    );
    assert_eq!(
        update["components"][0]["components"][1]["components"][0]["label"],
        "刷新"
    );
    assert_eq!(update["allowed_mentions"]["parse"], serde_json::json!([]));
}

#[tokio::test]
async fn components_acknowledge_and_edit_the_original_menu_without_posting_new_messages() {
    let server = server().await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    api.http.set_application_id(ApplicationId::new(1));
    let backend = Arc::new(MenuBackend(AtomicUsize::new(0)));
    let handler = Handler {
        api,
        backend: backend.clone(),
    };
    let mut interaction: ComponentInteraction = serde_json::from_value(serde_json::json!({
        "id":"100", "application_id":"1", "guild_id":"2", "channel_id":"7", "token":"fixture-token", "version":1,
        "data":{"custom_id":"pp:fixture-handle", "component_type":2}, "message": message(),
        "user":{"id":"10","username":"Fixture","discriminator":"0000","avatar":null},
        "locale":"zh-CN", "entitlements":[], "attachment_size_limit":20971520
    })).unwrap();
    handler.component(&handler.api.http, &interaction).await;
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let ack: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(ack["type"], 6);
    assert_eq!(requests[1].method, "PATCH");
    assert!(requests[1].url.path().ends_with("/messages/@original"));
    interaction.message.flags = None;
    handler.component(&handler.api.http, &interaction).await;
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn repeated_authorization_checks_reuse_the_short_lived_guild_owner_cache() {
    let server = server().await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    let input = Input {
        uploads: Vec::new(),
        locale: Some("en".into()),
        id: 100,
        bot: 1,
        guild: 2,
        channel: 7,
        user: 10,
        date: engine::now(),
        command: crate::discord::Command::Workspaces,
    };
    api.authorize(&input).await.unwrap();
    api.authorize(&input).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .count(),
        1
    );
}

#[tokio::test]
async fn creation_button_opens_a_modal_as_the_initial_response() {
    let server = server().await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    let backend = Arc::new(MenuBackend(AtomicUsize::new(0)));
    let handler = Handler {
        api,
        backend: backend.clone(),
    };
    let interaction: ComponentInteraction = serde_json::from_str(&format!(r#"{{"id":"100","application_id":"1","guild_id":"2","channel_id":"7","token":"fixture-token","version":1,"data":{{"custom_id":"pp-form:fixture-form","component_type":2}},"message":{},"user":{{"id":"10","username":"Fixture","discriminator":"0000","avatar":null}},"locale":"en-US","entitlements":[],"attachment_size_limit":20971520}}"#,serde_json::to_string(&message()).unwrap())).unwrap();
    handler.component(&handler.api.http, &interaction).await;
    #[derive(serde::Deserialize)]
    struct Response {
        #[serde(rename = "type")]
        kind: u8,
        data: ModalData,
    }
    #[derive(serde::Deserialize)]
    struct ModalData {
        custom_id: String,
        components: Vec<ModalRow>,
    }
    #[derive(serde::Deserialize)]
    struct ModalRow {
        components: Vec<TextField>,
    }
    #[derive(serde::Deserialize)]
    struct TextField {
        required: bool,
        max_length: u16,
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "opening a modal must not defer or call an agent"
    );
    let response: Response = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(response.kind, 9);
    assert_eq!(response.data.custom_id, "pp-create:fixture-draft");
    assert!(response.data.components[0].components[0].required);
    assert_eq!(response.data.components[0].components[0].max_length, 4000);
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn modal_submission_is_acknowledged_privately_before_rendering_confirmation() {
    let server = server().await;
    let api = Api::local_fixture(&server.uri(), 2, 1);
    api.http.set_application_id(ApplicationId::new(1));
    let backend = Arc::new(MenuBackend(AtomicUsize::new(0)));
    let handler = Handler {
        api,
        backend: backend.clone(),
    };
    let interaction:ModalInteraction=serde_json::from_str(r#"{"id":"101","application_id":"1","guild_id":"2","channel_id":"7","token":"fixture-token","version":1,"data":{"custom_id":"pp-create:fixture-draft","components":[{"type":1,"components":[{"type":4,"custom_id":"text","value":"A fixture task"}]}]},"user":{"id":"10","username":"Fixture","discriminator":"0000","avatar":null},"locale":"en-US","entitlements":[],"attachment_size_limit":20971520}"#).unwrap();
    handler.modal(&handler.api.http, &interaction).await;
    let requests = server.received_requests().await.unwrap();
    #[derive(serde::Deserialize)]
    struct Ack {
        #[serde(rename = "type")]
        kind: u8,
        data: Flags,
    }
    #[derive(serde::Deserialize)]
    struct Flags {
        flags: u64,
    }
    let ack: Ack = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(ack.kind, 5);
    assert_eq!(ack.data.flags, 64);
    assert_eq!(requests[1].method, "PATCH");
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
}
