use super::*;
use session::questions::Answer;

fn request(tool: &str) -> Request {
    serde_json::from_str(&format!(r#"{{"subtype":"can_use_tool","tool_name":"{tool}","input":{{"questions":[{{"question":"Pick languages","header":"Language","options":[{{"label":"Rust","description":"Native"}},{{"label":"Python","description":"Script"}}],"multiSelect":true,"futureQuestionField":"keep"}}],"futureInputField":{{"nested":true}}}},"tool_use_id":"tool-1"}}"#)).unwrap()
}

fn answer(dialog: &Dialog) -> Submission {
    Submission {
        token: dialog.token.clone(),
        answers: vec![Answer {
            question: "Pick languages".into(),
            values: vec!["Rust".into(), "Python".into()],
        }],
    }
}

#[test]
fn only_question_tools_can_construct_native_answers_and_extensions_survive() {
    let mut requests = Requests::default();
    requests
        .receive("approval".into(), request("Bash"), None)
        .unwrap();
    assert!(requests.list().is_empty());
    assert_eq!(requests.waiting(), (true, false));
    requests
        .receive(
            "original-request".into(),
            request("AskUserQuestion"),
            Some("turn".into()),
        )
        .unwrap();
    let dialog = requests.list().pop().unwrap();
    requests
        .receive("original-request".into(), request("AskUserQuestion"), None)
        .unwrap();
    assert_eq!(requests.list()[0].token, dialog.token);
    let submission = answer(&dialog);
    let (response, turn) = requests.answer(submission.clone()).unwrap();
    assert_eq!(turn.as_deref(), Some("turn"));
    #[derive(Deserialize)]
    struct Frame {
        subtype: String,
        request_id: String,
        response: Body,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Body {
        behavior: String,
        #[serde(rename = "updatedInput")]
        input: Box<RawValue>,
    }
    let wire = serde_json::to_string(&response).unwrap();
    let frame: Frame = serde_json::from_str(&wire).unwrap();
    assert_eq!(frame.subtype, "success");
    assert_eq!(frame.request_id, "original-request");
    assert_eq!(frame.response.behavior, "allow");
    #[derive(Deserialize)]
    struct Updated {
        questions: Vec<BTreeMap<String, Box<RawValue>>>,
        answers: BTreeMap<String, String>,
        #[serde(rename = "futureInputField")]
        extension: Box<RawValue>,
    }
    let updated: Updated = serde_json::from_str(frame.response.input.get()).unwrap();
    assert_eq!(updated.answers["Pick languages"], "Rust, Python");
    assert!(updated.questions[0].contains_key("futureQuestionField"));
    assert_eq!(updated.extension.get(), "{\"nested\":true}");
    assert!(matches!(
        requests.answer(submission),
        Err(Failure::Conflict)
    ));
    requests
        .receive("original-request".into(), request("AskUserQuestion"), None)
        .unwrap();
    assert!(requests.list().is_empty());
    assert_eq!(requests.waiting(), (true, false));
}

#[test]
fn cancellation_and_turn_end_invalidate_questions_and_late_snapshots() {
    for finish in [false, true] {
        let mut requests = Requests::default();
        requests
            .receive("request".into(), request("AskUserQuestion"), None)
            .unwrap();
        let submission = answer(&requests.list()[0]);
        if finish {
            requests.finish().unwrap();
        } else {
            requests.cancel("request").unwrap();
        }
        assert!(matches!(
            requests.answer(submission),
            Err(Failure::Conflict)
        ));
        assert!(
            !requests
                .receive("request".into(), request("AskUserQuestion"), None)
                .unwrap()
        );
        assert!(!requests.cancel("request").unwrap());
        assert!(requests.list().is_empty());
    }
}
