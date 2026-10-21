use super::*;

fn dialog() -> Dialog {
    Dialog {
        token: "live-handle".into(),
        turn: Some("turn".into()),
        questions: vec![Question {
            id: "choice".into(),
            header: "Format".into(),
            text: "Choose a format".into(),
            options: vec![OptionLabel {
                label: "Brief".into(),
                description: "A summary".into(),
            }],
            multiple: false,
            other: false,
            secret: false,
        }],
    }
}

fn answer() -> Submission {
    Submission {
        token: "live-handle".into(),
        answers: vec![Answer {
            question: "choice".into(),
            values: vec!["Brief".into()],
        }],
    }
}

#[test]
fn answers_require_all_fields_once_and_explicit_values() {
    let dialog = dialog();
    assert!(dialog.validate(&answer()).is_ok());
    let mut invalid = answer();
    invalid.answers.clear();
    assert!(dialog.validate(&invalid).is_err());
    let mut invalid = answer();
    invalid.answers.push(invalid.answers[0].clone());
    assert!(dialog.validate(&invalid).is_err());
    let mut invalid = answer();
    invalid.answers[0].values = vec!["unknown".into()];
    assert!(dialog.validate(&invalid).is_err());
    let mut invalid = answer();
    invalid.token = "stale".into();
    assert!(dialog.validate(&invalid).is_err());
}

#[test]
fn secret_and_duplicate_native_questions_cannot_be_answered() {
    let mut dialog = dialog();
    dialog.questions[0].secret = true;
    assert!(dialog.validate(&answer()).is_err());
    dialog.questions[0].secret = false;
    dialog.questions.push(dialog.questions[0].clone());
    assert!(!dialog.valid());
}

#[test]
fn multi_select_and_other_require_explicit_runtime_capabilities() {
    let mut dialog = dialog();
    let mut answer = answer();
    answer.answers[0].values.push("Custom".into());
    assert!(dialog.validate(&answer).is_err());
    dialog.questions[0].multiple = true;
    dialog.questions[0].other = true;
    assert!(dialog.validate(&answer).is_ok());
    answer.answers[0].values.push("Custom".into());
    assert!(dialog.validate(&answer).is_err());
}
