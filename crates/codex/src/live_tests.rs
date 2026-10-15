//! Uses the installed Codex binary with an isolated home and a local model fixture.
//! Never connects to a user daemon, reads user credentials or calls a paid model.
use crate::{Client, Codex, Connection};
use session::{AgentProvider, RuntimeStatus, SessionQuery, provider::Mutation};
use std::{process::Stdio, time::Duration};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const SSE: &str = concat!(
    "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"fixture-response\"}}\n\n",
    "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"fixture-message\",\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final_answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"fixture OK\",\"annotations\":[]}]}}\n\n",
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture-response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n",
);

async fn connect(connection: &Connection) -> Result<Client, crate::Error> {
    Client::connect(
        connection.clone(),
        Duration::from_secs(2),
        Duration::from_secs(10),
    )
    .await
}

#[tokio::test]
async fn real_codex_shares_active_state_and_keeps_running_after_the_last_client_disconnects() {
    exercise(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn real_codex_proxy_disconnect_does_not_end_the_shared_turn() {
    exercise(true).await;
}

async fn exercise(proxy: bool) {
    // Optional compatibility check against another installed native binary. This
    // affects only this isolated fixture, never a configured/user daemon.
    let binary =
        std::env::var("CODEX_TEST_BIN").unwrap_or_else(|_| env!("CODEX_SCHEMA_BIN_PATH").into());
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(SSE)
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&model)
        .await;
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let canonical_cwd = cwd.path().canonicalize().unwrap();
    let configuration = format!(
        r#"
model = "gpt-5.4"
model_provider = "fixture"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
shell_snapshot = false
code_mode_host = false

[model_providers.fixture]
name = "Local lifecycle fixture"
base_url = "{}/v1"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
supports_websockets = false
"#,
        model.uri()
    );
    std::fs::write(home.path().join("config.toml"), configuration).unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    let endpoint = format!("ws://{address}");
    drop(reservation);
    let socket = home.path().join("control.sock");
    let connection = if proxy {
        Connection::LocalProxy {
            binary: binary.clone(),
            socket: socket.clone(),
        }
    } else {
        Connection::Websocket {
            endpoint: endpoint.clone(),
        }
    };
    let listen = if proxy {
        format!("unix://{}", socket.display())
    } else {
        endpoint.clone()
    };
    let log = std::fs::File::create(home.path().join("app-server.log")).unwrap();
    let mut server = Command::new(&binary)
        .args(["app-server", "--listen", &listen])
        .env("CODEX_HOME", home.path())
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_ACCESS_TOKEN")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .current_dir(cwd.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let client = loop {
        if let Ok(client) = connect(&connection).await {
            break client;
        }
        assert!(
            server.try_wait().unwrap().is_none(),
            "isolated Codex exited; fixture log: {}",
            home.path().display()
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "isolated Codex did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let writer = Codex::new("fixture".into(), client).with_control(true);
    let thread = writer
        .execute(Mutation::Create {
            cwd: canonical_cwd.to_string_lossy().into_owned(),
        })
        .await
        .unwrap()
        .thread;
    let turn = writer
        .execute(Mutation::Start {
            thread: thread.clone(),
            text: "Return fixture OK without using any tools".into(),
            client_id: "fixture-user".into(),
        })
        .await
        .unwrap()
        .turn
        .unwrap();
    let observer = Codex::new("fixture".into(), connect(&connection).await.unwrap());
    let metadata = observer
        .client()
        .call(&crate::protocol::ThreadReadParams {
            thread_id: thread.clone(),
            include_turns: Some(false),
        })
        .await
        .unwrap();
    eprintln!(
        "isolated native fixture history mode: {}",
        metadata.thread.history_mode
    );
    let active_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let observed = loop {
        let observed = observer.read(&thread).await.unwrap();
        if matches!(observed.status, RuntimeStatus::Active { .. })
            || tokio::time::Instant::now() >= active_deadline
        {
            break observed;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        matches!(observed.status, RuntimeStatus::Active { .. }),
        "status {:?}; fixture log: {}",
        observed.status,
        std::fs::read_to_string(home.path().join("app-server.log")).unwrap()
    );
    assert_eq!(observed.active_turn.as_deref(), Some(turn.as_str()));
    let list = observer
        .list(SessionQuery {
            cwd: Some(canonical_cwd.to_string_lossy().into_owned()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    if !list.data.iter().any(|t| t.reference.thread == thread) {
        let raw = observer
            .client()
            .call(&crate::protocol::ThreadReadParams {
                thread_id: thread.clone(),
                include_turns: Some(false),
            })
            .await
            .unwrap();
        let all = observer
            .client()
            .call(&crate::protocol::ThreadListParams {
                limit: Some(10),
                use_state_db_only: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        panic!(
            "workspace list missing newly active thread: {list:?}; read {raw:?}; unfiltered {all:?}"
        );
    }
    writer
        .execute(Mutation::Steer {
            thread: thread.clone(),
            expected_turn: turn.clone(),
            text: "Keep the final reply short".into(),
            client_id: "fixture-steer".into(),
        })
        .await
        .unwrap();
    drop(writer);
    assert_eq!(
        observer.read(&thread).await.unwrap().active_turn.as_deref(),
        Some(turn.as_str())
    );
    // The original subscriber and the independent reader both leave while the model
    // request is delayed. Reconnect through a fresh transport without resume/start.
    drop(observer);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let reconnected = Codex::new("fixture".into(), connect(&connection).await.unwrap());
    assert_eq!(
        reconnected
            .read(&thread)
            .await
            .unwrap()
            .active_turn
            .as_deref(),
        Some(turn.as_str())
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let state = reconnected.read(&thread).await.unwrap();
        if state.status == RuntimeStatus::Idle {
            break;
        }
        if state.status == RuntimeStatus::SystemError {
            let turns = reconnected
                .client()
                .call(&crate::protocol::ThreadTurnsListParams {
                    thread_id: thread.clone(),
                    cursor: None,
                    limit: Some(10),
                    sort_direction: None,
                    items_view: Some(crate::protocol::TurnItemsView::Full),
                })
                .await
                .unwrap();
            panic!(
                "fixture failed: {turns:?}; isolated log: {}",
                std::fs::read_to_string(home.path().join("app-server.log")).unwrap()
            );
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "fixture turn failed to finish: {:?}",
            state.status
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let history = reconnected.history(&thread).await.unwrap();
    assert!(history.all_pages_read);
    assert!(
        history
            .items
            .iter()
            .any(|i| i.text.contains("fixture OK") && i.role == session::Role::Agent)
    );
    // Cleanup only the isolated test server after its turn has completed.
    server.kill().await.unwrap();
    server.wait().await.unwrap();
}
