use super::*;
use crate::AttachmentSource;
use serde::Serialize;

struct Offline;

#[async_trait::async_trait]
impl AttachmentSource for Offline {
    type Error = Error;

    async fn download(&self, _: &session::files::Incoming) -> Result<session::files::Downloaded> {
        Err(Error::Request)
    }
}

#[async_trait::async_trait]
impl Platform for Offline {
    async fn identity(&self) -> Result<super::super::api::Identity> {
        Err(Error::Request)
    }

    async fn send(
        &self,
        _: &Destination,
        _: &Card,
        _: &str,
    ) -> Result<super::super::api::NativeMessage> {
        Err(Error::Request)
    }

    async fn patch(&self, _: &str, _: &Card) -> Result<()> {
        Err(Error::Request)
    }
}

async fn service() -> (tempfile::TempDir, Arc<Service>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[service]\ndata_dir='data'\n").unwrap();
    let parser = |path: &std::path::Path, text: &str| {
        let mut config: engine::config::Config = toml::from_str(text).unwrap();
        config.prepare(path, text)?;
        Ok(engine::reload::ParsedConfig {
            config,
            restart_key: "fixture".into(),
        })
    };
    let config = Arc::new(engine::reload::ConfigFile::open(path, parser).unwrap());
    let core = engine::storage::Store::open(&directory.path().join("data"))
        .await
        .unwrap();
    let context = Context {
        config,
        store: core.clone(),
        runtimes: Arc::new(engine::runtimes::RuntimeRegistry::new(core.clone())),
    };
    let settings = super::super::tests::settings();
    let store = Store::new(core, &settings);
    let (_, stop) = watch::channel(false);
    let service = Arc::new(Service {
        settings,
        context,
        store,
        api: Arc::new(Offline),
        bot: "ou_bot".into(),
        cutoff: AtomicI64::new(engine::now() - 1),
        stop,
    });
    (directory, service)
}

#[derive(Serialize)]
struct Header<'a> {
    event_id: &'a str,
    event_type: &'a str,
    app_id: &'a str,
    tenant_key: &'a str,
    create_time: String,
}

fn message(id: &str, event_id: &str, owner: &str, tenant: &str) -> Vec<u8> {
    #[derive(Serialize)]
    struct Envelope<'a> {
        header: Header<'a>,
        event: MessageEvent<'a>,
    }

    #[derive(Serialize)]
    struct MessageEvent<'a> {
        sender: Sender<'a>,
        message: Message<'a>,
    }

    #[derive(Serialize)]
    struct Sender<'a> {
        sender_id: SenderId<'a>,
        sender_type: &'a str,
        tenant_key: &'a str,
    }

    #[derive(Serialize)]
    struct SenderId<'a> {
        open_id: &'a str,
    }

    #[derive(Serialize)]
    struct Message<'a> {
        message_id: &'a str,
        chat_id: &'a str,
        chat_type: &'a str,
        message_type: &'a str,
        content: &'a str,
        create_time: String,
    }

    serde_json::to_vec(&Envelope {
        header: Header {
            event_id,
            event_type: "im.message.receive_v1",
            app_id: "cli_example",
            tenant_key: tenant,
            create_time: engine::now().to_string(),
        },
        event: MessageEvent {
            sender: Sender {
                sender_id: SenderId { open_id: owner },
                sender_type: "user",
                tenant_key: tenant,
            },
            message: Message {
                message_id: id,
                chat_id: "oc_private",
                chat_type: "p2p",
                message_type: "text",
                content: r#"{"text":"/start"}"#,
                create_time: (engine::now() * 1000).to_string(),
            },
        },
    })
    .unwrap()
}

