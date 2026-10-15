//! In-process protocol and Telegram fixtures; no real user account is contacted.
use super::*;
use codex::protocol as p;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use telegram::api::Api;
use tokio::{net::TcpStream, task::JoinHandle};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path_regex},
};

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
}
struct State {
    status: p::ThreadStatus,
    items: Vec<p::ThreadItem>,
    requests: Vec<&'static str>,
    drop_write: bool,
    legacy: bool,
    turn_changed_on_steer: bool,
    incompatible_history: bool,
}
impl State {
    fn thread(&self) -> p::Thread {
        let mut thread: p::Thread = serde_json::from_str(THREAD).unwrap();
        thread.status = self.status.clone();
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
        let config = Arc::new(ConfigFile::open(file).unwrap());
        let store = Store::open(&config.snapshot().config.service.data_dir)
            .await
            .unwrap();
        let api =
            Api::local_fixture("1:abcdefghijklmnopqrstuvwxyz".into(), &telegram.uri()).unwrap();
        let state=Arc::new(Mutex::new(State{status:p::ThreadStatus::Active(vec![]),items:vec![
            serde_json::from_str(r#"{"type":"userMessage","id":"u","content":[{"type":"text","text":"original question"}]}"#).unwrap(),
            serde_json::from_str(r#"{"type":"agentMessage","id":"a","text":"original answer","phase":"final_answer"}"#).unwrap(),
            serde_json::from_str(r#"{"type":"reasoning","id":"r","summary":["must never appear in Telegram"]}"#).unwrap(),
        ],requests:vec![],drop_write:false,legacy:false,turn_changed_on_steer:false,incompatible_history:false}));
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let state = server_state.clone();
                tokio::spawn(async move {
                    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                    while let Some(Ok(Message::Text(text))) = socket.next().await {
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
                            Method::Create(params) => {
                                assert_eq!(params.cwd.as_deref(), Some("/workspace"));
                                assert!(
                                    params.approval_policy.is_none() && params.sandbox.is_none()
                                );
                                state.requests.push("thread/start");
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
            config,
            connections: Arc::new(Connections::new(store.clone())),
            store,
            api,
            bot: 1,
            topics_enabled: true,
            pair_attempts: Mutex::new(HashMap::new()),
            history_slots: Semaphore::new(4),
            started_at: 0,
        });
        Mock::given(method("POST")).and(path_regex("(?i).*/createforumtopic$")).respond_with(|request:&wiremock::Request| {
            #[derive(Deserialize)] struct Topic {name:String}
            let topic:Topic=serde_json::from_slice(&request.body).unwrap();
            let id=if topic.name=="PocketPilot · 管理" {9}else{11};
            ResponseTemplate::new(200).set_body_string(format!(r#"{{"ok":true,"result":{{"message_thread_id":{id},"name":"fixture","icon_color":7322096}}}}"#))
        }).mount(&telegram).await;
        Mock::given(method("POST")).and(path_regex("(?i).*/(sendmessage|senddocument|editmessagetext)$")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":{"message_id":700,"date":100,"chat":{"id":10,"type":"private","first_name":"Fixture"},"text":"fixture"}}"#)).mount(&telegram).await;
        Mock::given(method("POST"))
            .and(path_regex("(?i).*/(editforumtopic|answercallbackquery)$"))
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
    async fn pair(&self) {
        let code = self
            .service
            .store
            .pairing()
            .request_pairing(1, 10, 10, true, now())
            .await
            .unwrap();
        self.service
            .store
            .pairing()
            .approve_pairing(&code, now())
            .await
            .unwrap();
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
fn input(id: u32, topic: i32, text: &str) -> Input {
    Input {
        update_id: id,
        event_key: format!("message:10:{id}"),
        user: 10,
        chat: 10,
        topic,
        date: now(),
        text: text.into(),
        callback: None,
        callback_id: None,
        message_id: id as i32,
        config_hash: None,
        requires_confirmation: false,
    }
}

#[tokio::test]
async fn failed_history_does_not_flip_a_known_runtime_or_repeatedly_rename_the_topic() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture.state.lock().await.incompatible_history = true;
    let (stop, receiver) = watch::channel(false);
    let delivery = tokio::spawn(fixture.service.clone().deliver(receiver));
    for _ in 0..3 {
        let binding = fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .unwrap();
        if let Err(error) = fixture.service.refresh_binding(&binding).await {
            fixture
                .service
                .offline_status(&binding, &error)
                .await
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM deliveries WHERE state IN ('pending','dispatching')",
            )
            .fetch_one(&fixture.service.store.pool)
            .await
            .unwrap();
            if pending == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "fixture delivery did not settle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    stop.send(true).unwrap();
    delivery.await.unwrap();
    let requests = fixture.telegram.received_requests().await.unwrap();
    let renames = requests
        .iter()
        .filter(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/editforumtopic")
        })
        .count();
    let binding = fixture
        .service
        .store
        .binding(1, 10, 11)
        .await
        .unwrap()
        .unwrap();
    assert!(
        renames <= 1,
        "history failure caused {renames} repeated topic renames"
    );
    assert!(
        binding.status_text.unwrap().contains("运行中"),
        "readable runtime was replaced by unknown"
    );
}

#[tokio::test]
async fn management_topic_is_separate_and_unbound_messages_never_reach_an_agent() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .handle(&input(200, 0, "/start"))
        .await
        .unwrap();
    assert_eq!(
        fixture.service.store.management_topic(1, 10).await.unwrap(),
        Some(9)
    );
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 9)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .service
            .store
            .bind(1, 10, 9, "instance", "thread")
            .await
            .is_err()
    );
    fixture
        .service
        .handle(&input(201, 0, "default user message"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(202, 9, "management user message"))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let rows = sqlx::query("SELECT topic_id,payload FROM deliveries ORDER BY rowid")
        .fetch_all(&fixture.service.store.pool)
        .await
        .unwrap();
    use sqlx::Row;
    for row in rows {
        let body: Outbound = serde_json::from_str(row.get("payload")).unwrap();
        if body.text.contains("default user message")
            || body.text.contains("management user message")
        {
            assert_eq!(row.get::<i32, _>("topic_id"), 9);
            assert!(body.text.contains("未发送给 agent"));
        }
    }
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(203, 11, "only this session input"))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    let requests = fixture.telegram.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r
                .url
                .path()
                .to_ascii_lowercase()
                .ends_with("/createforumtopic"))
            .count(),
        1
    );
}

#[tokio::test]
async fn picker_navigation_edits_the_same_management_message_and_keeps_its_buttons() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .handle(&input(210, 0, "/workspaces"))
        .await
        .unwrap();
    let button = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.starts_with("管理 ·"))
        .unwrap()
        .buttons[0][0]
        .data
        .clone();
    let mut click = input(211, 9, "");
    click.message_id = 777;
    click.callback = Some(button);
    fixture.service.handle(&click).await.unwrap();
    let sessions = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("选择已有会话"))
        .unwrap();
    assert_eq!(sessions.edit_message, Some(777));
    let back = sessions.buttons.last().unwrap()[0].data.clone();
    let (stop, receiver) = watch::channel(false);
    let deliver = tokio::spawn(fixture.service.clone().deliver(receiver));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let requests = fixture.telegram.received_requests().await.unwrap();
        if requests.iter().any(|r| {
            r.url
                .path()
                .to_ascii_lowercase()
                .ends_with("/editmessagetext")
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "inline menu was not edited");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop.send(true).unwrap();
    deliver.await.unwrap();
    #[derive(Deserialize)]
    struct Edit {
        message_id: i32,
        reply_markup: Markup,
    }
    #[derive(Deserialize)]
    struct Markup {
        inline_keyboard: Vec<Vec<serde::de::IgnoredAny>>,
    }
    let requests = fixture.telegram.received_requests().await.unwrap();
    let edit: Edit = serde_json::from_slice(
        &requests
            .iter()
            .find(|r| {
                r.url
                    .path()
                    .to_ascii_lowercase()
                    .ends_with("/editmessagetext")
            })
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(edit.message_id, 777);
    assert!(!edit.reply_markup.inline_keyboard.is_empty());
    let mut click = input(212, 9, "");
    click.message_id = 777;
    click.callback = Some(back);
    fixture.service.handle(&click).await.unwrap();
    let outputs = fixture.outputs().await;
    assert_eq!(outputs.last().unwrap().edit_message, Some(777));
}

#[tokio::test]
async fn workspace_buttons_use_two_columns_without_padding_small_session_lists() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let file = &fixture.service.config.path;
    let mut text = std::fs::read_to_string(file).unwrap();
    for (id, name) in [("second", "Second"), ("third", "Third")] {
        text.push_str(&format!("\n[[workspaces]]\nid='{id}'\nprovider_id='instance'\nname='{name}'\nshort_name='{id}'\ncwd='/workspace/{id}'\n"));
    }
    std::fs::write(file, text).unwrap();
    fixture.service.config.reload().unwrap();
    fixture
        .service
        .handle(&input(215, 0, "/workspaces"))
        .await
        .unwrap();
    let menu = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.starts_with("管理 ·"))
        .unwrap();
    assert_eq!(
        menu.buttons.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![2, 1]
    );
    fixture
        .service
        .handle(&input(216, 9, "/sessions workspace"))
        .await
        .unwrap();
    let menu = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("选择已有会话"))
        .unwrap();
    assert_eq!(
        menu.buttons.len(),
        2,
        "one real session and one back button, with no padded rows"
    );
    assert!(
        menu.buttons
            .iter()
            .all(|row| !row.is_empty() && row.iter().all(|b| !b.label.trim().is_empty()))
    );
}

#[tokio::test]
async fn picker_hides_only_sessions_proven_to_be_local_operator_probes() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let store = &fixture.service.store;
    let id = store
        .reserve_operation("probe:fixture:create", "instance", "__new")
        .await
        .unwrap();
    store
        .transition(
            &id,
            session::operation::OperationState::Received,
            session::operation::OperationState::Prepared,
        )
        .await
        .unwrap();
    store
        .transition(
            &id,
            session::operation::OperationState::Prepared,
            session::operation::OperationState::Dispatching,
        )
        .await
        .unwrap();
    store
        .accept_receipt(
            &id,
            &Receipt {
                thread: "thread".into(),
                turn: None,
            },
        )
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(220, 0, "/sessions workspace"))
        .await
        .unwrap();
    let menu = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("选择已有会话"))
        .unwrap();
    assert_eq!(
        menu.buttons.len(),
        1,
        "dedicated probe leaked into session buttons"
    );
    assert_eq!(menu.buttons[0][0].label, "返回工作区");
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
}

