//! In-process protocol and Telegram fixtures; no real user account is contacted.
use super::*;

use crate::discord;

use crate::slack;
use crate::telegram::api::Api;
use crate::telegram::ledger::Outbound;
use adapters::codex::protocol as p;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use session::provider::Receipt;
use std::time::Duration;
use tokio::{net::TcpStream, sync::Mutex, task::JoinHandle, time::Instant};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

fn fixture_parser(
    path: &std::path::Path,
    text: &str,
) -> Result<engine::reload::ParsedConfig, engine::config::ConfigError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Root {
        #[serde(flatten)]
        runtime: engine::config::Config,
        telegram: Option<crate::telegram::config::Config>,
        #[serde(default)]
        providers: Vec<engine::config::Provider>,
        #[serde(default)]
        workspaces: Vec<engine::config::Workspace>,
    }
    let mut root: Root = toml::from_str(text).map_err(|_| engine::config::ConfigError::Parse)?;
    root.runtime.providers = root.providers;
    root.runtime.workspaces = root.workspaces;
    root.runtime.prepare(path, text)?;
    let restart_key = engine::digest(root.telegram.as_ref().map_or("", |t| t.token.as_str()));
    Ok(engine::reload::ParsedConfig {
        config: root.runtime,
        restart_key,
    })
}

const THREAD: &str = r#"{"id":"thread","sessionId":"thread","cliVersion":"0.162.1","projectId":null,"historyMode":"paginated","preview":"Fixture title","cwd":"/workspace","ephemeral":false,"source":"appServer","modelProvider":"fixture","createdAt":100,"updatedAt":100,"status":{"type":"idle"},"turns":[]}"#;

#[derive(Deserialize)]
struct Frame {
    id: Option<p::RequestId>,
    #[serde(flatten)]
    method: Method,
}

#[derive(Deserialize)]
#[serde(tag = "method", content = "params")]
enum Method {
    #[serde(rename = "fs/createDirectory")]
    CreateDirectory(p::FsCreateDirectoryParams),
    #[serde(rename = "fs/writeFile")]
    WriteFile(p::FsWriteFileParams),
    #[serde(rename = "initialize")]
    Initialize(p::InitializeParams),
    #[serde(rename = "initialized")]
    Initialized,
    #[serde(rename = "thread/loaded/list")]
    Loaded(p::ThreadLoadedListParams),
    #[serde(rename = "thread/list")]
    List(p::ThreadListParams),
    #[serde(rename = "thread/read")]
    Read(p::ThreadReadParams),
    #[serde(rename = "thread/turns/list")]
    Turns(p::ThreadTurnsListParams),
    #[serde(rename = "thread/start")]
    Create(p::ThreadStartParams),
    #[serde(rename = "thread/resume")]
    Resume(p::ThreadResumeParams),
    #[serde(rename = "turn/start")]
    Start(p::TurnStartParams),
    #[serde(rename = "turn/steer")]
    Steer(p::TurnSteerParams),
    #[serde(rename = "thread/name/set")]
    SetName(p::ThreadSetNameParams),
}

struct State {
    question: bool,
    answers: Vec<p::ToolRequestUserInputResponse>,
    uploads: Vec<p::FsWriteFileParams>,
    status: p::ThreadStatus,
    native_name: Option<String>,
    items: Vec<p::ThreadItem>,
    requests: Vec<&'static str>,
    drop_write: bool,
    legacy: bool,
    turn_changed_on_steer: bool,
    incompatible_history: bool,
    turn_error: Option<String>,
    full_history_reads: usize,
}

impl State {
    fn thread(&self) -> p::Thread {
        let mut thread: p::Thread = serde_json::from_str(THREAD).unwrap();
        thread.status = self.status.clone();
        thread.name = self.native_name.clone();
        if self.legacy {
            thread.history_mode = p::ThreadHistoryMode::Legacy;
        }
        thread
    }