#[tokio::test]
async fn socket_authorizes_owner_and_tenant_before_durable_enqueue() {
    let (_directory, service) = service().await;
    assert!(
        !service
            .receive(&message(
                "om_denied",
                "event_one",
                "ou_other",
                "tenant_example"
            ))
            .await
            .unwrap()
    );
    assert!(
        !service
            .receive(&message(
                "om_tenant",
                "event_two",
                "ou_owner",
                "other_tenant"
            ))
            .await
            .unwrap()
    );
    assert!(service.store.next().await.unwrap().is_none());
    assert!(
        service
            .receive(&message(
                "om_allowed",
                "event_three",
                "ou_owner",
                "tenant_example"
            ))
            .await
            .unwrap()
    );
    assert!(
        !service
            .receive(&message(
                "om_allowed",
                "event_four",
                "ou_owner",
                "tenant_example"
            ))
            .await
            .unwrap()
    );
    let row = service.store.next().await.unwrap().unwrap();
    let input: Input = serde_json::from_str(&row.payload).unwrap();
    assert!(matches!(input.command, Command::Menu(Action::Home { .. })));
    service.process(&input, &row.config_hash).await.unwrap();
    let output = service.store.outgoing().await.unwrap().unwrap();
    assert!(output.binding.is_none());
    assert!(output.edit.is_none());
    let card: Card = serde_json::from_str(&output.content).unwrap();
    assert!(card.bounded());
}

fn group_message(
    id: &str,
    event: &str,
    kind: &str,
    content: &str,
    root: &str,
    topic: &str,
    mentioned: bool,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"header":{"event_id":event,"event_type":"im.message.receive_v1","app_id":"cli_example","tenant_key":"tenant_example","create_time":(engine::now()*1000).to_string()},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_owner"},"tenant_key":"tenant_example"},"message":{"message_id":id,"chat_id":"oc_group","chat_type":"group","message_type":kind,"content":content,"create_time":(engine::now()*1000).to_string(),"root_id":root,"thread_id":topic,"mentions":if mentioned {serde_json::json!([{"key":"@_user_1","id":{"open_id":"ou_bot"}}])}else{serde_json::json!([])}}}})).unwrap()
}

async fn seed_binding(service: &Service, project: &str, cwd: &str, endpoint: &str) -> Binding {
    let b = Binding {
        id: "fixture-binding".into(),
        chat: "oc_group".into(),
        root: "om_root".into(),
        topic: "omt_thread".into(),
        project: project.into(),
        instance: "runtime".into(),
        session: "session".into(),
        cwd: cwd.into(),
        endpoint: endpoint.into(),
        revision: 1,
        active: true,
        cursor: "{}".into(),
        ready: false,
        status: String::new(),
        state: "ready".into(),
    };
    service.store.reserve(&b).await.unwrap();
    service
        .store
        .provision(&b.id, Some(&b.root), Some(&b.topic), false)
        .await
        .unwrap();
    b
}

#[tokio::test]
async fn group_inputs_require_exact_root_thread_and_explicit_mentions_except_separate_files() {
    let (_directory, service) = service().await;
    seed_binding(&service, "project", "/projects/example", "endpoint").await;
    let text = r#"{"text":"@_user_1 task"}"#;
    assert!(
        !service
            .receive(&group_message(
                "om_no_at",
                "event_one",
                "text",
                text,
                "om_root",
                "omt_thread",
                false
            ))
            .await
            .unwrap()
    );
    assert!(
        !service
            .receive(&group_message(
                "om_wrong",
                "event_two",
                "text",
                text,
                "om_other",
                "omt_thread",
                true
            ))
            .await
            .unwrap()
    );
    assert!(
        !service
            .receive(&group_message(
                "om_wrong",
                "event_two",
                "text",
                text,
                "om_root",
                "omt_other",
                true
            ))
            .await
            .unwrap()
    );
    assert!(service.store.next().await.unwrap().is_none());
    assert!(
        service
            .receive(&group_message(
                "om_file",
                "event_three",
                "file",
                r#"{"file_key":"file_one","file_name":"../input.txt"}"#,
                "om_root",
                "omt_thread",
                false
            ))
            .await
            .unwrap()
    );
    let input: Input =
        serde_json::from_str(&service.store.next().await.unwrap().unwrap().payload).unwrap();
    assert_eq!(input.files[0].name, "input.txt");
    assert!(matches!(input.command,Command::Text(text) if text.is_empty()));
    let post = r#"{"title":"Question","content":[[{"tag":"text","text":"look "},{"tag":"at","user_id":"@_user_1"}],[{"tag":"img","image_key":"img_one"}]]}"#;
    assert!(
        service
            .receive(&group_message(
                "om_post",
                "event_four",
                "post",
                post,
                "om_root",
                "omt_thread",
                true
            ))
            .await
            .unwrap()
    );
    let input: Input =
        serde_json::from_str(&service.store.next().await.unwrap().unwrap().payload).unwrap();
    assert!(
        matches!(input.command,Command::Text(text) if text.contains("Question") && text.contains("look") && !text.contains("@_user_1"))
    );
    assert_eq!(input.files.len(), 1);
    assert!(input.files[0].image);
}