#[tokio::test]
async fn old_queued_title_updates_and_stale_keyboards_are_not_replayed_after_restart() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    let binding = store.binding(1, 10, 11).await.unwrap().unwrap();
    let mut status = Outbound::text("old status");
    status.topic_title = Some("old state in title".into());
    store.enqueue_status(&binding, &status).await.unwrap();
    let mut menu = Outbound::text("stale menu");
    menu.buttons = vec![vec![telegram::api::Button {
        label: "stale".into(),
        data: "stale".into(),
    }]];
    store.enqueue(1, 10, 0, "stale menu", &menu).await.unwrap();
    store.recover_inbox(1).await.unwrap();
    let pending = store.pending_deliveries(1, now()).await.unwrap();
    assert_eq!(pending.len(), 1);
    let body: Outbound = serde_json::from_str(&pending[0].payload).unwrap();
    assert!(body.topic_title.is_none());
    assert_eq!(body.text, "old status");
}

#[tokio::test]
async fn a_management_creation_with_a_lost_reply_is_not_retried_or_bound_to_an_agent() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.telegram.reset().await;
    Mock::given(method("POST"))
        .and(path_regex("(?i).*/createforumtopic$"))
        .respond_with(ResponseTemplate::new(200).set_body_string("incompatible fixture response"))
        .expect(1)
        .mount(&fixture.telegram)
        .await;
    assert!(
        fixture
            .service
            .handle(&input(230, 0, "/start"))
            .await
            .is_err()
    );
    assert!(matches!(
        fixture.service.handle(&input(231, 0, "/start")).await,
        Err(Error::Unknown)
    ));
    assert!(
        fixture
            .service
            .store
            .management_topic(1, 10)
            .await
            .unwrap()
            .is_none()
    );
    assert!(fixture.service.store.bindings(1).await.unwrap().is_empty());
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn queued_input_after_restart_requires_a_fresh_binding_scoped_confirmation() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    let store = &fixture.service.store;
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    store.set_workspace(1, 10, 11, "workspace").await.unwrap();
    let queued = input(70, 11, "queued while offline");
    let old_button = store
        .callback(
            1,
            &queued,
            &fixture.service.config.snapshot().hash,
            "old action",
            now(),
        )
        .await
        .unwrap();
    let (epoch, _) = store.cursor(1, now()).await.unwrap();
    store
        .ingest(1, epoch, &[(70, Some(queued.clone()))], now())
        .await
        .unwrap();
    store.recover_inbox(1).await.unwrap();
    let recovered = store.claim_input(1).await.unwrap().unwrap();
    assert!(recovered.input.requires_confirmation);
    assert_eq!(recovered.input.date, queued.date);
    fixture.service.handle(&recovered.input).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    let mut old_click = input(71, 11, "");
    old_click.callback = Some(old_button);
    assert!(matches!(
        fixture.service.handle(&old_click).await,
        Err(Error::Expired)
    ));
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("尚未发送"))
        .unwrap();
    assert!(draft.text.contains("queued while offline"));
    let mut confirm = input(72, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    fixture.service.handle(&confirm).await.unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    // A still-visible draft cannot be sent after unbinding/rebinding the topic.
    let mut delayed = input(73, 11, "second queued draft");
    delayed.requires_confirmation = true;
    fixture.service.handle(&delayed).await.unwrap();
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("second queued draft"))
        .unwrap();
    store.unbind(1, 10, 11).await.unwrap();
    store.bind(1, 10, 11, "instance", "thread").await.unwrap();
    let mut confirm = input(74, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    assert!(matches!(
        fixture.service.handle(&confirm).await,
        Err(Error::Expired)
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_message_older_than_the_new_process_is_a_draft_even_within_five_minutes() {
    let mut fixture = Fixture::new().await;
    let boot = now();
    Arc::get_mut(&mut fixture.service).unwrap().started_at = boot;
    fixture.pair().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    let mut queued = input(80, 11, "offline before boot");
    queued.date = boot - 1;
    assert!((0..=300).contains(&now().saturating_sub(queued.date)));
    fixture.service.handle(&queued).await.unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    assert!(
        fixture
            .outputs()
            .await
            .iter()
            .any(|b| b.text.contains("offline before boot"))
    );
}

#[tokio::test]
async fn updates_fetched_after_a_poll_outage_are_drafts_without_replaying_agent_input() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new().await;
    let attempts = AtomicUsize::new(0);
    Mock::given(method("POST")).and(path_regex("(?i).*/getupdates$"))
        .respond_with(move |_:&wiremock::Request| match attempts.fetch_add(1,Ordering::SeqCst) {
            0=>ResponseTemplate::new(500).set_body_string(r#"{"ok":false,"error_code":500,"description":"fixture outage"}"#),
            1=>ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[{"update_id":90,"message":{"message_id":90,"message_thread_id":11,"date":100,"from":{"id":10,"is_bot":false,"first_name":"Fixture"},"chat":{"id":10,"type":"private","first_name":"Fixture"},"text":"offline input","is_topic_message":true}}]}"#),
            _=>ResponseTemplate::new(200).set_body_string(r#"{"ok":true,"result":[]}"#).set_delay(Duration::from_millis(50)),
        }).mount(&fixture.telegram).await;
    let (stop, receiver) = watch::channel(false);
    let poll = tokio::spawn(fixture.service.clone().poll(receiver));
    let deadline = Instant::now() + Duration::from_secs(6);
    let recovered = loop {
        if let Some(input) = fixture.service.store.claim_input(1).await.unwrap() {
            break input;
        }
        assert!(
            Instant::now() < deadline,
            "fixture recovery update was not received"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    stop.send(true).unwrap();
    poll.await.unwrap();
    assert!(recovered.input.requires_confirmation);
    assert_eq!(recovered.input.text, "offline input");
    assert!(fixture.state.lock().await.requests.is_empty());
}

#[tokio::test]
async fn a_steer_race_preserves_text_and_waits_for_confirmation_before_starting_another_turn() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture.state.lock().await.turn_changed_on_steer = true;
    fixture
        .service
        .handle(&input(94, 11, "preserve this race input"))
        .await
        .unwrap();
    {
        let state = fixture.state.lock().await;
        assert_eq!(
            state
                .requests
                .iter()
                .filter(|r| **r == "turn/steer")
                .count(),
            1
        );
        assert!(!state.requests.contains(&"turn/start"));
        assert!(!state.items.iter().any(|i|matches!(i,p::ThreadItem::UserMessage{content,..} if content.iter().any(|c|matches!(c,p::UserInput::TextUserInput{text,..} if text=="preserve this race input")))));
    }
    let draft = fixture
        .outputs()
        .await
        .into_iter()
        .find(|b| b.text.contains("preserve this race input"))
        .unwrap();
    assert!(draft.text.contains("尚未发送"));
    let mut confirm = input(95, 11, "");
    confirm.callback = Some(draft.buttons[0][0].data.clone());
    fixture.service.handle(&confirm).await.unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/start")
            .count(),
        1
    );
}

#[cfg(feature = "discord")]
#[tokio::test]
async fn discord_prototype_shares_the_telegram_connection_actor_and_write_receipts() {
    use discord::{Backend, Command as DCommand, Input as DInput};
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
    let service = crate::discord_service::Service::local_fixture(
        fixture.service.config.clone(),
        fixture.service.store.clone(),
        fixture.service.connections.clone(),
        receiver,
    );
    let make = |id, channel, command| DInput {
        id,
        bot: 1,
        guild: 2,
        channel,
        user: 10,
        date: now(),
        command,
    };
    let open = make(
        1,
        7,
        DCommand::Open {
            workspace: "FX".into(),
            thread: "thread".into(),
        },
    );
    assert!(service.handle(&api, open).await.unwrap().contains("<#11>"));
    let history = fixture
        .service
        .store
        .discord_pending(1, 2, now())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].payload.contains("original question"));
    assert!(!history[0].payload.contains("must never appear"));
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| matches!(*r, "thread/resume" | "turn/start" | "turn/steer"))
    );
    let ask = make(
        2,
        11,
        DCommand::Ask {
            text: "continue".into(),
        },
    );
    service.handle(&api, ask.clone()).await.unwrap();
    service.handle(&api, ask).await.unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    let mut denied = make(
        3,
        11,
        DCommand::Ask {
            text: "unauthorized".into(),
        },
    );
    denied.user = 20;
    assert!(service.handle(&api, denied).await.is_err());
    fixture.state.lock().await.drop_write = true;
    let lost = make(
        4,
        11,
        DCommand::Ask {
            text: "lost receipt".into(),
        },
    );
    assert!(service.handle(&api, lost.clone()).await.is_err());
    assert!(service.handle(&api, lost).await.is_err());
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        2
    );
    service
        .handle(&api, make(5, 11, DCommand::Unbind))
        .await
        .unwrap();
    assert!(
        service
            .handle(
                &api,
                make(
                    6,
                    11,
                    DCommand::Ask {
                        text: "unbound".into()
                    }
                )
            )
            .await
            .is_err()
    );
    assert!(matches!(
        fixture.state.lock().await.status,
        p::ThreadStatus::Active(_)
    ));
}

