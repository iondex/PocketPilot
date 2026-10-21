use super::*;
use session::{
    Capabilities, Page, Session, SessionQuery,
    questions::{OptionLabel, Question},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Runtime {
    reads: AtomicUsize,
    secret: bool,
}

#[async_trait::async_trait]
impl AgentProvider for Runtime {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn list(&self, _: SessionQuery) -> Result<Page<Session>, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    async fn read(&self, _: &str) -> Result<Session, ProviderError> {
        Err(ProviderError::Unsupported)
    }

    async fn history(&self, _: &str) -> Result<History, ProviderError> {
        Ok(History {
            items: vec![],
            all_pages_read: true,
        })
    }

    async fn questions(&self, thread: &str) -> Result<Vec<Dialog>, ProviderError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if thread != "session" {
            return Ok(vec![]);
        }
        Ok(vec![Dialog {
            token: "handle".into(),
            turn: Some("turn".into()),
            questions: vec![Question {
                id: "native-field".into(),
                header: "Choice".into(),
                text: "Private question text".into(),
                options: vec![OptionLabel {
                    label: "A".into(),
                    description: "First".into(),
                }],
                multiple: true,
                other: true,
                secret: self.secret,
            }],
        }])
    }
}

#[tokio::test]
async fn ordinary_input_never_becomes_an_answer_or_even_reads_question_state() {
    let runtime = Runtime {
        reads: AtomicUsize::new(0),
        secret: false,
    };
    for text in [
        "A",
        "continue",
        "!answering is ordinary text",
        "Please answer this",
    ] {
        assert!(
            prepare(&runtime, "session", text, false)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(runtime.reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn explicit_answers_are_correlated_and_validated_before_any_native_write() {
    let runtime = Runtime {
        reads: AtomicUsize::new(0),
        secret: false,
    };
    let Some(Mutation::Answer { thread, submission }) = prepare(
        &runtime,
        "session",
        "!answer handle\n1: [\"A\", \"Custom\"]",
        false,
    )
    .await
    .unwrap() else {
        panic!("answer expected")
    };
    assert_eq!(thread, "session");
    assert_eq!(submission.answers[0].question, "native-field");
    assert_eq!(submission.answers[0].values, ["A", "Custom"]);
    for text in [
        "!answer",
        "!answer handle",
        "!answer handle\n0: A",
        "!answer handle\n2: A",
        "!answer handle\n1: ",
        "!answer handle\n1: [",
        "!answer handle\n1: A\n1: A",
    ] {
        assert!(prepare(&runtime, "session", text, false).await.is_err());
    }
    assert!(matches!(
        prepare(&runtime, "other-session", "!answer handle\n1: A", false).await,
        Err(ProviderError::Conflict)
    ));
    assert!(matches!(
        prepare(&runtime, "session", "!answer stale\n1: A", false).await,
        Err(ProviderError::Conflict)
    ));
    assert!(
        prepare(&runtime, "session", "!answer handle\n1: A", true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn question_projection_is_shared_and_secret_dialogs_have_no_question_or_answer_code() {
    for secret in [true, false] {
        let runtime = Runtime {
            reads: AtomicUsize::new(0),
            secret,
        };
        let history = history(&runtime, "session", "en").await.unwrap();
        let sections = session::presentation::latest(&history, "en");
        let body = &sections.last().unwrap().body;
        assert_eq!(body.contains("Private question text"), !secret);
        assert_eq!(body.contains("!answer handle"), !secret);
        assert_eq!(
            prepare(&runtime, "session", "!answer handle\n1: A", false)
                .await
                .is_err(),
            secret
        );
        // Every platform receives the same source, then applies its existing native renderer.
        use crate::rendering::{RenderRequest, Renderer};
        let request = || RenderRequest {
            source: body,
            format: session::markdown::Format::Markdown,
            heading: Some("Question"),
        };
        assert!(
            crate::slack::render::BlockRenderer
                .render(request())
                .is_ok()
        );
        assert!(
            crate::discord::render::ComponentRenderer
                .render(request())
                .is_ok()
        );
        assert!(
            crate::telegram::rich::RichRenderer
                .render(request())
                .is_ok()
        );
        assert!(
            crate::feishu::render::CardRenderer
                .render(request())
                .is_ok()
        );
    }
}