#[tokio::test]
async fn sdk_callback_ack_waits_for_durable_queue_and_accepts_microsecond_timestamps() {
    let (_directory, service) = service().await;
    let hash = service.context.config.snapshot().hash.clone();
    let nonce = service
        .store
        .action("oc_private", Action::Home { offset: 0 }, &hash)
        .await
        .unwrap();
    sqlx::query("UPDATE feishu_actions SET message='om_menu' WHERE nonce=?")
        .bind(&nonce)
        .execute(service.store.core.pool())
        .await
        .unwrap();
    let payload=serde_json::to_vec(&serde_json::json!({"header":{"event_id":"event_callback","event_type":"card.action.trigger","app_id":"cli_example","tenant_key":"tenant_example","create_time":(engine::now()*1_000_000).to_string()},"event":{"operator":{"open_id":"ou_owner","tenant_key":"tenant_example"},"context":{"open_message_id":"om_menu","open_chat_id":"oc_private"},"action":{"value":{"nonce":nonce}}}})).unwrap();
    let handler = super::super::socket::Handler {
        service: service.clone(),
        handle: tokio::runtime::Handle::current(),
    };
    let dispatcher = openlark_client::ws_client::EventDispatcherHandler::builder()
        .register_callback("card.action.trigger", handler)
        .unwrap()
        .build();
    let response = tokio::task::spawn_blocking(move || dispatcher.dispatch_with_response(&payload))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!response.is_empty());
    assert!(service.store.next().await.unwrap().is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn authorized_group_input_reaches_the_adapter_once_and_never_from_unmentioned_text() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, service) = service().await;
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let binary = directory.path().join("fake-runtime");
    std::fs::write(&binary,r#"#!/usr/bin/env python3
import json,sys,pathlib
session=json.loads(pathlib.Path(__file__).with_suffix('.session').read_text())
for line in sys.stdin:
    request=json.loads(line);call=request['call'];method=call['method']
    if method=='initialize': reply={'type':'initialized','version':call['version'],'runtime':'claude_code','steer':False}
    elif method=='read': reply={'type':'session','session':session}
    elif method=='history': reply={'type':'history','history':{'items':[],'all_pages_read':True}}
    elif method=='questions':
        dialog=pathlib.Path(__file__).with_suffix('.dialog')
        reply={'type':'questions','dialogs':[json.loads(dialog.read_text())] if dialog.exists() else []}
    elif method=='mutate':
        with pathlib.Path(__file__).with_suffix('.calls').open('a') as log: log.write(json.dumps(call['mutation'])+'\n')
        if call['mutation']['action']=='answer': pathlib.Path(__file__).with_suffix('.dialog').unlink()
        reply={'type':'receipt','receipt':{'thread':'session','turn':'turn'}}
    else: sys.exit(1)
    print(json.dumps({'id':request['id'],'result':{'Ok':reply}}),flush=True)
"#).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        binary.with_extension("session"),
        serde_json::to_vec(&session::Session {
            reference: session::SessionRef {
                instance: "runtime".into(),
                thread: "session".into(),
            },
            title: "Fixture".into(),
            native_title: Some("Fixture".into()),
            cwd: project.to_str().unwrap().into(),
            status: session::RuntimeStatus::Idle,
            active_turn: None,
            updated_at: None,
        })
        .unwrap(),
    )
    .unwrap();
    let mut config = service.context.config.snapshot().config.clone();
    config.hosts.push(
        toml::from_str(&format!(
            "id='host'\nname='Host'\ntransport='local_proxy'\nallowed_workspace_roots=[{:?}]",
            project.to_str().unwrap()
        ))
        .unwrap(),
    );
    config.providers.push(engine::config::Provider {
        id: "runtime".into(),
        host_id: "host".into(),
        kind: "claude_code".into(),
        connection_mode: "existing_proxy".into(),
        compatibility_profile: "pocketpilot-runtime-v2".into(),
        control_verified: true,
        endpoint: Some(adapters::ServerEndpoint {
            binary: binary.to_str().unwrap().into(),
            socket: directory
                .path()
                .join("runtime/server.sock")
                .to_str()
                .unwrap()
                .into(),
        }),
    });
    config.workspaces.push(engine::config::Workspace {
        id: "project".into(),
        provider_id: "runtime".into(),
        name: "Project".into(),
        short_name: "project".into(),
        cwd: project.to_str().unwrap().into(),
        additional_cwds: vec![],
        updated_at: None,
    });
    let endpoint = engine::runtimes::fingerprint(&config.hosts[0], &config.providers[0]);
    service.context.config.publish_catalog(config).unwrap();
    seed_binding(&service, "project", project.to_str().unwrap(), &endpoint).await;
    let content = r#"{"text":"@_user_1 fixture task"}"#;
    assert!(
        !service
            .receive(&group_message(
                "om_plain",
                "event_plain",
                "text",
                content,
                "om_root",
                "omt_thread",
                false
            ))
            .await
            .unwrap()
    );
    assert!(
        service
            .receive(&group_message(
                "om_input",
                "event_input",
                "text",
                content,
                "om_root",
                "omt_thread",
                true
            ))
            .await
            .unwrap()
    );
    assert!(
        !service
            .receive(&group_message(
                "om_input",
                "event_redelivery",
                "text",
                content,
                "om_root",
                "omt_thread",
                true
            ))
            .await
            .unwrap()
    );
    let row = service.store.next().await.unwrap().unwrap();
    let input: Input = serde_json::from_str(&row.payload).unwrap();
    service.process(&input, &row.config_hash).await.unwrap();
    service.process(&input, &row.config_hash).await.unwrap();
    let calls = std::fs::read_to_string(binary.with_extension("calls")).unwrap();
    assert_eq!(calls.lines().count(), 1);
    let mutation: session::provider::Mutation =
        serde_json::from_str(calls.lines().next().unwrap()).unwrap();
    assert!(
        matches!(mutation,session::provider::Mutation::Start {text,..} if text=="fixture task")
    );
    // The same authenticated group/thread input path handles structured answers;
    // no native steer or permission approval is substituted for the answer.
    let dialog = session::questions::Dialog {
        token: "fixture-answer-code".into(),
        turn: Some("turn".into()),
        questions: vec![session::questions::Question {
            id: "format".into(),
            header: "Format".into(),
            text: "Choose format".into(),
            options: vec![session::questions::OptionLabel {
                label: "Brief".into(),
                description: "Summary".into(),
            }],
            multiple: false,
            other: false,
            secret: false,
        }],
    };
    std::fs::write(
        binary.with_extension("dialog"),
        serde_json::to_vec(&dialog).unwrap(),
    )
    .unwrap();
    #[derive(Serialize)]
    struct Text<'a> {
        text: &'a str,
    }
    let content = serde_json::to_string(&Text {
        text: "@_user_1 !answer fixture-answer-code\n1: Brief",
    })
    .unwrap();
    assert!(
        !service
            .receive(&group_message(
                "om_denied_answer",
                "event_denied_answer",
                "text",
                &content,
                "om_root",
                "omt_thread",
                false
            ))
            .await
            .unwrap()
    );
    assert!(
        service
            .receive(&group_message(
                "om_answer",
                "event_answer",
                "text",
                &content,
                "om_root",
                "omt_thread",
                true
            ))
            .await
            .unwrap()
    );
    let row = service.store.next().await.unwrap().unwrap();
    let input: Input = serde_json::from_str(&row.payload).unwrap();
    service.process(&input, &row.config_hash).await.unwrap();
    let calls = std::fs::read_to_string(binary.with_extension("calls")).unwrap();
    assert_eq!(calls.lines().count(), 2);
    let mutation: session::provider::Mutation =
        serde_json::from_str(calls.lines().last().unwrap()).unwrap();
    let session::provider::Mutation::Answer { thread, submission } = mutation else {
        panic!("native answer expected")
    };
    assert_eq!(thread, "session");
    assert_eq!(submission.answers[0].question, "format");
    assert_eq!(submission.answers[0].values, ["Brief"]);
    assert!(service.process(&input, &row.config_hash).await.is_err());
    assert_eq!(
        std::fs::read_to_string(binary.with_extension("calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}
