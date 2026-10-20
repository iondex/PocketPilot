use super::*;

#[test]
fn locales_are_explicit_and_do_not_leak_between_users() {
    assert_eq!(text("en", "nav.hosts"), "Hosts");
    assert_eq!(text("zh-CN", "nav.hosts"), "主机");
    assert_eq!(text("en", "nav.workspaces"), "Workspaces");
    assert_eq!(text("zh-CN", "nav.workspaces"), "工作区");
}
