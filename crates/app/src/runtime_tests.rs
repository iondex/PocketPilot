use super::*;

#[test]
fn feishu_and_lark_can_run_together_using_the_shared_integration() {
    let text = ["feishu", "lark"].map(|platform| format!(
        "[{platform}]\napp_id='cli_fixture'\napp_secret='fixture'\ntenant_key='tenant'\nowner_open_id='ou_owner'\n[[{platform}.groups]]\nchat_id='oc_group'\nname='Fixture'\n"
    )).join("\n");
    let root = crate::config::parse(std::path::Path::new("/tmp/pocketpilot.toml"), &text)
        .unwrap()
        .config;
    let integrations = configured_integrations(root);
    let names: Vec<_> = integrations.iter().map(|i| i.name()).collect();
    assert_eq!(names, ["feishu", "lark"]);
    assert!(integrations.iter().all(|i| matches!(
        i.capabilities().format,
        integrations::rendering::MessageFormat::FeishuCard
    )));
}
