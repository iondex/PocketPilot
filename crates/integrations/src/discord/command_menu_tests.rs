use super::*;

#[derive(serde::Deserialize)]
struct Command {
    name: String,
    #[serde(default)]
    options: Vec<OptionDefinition>,
}

#[derive(serde::Deserialize)]
struct OptionDefinition {
    name: String,
    #[serde(default)]
    required: bool,
}

#[test]
fn pp_ask_requires_text_and_retains_an_optional_attachment() {
    let commands: Vec<Command> =
        serde_json::from_str(&serde_json::to_string(&commands()).unwrap()).unwrap();
    let ask = commands
        .iter()
        .find(|command| command.name == "pp-ask")
        .unwrap();
    assert!(
        ask.options
            .iter()
            .find(|option| option.name == "text")
            .unwrap()
            .required
    );
    assert!(
        !ask.options
            .iter()
            .find(|option| option.name == "file")
            .unwrap()
            .required
    );
}
