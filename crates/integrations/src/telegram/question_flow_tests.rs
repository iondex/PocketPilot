//! Native requests cross each platform's actual authorization and bound-input path.
use super::*;

async fn answer_text(fixture: &Fixture) -> String {
    let snapshot = fixture.service.config.snapshot();
    let provider = fixture
        .service
        .runtimes
        .get(&snapshot.config, &snapshot.config.workspaces[0])
        .await
        .unwrap();
    provider.read("thread").await.unwrap();
    let dialog = provider.questions("thread").await.unwrap().pop().unwrap();
    format!("!answer {}\n1: Brief", dialog.token)
}

async fn one_answer(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !fixture.state.lock().await.answers.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let state = fixture.state.lock().await;
    assert_eq!(state.answers.len(), 1);
    assert_eq!(state.answers[0].answers["format"].answers, ["Brief"]);
    assert!(
        !state
            .requests
            .iter()
            .any(|r| matches!(*r, "turn/start" | "turn/steer" | "thread/resume"))
    );
}

#[tokio::test]
async fn telegram_bound_question_answer_does_not_steer_and_rejects_another_user() {
    let fixture = Fixture::new().await;
    fixture.state.lock().await.question = true;
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
    let text = answer_text(&fixture).await;
    let mut denied = input(200, 11, &text);
    denied.user = 20;
    let _ = fixture.service.handle(&denied).await;
    assert!(fixture.state.lock().await.answers.is_empty());
    fixture
        .service
        .handle(&input(201, 11, &text))
        .await
        .unwrap();
    one_answer(&fixture).await;
    let _ = fixture.service.handle(&input(202, 11, &text)).await;
    one_answer(&fixture).await;
}

#[tokio::test]
async fn discord_bound_question_answer_does_not_steer_and_rejects_another_user() {
    use discord::{Backend, Command, Input};
    let (fixture, _http, api, service) = discord_fixture().await;
    fixture.state.lock().await.question = true;
    let make = |id, channel, command| Input {
        uploads: vec![],
        locale: None,
        id,
        bot: 1,
        guild: 2,
        channel,
        user: 10,
        date: now(),
        command,
    };
    service
        .handle(
            &api,
            make(
                200,
                7,
                Command::Open {
                    workspace: "host".into(),
                    project: "workspace".into(),
                    thread: "thread".into(),
                },
            ),
        )
        .await
        .unwrap();
    let text = answer_text(&fixture).await;
    let mut denied = make(201, 11, Command::Ask { text: text.clone() });
    denied.user = 20;
    assert!(service.handle(&api, denied).await.is_err());
    assert!(fixture.state.lock().await.answers.is_empty());
    service
        .handle(&api, make(202, 11, Command::Ask { text: text.clone() }))
        .await
        .unwrap();
    one_answer(&fixture).await;
    assert!(
        service
            .handle(&api, make(203, 11, Command::Ask { text }))
            .await
            .is_err()
    );
    one_answer(&fixture).await;
}

#[tokio::test]
async fn slack_bound_question_answer_does_not_steer_and_rejects_another_user() {
    use slack::{
        api::Platform,
        model::{Command, Content, Request},
    };
    struct Offline;
    #[async_trait::async_trait]
    impl Platform for Offline {
        async fn channel(&self, _: &str) -> slack::Result<()> {
            Ok(())
        }

        async fn post(&self, _: &str, _: Option<&str>, _: &Content) -> slack::Result<String> {
            Ok("123.000001".into())
        }

        async fn update(&self, _: &str, _: &str, _: &Content) -> slack::Result<()> {
            Ok(())
        }
    }
    let fixture = Fixture::new().await;
    fixture.state.lock().await.question = true;
    let core = fixture.service.store.core().clone();
    let settings = slack::config::Config {
        app_token: "xapp-fixture".into(),
        proxy: Default::default(),
        bot_token: "xoxb-fixture".into(),
        app_id: "A1".into(),
        team_id: "T1".into(),
        owner_user_id: "U1".into(),
    };
    let context = crate::Context {
        config: fixture.service.config.clone(),
        store: core.clone(),
        runtimes: fixture.service.runtimes.clone(),
    };
    let (_stop, receiver) = watch::channel(false);
    let service = slack::runtime::Service::local_fixture(
        settings,
        context,
        Arc::new(Offline),
        "U2".into(),
        "D1".into(),
        receiver,
    );
    let mut request = Request {
        uploads: vec![],
        menu_message: None,
        id: "open".into(),
        channel: "D1".into(),
        user: "U1".into(),
        root: None,
        binding: None,
        revision: None,
        date: now(),
        command: Command::Open {
            workspace: "workspace".into(),
            thread: "thread".into(),
        },
    };
    service.handle_fixture(&request).await.unwrap();
    let store = slack::storage::Store::new(core, "A1", "T1", "U2");
    let binding = store.at("D1", "123.000001").await.unwrap().unwrap();
    let text = answer_text(&fixture).await;
    request.root = Some(binding.root);
    request.binding = Some(binding.id);
    request.revision = Some(binding.revision);
    request.command = Command::Input { text };
    request.id = "denied".into();
    request.user = "U3".into();
    assert!(service.handle_fixture(&request).await.is_err());
    assert!(fixture.state.lock().await.answers.is_empty());
    request.id = "answer".into();
    request.user = "U1".into();
    service.handle_fixture(&request).await.unwrap();
    one_answer(&fixture).await;
    request.id = "stale-answer".into();
    assert!(service.handle_fixture(&request).await.is_err());
    one_answer(&fixture).await;
}