    fn turn(&self) -> p::Turn {
        let mut turn: p::Turn = serde_json::from_str(
            r#"{"id":"turn","status":"inProgress","itemsView":"full","items":[]}"#,
        )
        .unwrap();
        turn.items = self.items.clone();
        turn.status = if matches!(self.status, p::ThreadStatus::Active(_)) {
            p::TurnStatus::InProgress
        } else {
            p::TurnStatus::Completed
        };
        if let Some(error) = &self.turn_error {
            turn.status = p::TurnStatus::Failed;
            turn.error = Some(
                serde_json::from_str(&format!(
                    r#"{{"message":{}}}"#,
                    serde_json::to_string(error).unwrap()
                ))
                .unwrap(),
            );
        }
        turn
    }
}

async fn reply<T: Serialize>(socket: &mut WebSocketStream<TcpStream>, id: p::RequestId, result: T) {
    #[derive(Serialize)]
    struct Response<T> {
        id: p::RequestId,
        result: T,
    }
    socket
        .send(Message::Text(
            serde_json::to_string(&Response { id, result })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
}

#[derive(Deserialize)]
struct SentRich {
    message_thread_id: Option<i32>,
    rich_message: crate::telegram::rich::Message,
}

fn sent_rich(request: &wiremock::Request) -> Option<SentRich> {
    request
        .url
        .path()
        .to_ascii_lowercase()
        .ends_with("/sendrichmessage")
        .then(|| serde_json::from_slice(&request.body).unwrap())
}

struct Fixture {
    _directory: tempfile::TempDir,
    telegram: MockServer,
    service: Arc<Service>,
    state: Arc<Mutex<State>>,
    _server: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self._server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let telegram = MockServer::start().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let file = directory.path().join("pocketpilot.toml");
        std::fs::write(
            &file,
            format!(
                r#"
[telegram]
owner_user_id=10
token="1:abcdefghijklmnopqrstuvwxyz"
[[hosts]]
id="host"
name="Fixture"
transport="websocket"
endpoint="{endpoint}"
allowed_workspace_roots=["/workspace"]
[[providers]]
id="instance"
host_id="host"
kind="codex"
connection_mode="existing_proxy"
compatibility_profile="fixture"
control_verified=true
[[workspaces]]
id="workspace"
provider_id="instance"
name="Fixture workspace"
short_name="FX"
cwd="/workspace"
[delivery]
telegram_messages_per_minute=60000
"#
            ),
        )
        .unwrap();
        let config = Arc::new(ConfigFile::open(file, fixture_parser).unwrap());
        let store = Store::open(
            &config.snapshot().config.service.data_dir,
            crate::telegram::Owner::new(10).unwrap(),
        )
        .await
        .unwrap();
        let api =
            Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &telegram.uri()).unwrap();
        let state=Arc::new(Mutex::new(State{question:false,answers:Vec::new(),uploads:Vec::new(),native_name:Some("Fixture title".into()),status:p::ThreadStatus::Active(vec![]),items:vec![
            serde_json::from_str(r#"{"type":"userMessage","id":"u","content":[{"type":"text","text":"original question"}]}"#).unwrap(),
            serde_json::from_str(r#"{"type":"agentMessage","id":"a","text":"original answer","phase":"final_answer"}"#).unwrap(),
            serde_json::from_str(r#"{"type":"reasoning","id":"r","summary":["must never appear in Telegram"]}"#).unwrap(),
        ],requests:vec![],drop_write:false,legacy:false,turn_changed_on_steer:false,incompatible_history:false,turn_error:None,full_history_reads:0}));
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = server_state.clone();
                tokio::spawn(async move {
                    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let mut question_sent = false;
                    while let Some(Ok(Message::Text(text))) = socket.next().await {
                        #[derive(Deserialize)]
                        struct Answered {
                            id: String,
                            result: p::ToolRequestUserInputResponse,
                        }
                        if let Ok(answer) = serde_json::from_str::<Answered>(&text) {
                            assert_eq!(answer.id, "native-question");
                            let mut state = state.lock().await;
                            state.question = false;
                            state.answers.push(answer.result);
                            continue;
                        }
                        let frame: Frame = serde_json::from_str(&text).unwrap();
                        let mut state = state.lock().await;
                        match frame.method {
                            Method::Initialize(params) => {
                                assert_eq!(params.client_info.name, "pocketpilot");
                                state.requests.push("initialize");
                                let response:p::InitializeResponse=serde_json::from_str(r#"{"codexHome":"/fixture","userAgent":"fixture","platformFamily":"unix","platformOs":"test"}"#).unwrap();
                                reply(&mut socket, frame.id.unwrap(), response).await;
                            }
                            Method::Initialized => state.requests.push("initialized"),
                            Method::Loaded(params) => {
                                assert!(params.limit.is_some());
                                state.requests.push("thread/loaded/list");
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::ThreadLoadedListResponse {
                                        data: vec!["thread".into()],
                                        next_cursor: None,
                                    },
                                )
                                .await;
                            }
                            Method::List(params) => {
                                assert!(params.use_state_db_only.unwrap_or(false));
                                state.requests.push("thread/list");
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::ThreadListResponse {
                                        data: vec![],
                                        next_cursor: None,
                                        backwards_cursor: None,
                                    },
                                )
                                .await;
                            }
                            Method::Read(params) => {
                                if state.question && !question_sent {
                                    question_sent = true;
                                    socket.send(Message::Text(r#"{"id":"native-question","method":"item/tool/requestUserInput","params":{"threadId":"thread","turnId":"turn","itemId":"question-tool","isBlocking":true,"questions":[{"id":"format","header":"Format","question":"Choose format","options":[{"label":"Brief","description":"Summary"},{"label":"Detailed","description":"Full text"}]}]}}"#.into())).await.unwrap();
                                }
                                assert_eq!(params.thread_id, "thread");
                                let mut thread = state.thread();
                                if params.include_turns == Some(true) {
                                    thread.turns = vec![state.turn()];
                                    state.requests.push("thread/read/full");
                                } else {
                                    state.requests.push("thread/read");
                                }
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::ThreadReadResponse { thread },
                                )
                                .await;
                            }
                            Method::Turns(params) => {
                                assert_eq!(params.thread_id, "thread");
                                state.requests.push("thread/turns/list");
                                if matches!(params.items_view, Some(p::TurnItemsView::Full)) {
                                    state.full_history_reads += 1;
                                }
                                let mut turn = state.turn();
                                if state.incompatible_history
                                    && matches!(params.items_view, Some(p::TurnItemsView::Full))
                                {
                                    turn.items_view = p::TurnItemsView::Summary;
                                }
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::ThreadTurnsListResponse {
                                        data: vec![turn],
                                        next_cursor: None,
                                        backwards_cursor: None,
                                    },
                                )
                                .await;
                            }
                            Method::CreateDirectory(params) => {
                                assert!(
                                    params.path.0.starts_with(
                                        "/private/account/.codex/pocketpilot/uploads/"
                                    )
                                );
                                state.requests.push("fs/createDirectory");
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    serde_json::from_str::<p::FsCreateDirectoryResponse>("{}")
                                        .unwrap(),
                                )
                                .await;
                            }
                            Method::WriteFile(params) => {
                                state.requests.push("fs/writeFile");
                                state.uploads.push(params);
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    serde_json::from_str::<p::FsWriteFileResponse>("{}").unwrap(),
                                )
                                .await;
                            }
                            Method::SetName(params) => {
                                assert_eq!(params.thread_id, "thread");
                                state.requests.push("thread/name/set");
                                state.native_name = Some(params.name);
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    serde_json::from_str::<p::ThreadSetNameResponse>("{}").unwrap(),
                                )
                                .await;
                            }
                            Method::Create(params) => {
                                assert_eq!(params.cwd.as_deref(), Some("/workspace"));
                                assert!(
                                    params.approval_policy.is_none() && params.sandbox.is_none()
                                );
                                state.requests.push("thread/start");
                                state.native_name = None;
                                state.items.clear();
                                state.status = p::ThreadStatus::Idle;
                                let response:p::ThreadStartResponse=serde_json::from_str(&format!(r#"{{"thread":{THREAD},"approvalPolicy":"on-request","approvalsReviewer":"user","cwd":"/workspace","model":"fixture","modelProvider":"fixture","sandbox":{{"type":"readOnly"}}}}"#)).unwrap();
                                reply(&mut socket, frame.id.unwrap(), response).await;
                            }
                            Method::Resume(params) => {
                                assert_eq!(params.thread_id, "thread");
                                assert_eq!(params.exclude_turns, Some(true));
                                assert!(
                                    params.model.is_none()
                                        && params.cwd.is_none()
                                        && params.approval_policy.is_none()
                                        && params.sandbox.is_none()
                                );
                                state.requests.push("thread/resume");
                                state.status = p::ThreadStatus::Idle;
                                let response:p::ThreadResumeResponse=serde_json::from_str(&format!(r#"{{"thread":{THREAD},"approvalPolicy":"on-request","approvalsReviewer":"user","cwd":"/workspace","model":"fixture","modelProvider":"fixture","sandbox":{{"type":"readOnly"}}}}"#)).unwrap();
                                reply(&mut socket, frame.id.unwrap(), response).await;
                            }
                            Method::Start(params) => {
                                assert!(
                                    params.approval_policy.is_none()
                                        && params.sandbox_policy.is_none()
                                        && params.model.is_none()
                                        && params.cwd.is_none()
                                );
                                state.requests.push("turn/start");
                                state.status = p::ThreadStatus::Active(vec![]);
                                let id = format!("u{}", state.items.len());
                                state.items.push(p::ThreadItem::UserMessage {
                                    id,
                                    content: params.input,
                                    client_id: params.client_user_message_id,
                                });
                                if state.drop_write {
                                    break;
                                }
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::TurnStartResponse { turn: state.turn() },
                                )
                                .await;
                            }
                            Method::Steer(params) => {
                                assert_eq!(params.expected_turn_id, "turn");
                                state.requests.push("turn/steer");
                                if state.turn_changed_on_steer {
                                    state.turn_changed_on_steer = false;
                                    state.status = p::ThreadStatus::Idle;

                                    #[derive(Serialize)]
                                    struct Rejection {
                                        id: p::RequestId,
                                        error: WireError,
                                    }

                                    #[derive(Serialize)]
                                    struct WireError {
                                        code: i64,
                                        message: &'static str,
                                    }
                                    socket
                                        .send(Message::Text(
                                            serde_json::to_string(&Rejection {
                                                id: frame.id.unwrap(),
                                                error: WireError {
                                                    code: -32600,
                                                    message: "no active turn to steer",
                                                },
                                            })
                                            .unwrap()
                                            .into(),
                                        ))
                                        .await
                                        .unwrap();
                                    continue;
                                }
                                let id = format!("u{}", state.items.len());
                                state.items.push(p::ThreadItem::UserMessage {
                                    id,
                                    content: params.input,
                                    client_id: params.client_user_message_id,
                                });
                                if state.drop_write {
                                    break;
                                }
                                reply(
                                    &mut socket,
                                    frame.id.unwrap(),
                                    p::TurnSteerResponse {
                                        turn_id: "turn".into(),
                                    },
                                )
                                .await;
                            }
                        }
                    }
                });
            }
        });
        let service = Arc::new(Service {
            presence: super::presence::Presence::default(),
            default_locale: "zh-CN".into(),
            config,
            runtimes: Arc::new(RuntimeRegistry::new(store.core().clone())),
            store,
            api,
            bot: 1,
            topics_enabled: true,
            history_slots: Semaphore::new(4),
            started_at: 0,
        });
        Mock::given(method("POST")).and(path_regex("(?i).*/createforumtopic$")).respond_with(|request:&wiremock::Request| {
            #[derive(Deserialize)] struct Topic {name:String}
            let topic:Topic=serde_json::from_slice(&request.body).unwrap();
            let id=if topic.name=="管理" {9}else{11};
            ResponseTemplate::new(200).set_body_string(format!(r#"{{"ok":true,"result":{{"message_thread_id":{id},"name":"fixture","icon_color":7322096}}}}"#))
        }).mount(&telegram).await;
        Mock::given(method("POST")).and(path_regex("(?i).*/(sendmessage|sendrichmessage|senddocument|editmessagetext)$")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{"message_id":700,"date":100,"chat":{"id":10,"type":"private","first_name":"Fixture"},"text":"fixture"}}"#)).mount(&telegram).await;
        Mock::given(method("POST"))
            .and(path_regex("(?i).*/(editforumtopic|answercallbackquery)$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#),
            )
            .mount(&telegram)
            .await;
        Mock::given(method("POST")).and(path_regex("(?i).*/getforumtopiciconstickers$"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[{"file_id":"f","file_unique_id":"u","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"⚡️","custom_emoji_id":"busy"},{"file_id":"g","file_unique_id":"v","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"💬","custom_emoji_id":"idle"},{"file_id":"h","file_unique_id":"w","width":100,"height":100,"type":"custom_emoji","is_animated":false,"is_video":false,"emoji":"📁","custom_emoji_id":"folder"}]}"#)).mount(&telegram).await;
        Mock::given(method("POST"))
            .and(path_regex("(?i).*/(sendchataction|deletemessage)$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":true}"#),
            )
            .mount(&telegram)
            .await;
        Self {
            _directory: directory,
            telegram,
            service,
            state,
            _server: server,
        }
    }

    async fn choose(&self, id: u32, number: &str) {
        let (page, message): (String, Option<i32>) = sqlx::query_as(
            "SELECT id,message_id FROM navigation WHERE bot_id=1 AND chat_id=10 AND topic_id=9",
        )
        .fetch_one(&self.service.store.pool)
        .await
        .unwrap();
        let menu = self
            .outputs()
            .await
            .into_iter()
            .rev()
            .find(|b| b.navigation_id.as_deref() == Some(page.as_str()))
            .unwrap();
        let suffix = if number == "0" { "b" } else { number };
        let data = menu
            .buttons
            .iter()
            .flatten()
            .find(|b| b.data == format!("nav:{page}:{suffix}"))
            .unwrap()
            .data
            .clone();
        let message = message.unwrap_or(id as i32 - 1);
        self.service
            .store
            .navigation_receipt(&page, message, now())
            .await
            .unwrap();
        let mut click = input(id, 9, "");
        click.message_id = message;
        click.date = 100; // Telegram keeps the menu's send date even after an edit.
        click.callback = Some(data);
        click.callback_id = Some(format!("fixture-{id}"));
        click.event_key = format!("callback:fixture-{id}");
        self.service.handle(&click).await.unwrap();
    }

    async fn deliver_pending(&self) {
        let (stop, receiver) = watch::channel(false);
        let worker = tokio::spawn(self.service.clone().deliver(receiver));
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM deliveries WHERE state IN ('pending','dispatching')",
            )
            .fetch_one(&self.service.store.pool)
            .await
            .unwrap();
            if pending == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "fixture delivery did not settle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stop.send(true).unwrap();
        worker.await.unwrap();
    }

    async fn outputs(&self) -> Vec<Outbound> {
        sqlx::query_scalar::<_, String>("SELECT payload FROM deliveries ORDER BY rowid")
            .fetch_all(&self.service.store.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|p| serde_json::from_str(&p).unwrap())
            .collect()
    }
}

fn menu_labels(body: &Outbound) -> String {
    body.buttons
        .iter()
        .flatten()
        .map(|b| b.label.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn detach_topic(store: &Store) {
    let manager =
        crate::bindings::Manager::new(store, crate::bindings::Scope::Telegram { bot: 1, chat: 10 });
    let target = crate::bindings::Target::Telegram { topic: 11 };
    let config = toml::from_str("").unwrap();
    let entry = manager.entry(&config, &target, "en").await.unwrap();
    manager
        .change(&target, entry.version, crate::bindings::Change::Detach)
        .await
        .unwrap();
}

fn input(id: u32, topic: i32, text: &str) -> Input {
    Input {
        update_id: id,
        locale: None,
        event_key: format!("message:10:{id}"),
        user: 10,
        chat: 10,
        topic,
        date: now(),
        text: text.into(),
        attachments: Vec::new(),
        callback: None,
        callback_id: None,
        message_id: id as i32,
        config_hash: None,
        requires_confirmation: false,
        uploads: Vec::new(),
        reply_to_message_id: None,
    }
}

async fn discord_fixture() -> (Fixture, MockServer, discord::Api, discord::runtime::Service) {
    use wiremock::matchers::body_string_contains;
    let fixture = Fixture::new().await;
    let http = MockServer::start().await;
    let guild = r#"{"id":"2","name":"Fixture","owner_id":"10","verification_level":0,"default_message_notifications":0,"explicit_content_filter":0,"roles":[],"emojis":[],"features":[],"mfa_level":0,"system_channel_flags":0,"premium_tier":0,"preferred_locale":"en-US","nsfw_level":0,"stickers":[],"premium_progress_bar_enabled":false}"#;
    Mock::given(method("GET"))
        .and(path_regex(".*/guilds/2$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(guild))
        .mount(&http)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(".*/guilds/2/channels$"))
        .and(body_string_contains("\"type\":4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"id":"9","guild_id":"2","type":4,"name":"Fixture category"}"#),
        )
        .with_priority(1)
        .mount(&http)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(".*/guilds/2/channels$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"id":"11","guild_id":"2","type":0,"name":"Fixture session"}"#),
        )
        .with_priority(10)
        .mount(&http)
        .await;
    let api = discord::Api::local_fixture(&http.uri(), 2, 1);
    let (_stop, receiver) = watch::channel(false);
    let service = discord::runtime::Service::local_fixture(
        fixture.service.config.clone(),
        fixture.service.store.core().clone(),
        fixture.service.runtimes.clone(),
        receiver,
    );
    (fixture, http, api, service)
}

fn visible_item(
    turn: &str,
    id: &str,
    role: session::Role,
    phase: Option<&str>,
    text: &str,
) -> session::TextItem {
    session::TextItem {
        turn_id: turn.into(),
        item_id: id.into(),
        role,
        phase: phase.map(str::to_owned),
        text: text.into(),
        complete: true,
        client_id: None,
        attachments: vec![],
    }
}

fn navigation_click(menu: &Outbound, id: u32, selection: &str, message: i32) -> Input {
    let mut click = input(id, 9, "");
    click.event_key = format!("callback:synthetic-{id}");
    click.callback_id = Some(format!("synthetic-{id}"));
    click.callback = Some(format!(
        "nav:{}:{selection}",
        menu.navigation_id.as_deref().unwrap()
    ));
    click.message_id = message;
    click.date = 100;
    click
}

#[path = "binding_menu_tests.rs"]
mod bindings;

#[path = "access_tests.rs"]
mod access;

#[path = "discord_creation_tests.rs"]
mod discord_creation;

#[path = "question_flow_tests.rs"]
mod question_flow_tests;

#[path = "service_history_tests.rs"]
mod service_history;

#[path = "service_navigation_tests.rs"]
mod service_navigation;

#[path = "service_recovery_tests.rs"]
mod service_recovery;

#[path = "service_input_tests.rs"]
mod service_input;

#[path = "cross_platform_tests.rs"]
mod cross_platform;

#[path = "service_topics_tests.rs"]
mod service_topics;