#[tokio::test]
async fn private_pairing_topic_history_and_steer_form_a_durable_workflow() {
    let fixture = Fixture::new().await;
    fixture
        .service
        .handle(&input(1, 0, "/workspaces"))
        .await
        .unwrap();
    assert!(fixture.state.lock().await.requests.is_empty());
    fixture
        .service
        .handle(&input(2, 0, "/start"))
        .await
        .unwrap();
    assert!(
        !fixture
            .service
            .store
            .pairing()
            .is_paired(1, 10, 10)
            .await
            .unwrap()
    );
    fixture.pair().await;
    fixture
        .service
        .handle(&input(3, 0, "/sessions workspace"))
        .await
        .unwrap();
    let outputs = fixture.outputs().await;
    let button = outputs
        .last()
        .unwrap()
        .buttons
        .first()
        .unwrap()
        .first()
        .unwrap()
        .data
        .clone();
    let mut click = input(4, 9, "");
    click.callback = Some(button);
    fixture.service.handle(&click).await.unwrap();
    assert!(
        fixture
            .service
            .store
            .binding(1, 10, 11)
            .await
            .unwrap()
            .is_some()
    );
    let bodies = fixture.outputs().await;
    let history = bodies
        .iter()
        .find(|b| b.text.starts_with("会话历史"))
        .unwrap();
    assert!(history.text.contains("original question") && history.text.contains("original answer"));
    assert!(!history.text.contains("must never appear"));
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .any(|r| *r == "thread/resume" || *r == "turn/start")
    );
    fixture
        .service
        .handle(&input(5, 11, "follow up"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(5, 11, "follow up"))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    let (stop, receiver) = watch::channel(false);
    let sender = tokio::spawn(fixture.service.clone().deliver(receiver));
    tokio::time::sleep(Duration::from_millis(500)).await;
    stop.send(true).unwrap();
    sender.await.unwrap();
    let requests = fixture.telegram.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.url.path().to_ascii_lowercase().ends_with("sendmessage"))
    );
}

