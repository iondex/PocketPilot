use super::*;
use session::Role;
use std::os::unix::fs::PermissionsExt;

async fn runtime() -> (tempfile::TempDir, Arc<Runtime>) {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("fake-claude");
    std::fs::write(&binary,r#"#!/usr/bin/env python3
import sys,json,time,pathlib
assert '--permission-mode' in sys.argv and 'manual' in sys.argv
assert '--permission-prompt-tool' in sys.argv and 'stdio' in sys.argv
assert not any('bypassPermissions' in x for x in sys.argv)
thread=next(x.split('=',1)[1] for x in sys.argv if x.startswith(('--session-id=','--resume=')))
pathlib.Path(__file__).with_suffix('.started').write_text('started')
for raw in sys.stdin:
    data=json.loads(raw)
    if data['type']=='control_response':
        response=data['response']
        assert response['request_id']=='native-question'
        assert response['subtype']=='success'
        assert response['response']['behavior']=='allow'
        updated=response['response']['updatedInput']
        assert updated['answers']=={'Pick language':'Rust'}
        assert updated['questions'][0]['extension']=='preserved'
        assert updated['extension']=={'keep':True}
        assert 'updatedPermissions' not in response['response']
        pathlib.Path(__file__).with_suffix('.answered').write_text('once')
        print(json.dumps({'type':'assistant','session_id':thread,'uuid':'question-final','message':{'content':[{'type':'text','text':'Received Rust'}]}}),flush=True)
        print(json.dumps({'type':'result','session_id':thread,'subtype':'success','is_error':False,'result':'Received Rust'}),flush=True)
    elif data['type']=='control_request':
        if pathlib.Path.cwd().name=='needs-question':
            print(json.dumps({'type':'control_request','request_id':'question','request':{'subtype':'can_use_tool','tool_name':'AskUserQuestion'}}),flush=True)
        print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':data['request_id'],'response':{}}}),flush=True)
    else:
        if data['message']['content']=='no-echo': sys.exit(0)
        data['session_id']=thread
        data['isReplay']=True
        if data['message']['content']=='wrong-echo': data['session_id']='other-session'
        if isinstance(data['message']['content'],list):
            assert data['message']['content'][1]['type']=='image'
            assert data['message']['content'][1]['source']['media_type']=='image/png'
        print(json.dumps(data),flush=True)
        if data['message']['content']=='wrong-echo': sys.exit(0)
        if data['message']['content'] in ('ask-user','ask-cancel','ask-approval'):
            request={'subtype':'can_use_tool','tool_name':'AskUserQuestion' if data['message']['content']!='ask-approval' else 'Bash','tool_use_id':'tool','input':{'questions':[{'question':'Pick language','header':'Language','options':[{'label':'Rust','description':'Native'}],'multiSelect':False,'extension':'preserved'}],'extension':{'keep':True}}}
            print(json.dumps({'type':'control_request','request_id':'native-question','request':request}),flush=True)
            if data['message']['content']=='ask-cancel':
                time.sleep(0.3)
                print(json.dumps({'type':'control_cancel_request','request_id':'native-question'}),flush=True)
            continue
        time.sleep(0.1)
        print(json.dumps({'type':'assistant','session_id':thread,'uuid':'answer','message':{'content':[{'type':'thinking','thinking':'never expose'},{'type':'text','text':'fixture answer'}]},'parent_tool_use_id':None}),flush=True)
        print(json.dumps({'type':'result','session_id':thread,'subtype':'success','is_error':False,'result':'fixture answer'}),flush=True)
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    let uploads = directory.path().join("uploads");
    private_directory(&uploads).unwrap();
    let runtime = Arc::new(Runtime {
        store,
        binary,
        roots: vec![directory.path().to_owned()],
        uploads,
        drivers: Mutex::new(HashMap::new()),
    });
    (directory, runtime)
}

async fn create(runtime: &Runtime, path: &Path, operation: &str) -> Receipt {
    runtime
        .mutate(
            operation,
            Mutation::Create {
                cwd: path.to_str().unwrap().into(),
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn browsing_is_read_only_and_projects_are_isolated() {
    let (directory, runtime) = runtime().await;
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let one = create(&runtime, &first, "one").await;
    create(&runtime, &second, "two").await;
    let page = runtime
        .store
        .list(session::SessionQuery {
            cwd: Some(first.to_str().unwrap().into()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].reference.thread, one.thread);
    assert!(
        runtime
            .store
            .history(&one.thread)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(!runtime.binary.with_extension("started").exists());
    assert!(matches!(
        runtime
            .mutate(
                "steer",
                Mutation::Steer {
                    thread: one.thread,
                    expected_turn: "turn".into(),
                    text: "text".into(),
                    client_id: "input".into()
                }
            )
            .await,
        Err(Failure::Unsupported)
    ));
}

#[tokio::test]
async fn receipts_are_durable_and_conflicting_reuse_is_rejected() {
    let (directory, runtime) = runtime().await;
    let first = create(&runtime, directory.path(), "same-operation").await;
    let again = create(&runtime, directory.path(), "same-operation").await;
    assert_eq!(first.thread, again.thread);
    assert!(matches!(
        runtime
            .mutate(
                "same-operation",
                Mutation::Resume {
                    thread: first.thread
                }
            )
            .await,
        Err(Failure::Conflict)
    ));
    assert!(
        runtime
            .store
            .claim("ambiguous", "same")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        runtime.store.claim("ambiguous", "same").await,
        Err(Failure::Unknown)
    ));
}

#[tokio::test]
async fn proxy_disconnect_does_not_own_the_native_turn() {
    let (directory, runtime) = runtime().await;
    let receipt = create(&runtime, directory.path(), "create").await;
    let (server, peer) = UnixStream::pair().unwrap();
    let server_task = tokio::spawn(client(server, runtime.clone()));
    let mut peer = Framed::new(
        peer,
        LinesCodec::new_with_max_length(server::MAX_FRAME_BYTES),
    );
    peer.send(
        serde_json::to_string(&Request {
            id: 1,
            call: Call::Initialize {
                version: server::VERSION,
                runtime: "claude_code".into(),
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    peer.next().await.unwrap().unwrap();
    let mutation = Mutation::Start {
        thread: receipt.thread.clone(),
        text: "fixture task".into(),
        client_id: "client-one".into(),
    };
    peer.send(
        serde_json::to_string(&Request {
            id: 2,
            call: Call::Mutate {
                operation: "start".into(),
                mutation: mutation.clone(),
            },
        })
        .unwrap(),
    )
    .await
    .unwrap();
    drop(peer);
    // Wait for the owned fake process, never an actual Claude model request.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if runtime
                .store
                .history(&receipt.thread)
                .await
                .unwrap()
                .items
                .iter()
                .any(|i| i.phase.as_deref() == Some("final_answer"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let accepted = runtime.mutate("start", mutation).await.unwrap();
    assert!(accepted.turn.is_some());
    let history = runtime.store.history(&receipt.thread).await.unwrap();
    assert_eq!(
        history
            .items
            .iter()
            .filter(|i| i.role == Role::User)
            .count(),
        1
    );
    assert!(history.items.iter().any(|i| i.text == "fixture answer"));
    assert!(
        !history
            .items
            .iter()
            .any(|i| i.text.contains("never expose"))
    );
    let _ = server_task.await;
}

#[tokio::test]
async fn missing_native_echo_stays_unknown() {
    let (directory, runtime) = runtime().await;
    let receipt = create(&runtime, directory.path(), "create").await;
    let mutation = Mutation::Start {
        thread: receipt.thread,
        text: "no-echo".into(),
        client_id: "input".into(),
    };
    assert!(matches!(
        runtime.mutate("start", mutation.clone()).await,
        Err(Failure::Unknown)
    ));
    assert!(matches!(
        runtime.mutate("start", mutation).await,
        Err(Failure::Unknown)
    ));
}

#[tokio::test]
async fn uploaded_paths_cannot_escape_or_cross_sessions() {
    let (directory, runtime) = runtime().await;
    let first = create(&runtime, directory.path(), "first").await;
    let second = create(&runtime, directory.path(), "second").await;
    let path = runtime
        .uploads
        .join("source/0/input.txt")
        .to_str()
        .unwrap()
        .to_owned();
    runtime
        .mutate(
            "upload",
            Mutation::Upload {
                thread: first.thread.clone(),
                path: path.clone(),
                bytes: b"data".to_vec(),
            },
        )
        .await
        .unwrap();
    assert!(runtime.store.check_file(&first.thread, &path).await.is_ok());
    assert!(matches!(
        runtime.store.check_file(&second.thread, &path).await,
        Err(Failure::Unauthorized)
    ));
    let escaped = runtime
        .uploads
        .join("../outside")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(matches!(
        runtime.upload(&first.thread, &escaped, b"bad").await,
        Err(Failure::Unauthorized)
    ));
    assert!(!directory.path().join("outside").exists());
}

#[tokio::test]
async fn echoed_input_must_match_the_native_session() {
    let (directory, runtime) = runtime().await;
    let receipt = create(&runtime, directory.path(), "create").await;
    assert!(matches!(
        runtime
            .mutate(
                "start",
                Mutation::Start {
                    thread: receipt.thread.clone(),
                    text: "wrong-echo".into(),
                    client_id: "input".into()
                }
            )
            .await,
        Err(Failure::Unknown)
    ));
    assert!(
        runtime
            .store
            .history(&receipt.thread)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn image_input_uses_native_blocks_and_preserves_attachment_history() {
    let (directory, runtime) = runtime().await;
    let receipt = create(&runtime, directory.path(), "create").await;
    let path = runtime
        .uploads
        .join("source/0/input.png")
        .to_str()
        .unwrap()
        .to_owned();
    runtime
        .upload(&receipt.thread, &path, b"\x89PNG\r\n\x1a\nfixture")
        .await
        .unwrap();
    runtime
        .mutate(
            "start",
            Mutation::StartWithFiles {
                thread: receipt.thread.clone(),
                text: "describe fixture".into(),
                client_id: "image".into(),
                files: vec![session::files::Uploaded {
                    name: "input.png".into(),
                    path,
                    image: true,
                }],
            },
        )
        .await
        .unwrap();
    let history = runtime.store.history(&receipt.thread).await.unwrap();
    assert_eq!(history.items[0].attachments[0].name, "input.png");
}

#[tokio::test]
async fn a_question_during_initialization_cannot_be_overwritten_as_idle() {
    let (directory, runtime) = runtime().await;
    let project = directory.path().join("needs-question");
    std::fs::create_dir_all(&project).unwrap();
    let receipt = create(&runtime, &project, "create").await;
    runtime
        .mutate(
            "resume",
            Mutation::Resume {
                thread: receipt.thread.clone(),
            },
        )
        .await
        .unwrap();
    let state = runtime.store.read(&receipt.thread).await.unwrap();
    assert!(matches!(
        state.status,
        RuntimeStatus::Active {
            waiting_on_user_input: true,
            ..
        }
    ));
    assert!(matches!(
        runtime
            .mutate(
                "start",
                Mutation::Start {
                    thread: receipt.thread.clone(),
                    text: "normal input".into(),
                    client_id: "input".into()
                }
            )
            .await,
        Err(Failure::Busy)
    ));
    assert!(
        runtime
            .store
            .history(&receipt.thread)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

async fn ask(runtime: &Runtime, thread: &str, text: &str) -> Vec<session::questions::Dialog> {
    runtime
        .mutate(
            "start-question",
            Mutation::Start {
                thread: thread.into(),
                text: text.into(),
                client_id: "client-question".into(),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Reply::Questions { dialogs } = runtime
                .call(Call::Questions {
                    thread: thread.into(),
                })
                .await
                .unwrap()
            else {
                panic!("questions expected")
            };
            if !dialogs.is_empty() {
                break dialogs;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn native_question_answer_survives_proxy_replacement_and_has_a_durable_receipt() {
    let (directory, runtime) = runtime().await;
    let created = create(&runtime, directory.path(), "create-question").await;
    let dialogs = ask(&runtime, &created.thread, "ask-user").await;
    let submission = session::questions::Submission {
        token: dialogs[0].token.clone(),
        answers: vec![session::questions::Answer {
            question: "Pick language".into(),
            values: vec!["Rust".into()],
        }],
    };
    let mutation = Mutation::Answer {
        thread: created.thread.clone(),
        submission,
    };
    // A new RPC client can inspect and answer the process-owned question; dropping it
    // later cannot cancel native execution. The shared server state owns the token.
    let (server, peer) = UnixStream::pair().unwrap();
    let task = tokio::spawn(client(server, runtime.clone()));
    let mut peer = Framed::new(peer, LinesCodec::new());
    for (id, call) in [
        (
            1,
            Call::Initialize {
                version: server::VERSION,
                runtime: "claude_code".into(),
            },
        ),
        (
            2,
            Call::Questions {
                thread: created.thread.clone(),
            },
        ),
        (
            3,
            Call::Mutate {
                operation: "answer-question".into(),
                mutation: mutation.clone(),
            },
        ),
    ] {
        peer.send(serde_json::to_string(&Request { id, call }).unwrap())
            .await
            .unwrap();
        let response: Response =
            serde_json::from_str(&peer.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(response.id, id);
        assert!(response.result.is_ok());
    }
    drop(peer);
    task.await.unwrap().unwrap();
    let again = runtime
        .mutate("answer-question", mutation.clone())
        .await
        .unwrap();
    assert_eq!(again.thread, created.thread);
    assert!(matches!(
        runtime.mutate("duplicate-answer", mutation).await,
        Err(Failure::Conflict)
    ));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if runtime
                .store
                .history(&created.thread)
                .await
                .unwrap()
                .items
                .iter()
                .any(|i| i.text == "Received Rust")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(runtime.binary.with_extension("answered")).unwrap(),
        "once"
    );
}

#[tokio::test]
async fn cancelled_native_question_never_writes_an_answer() {
    let (directory, runtime) = runtime().await;
    let created = create(&runtime, directory.path(), "create-question").await;
    let dialogs = ask(&runtime, &created.thread, "ask-cancel").await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Reply::Questions { dialogs } = runtime
                .call(Call::Questions {
                    thread: created.thread.clone(),
                })
                .await
                .unwrap()
            else {
                panic!("questions expected")
            };
            if dialogs.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        runtime
            .mutate(
                "cancelled-answer",
                Mutation::Answer {
                    thread: created.thread,
                    submission: session::questions::Submission {
                        token: dialogs[0].token.clone(),
                        answers: vec![session::questions::Answer {
                            question: "Pick language".into(),
                            values: vec!["Rust".into()]
                        }]
                    }
                }
            )
            .await,
        Err(Failure::Conflict)
    ));
    assert!(!runtime.binary.with_extension("answered").exists());
}