#[tokio::test]
async fn a_lost_write_receipt_cannot_cause_a_second_steer() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture.state.lock().await.drop_write = true;
    let event = input(1, 11, "uncertain steer");
    assert!(matches!(
        fixture.service.handle(&event).await,
        Err(Error::Unknown)
    ));
    assert!(matches!(
        fixture.service.handle(&event).await,
        Err(Error::Unknown)
    ));
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
}

#[tokio::test]
async fn new_session_requires_confirmation_and_an_unloaded_session_resumes_only_on_user_input() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .handle(&input(1, 0, "/new workspace first task"))
        .await
        .unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    let mut click = input(2, 9, "");
    click.callback = Some(
        fixture.outputs().await.last().unwrap().buttons[0][0]
            .data
            .clone(),
    );
    fixture.service.handle(&click).await.unwrap();
    assert!(
        fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/start")
    );
    fixture.state.lock().await.status = p::ThreadStatus::NotLoaded;
    fixture
        .service
        .handle(&input(3, 11, "/status"))
        .await
        .unwrap();
    assert!(
        !fixture
            .state
            .lock()
            .await
            .requests
            .contains(&"thread/resume")
    );
    fixture
        .service
        .handle(&input(4, 11, "resume task"))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "thread/resume")
            .count(),
        1
    );
}

#[tokio::test]
async fn waiting_approval_still_accepts_steer_without_answering_the_approval() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .store
        .bind(1, 10, 11, "instance", "thread")
        .await
        .unwrap();
    fixture
        .service
        .store
        .set_workspace(1, 10, 11, "workspace")
        .await
        .unwrap();
    fixture.state.lock().await.status =
        p::ThreadStatus::Active(vec![p::ThreadActiveFlag::WaitingOnApproval]);
    fixture
        .service
        .handle(&input(1, 11, "/steer investigate another file meanwhile"))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .state
            .lock()
            .await
            .requests
            .iter()
            .filter(|r| **r == "turn/steer")
            .count(),
        1
    );
    assert!(matches!(
        fixture.state.lock().await.status,
        p::ThreadStatus::Active(_)
    ));
}

#[tokio::test]
async fn display_and_rate_changes_preserve_authorized_live_outputs() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture
        .service
        .handle(&input(1, 0, "/open workspace thread"))
        .await
        .unwrap();
    let before = fixture.outputs().await;
    let history = before
        .iter()
        .find(|b| b.text.starts_with("会话历史"))
        .unwrap();
    let scope = history.scope.as_ref().unwrap();
    let path = fixture.service.config.path.clone();
    let contents = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Fixture workspace", "Renamed workspace")
        .replace(
            "telegram_messages_per_minute=60000",
            "telegram_messages_per_minute=100",
        );
    std::fs::write(path, contents).unwrap();
    fixture.service.config.reload().unwrap();
    assert!(
        fixture
            .service
            .scope_allowed(&fixture.service.config.snapshot().config, scope)
    );
    fixture
        .service
        .handle(&input(2, 11, "/rename new display name"))
        .await
        .unwrap();
    let binding = fixture
        .service
        .store
        .binding(1, 10, 11)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history.binding_revision, Some(binding.revision));
}

#[tokio::test]
async fn declared_legacy_history_uses_only_read_and_steer_without_resume() {
    let fixture = Fixture::new().await;
    fixture.pair().await;
    fixture.state.lock().await.legacy = true;
    fixture
        .service
        .handle(&input(1, 0, "/open workspace thread"))
        .await
        .unwrap();
    fixture
        .service
        .handle(&input(2, 11, "/steer legacy follow up"))
        .await
        .unwrap();
    let state = fixture.state.lock().await;
    assert!(state.requests.contains(&"thread/read/full"));
    assert!(state.requests.contains(&"turn/steer"));
    assert!(!state.requests.contains(&"thread/turns/list"));
    assert!(!state.requests.contains(&"thread/resume"));
}
