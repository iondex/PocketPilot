use super::*;

#[test]
fn catalogs_share_ids_and_placeholders_and_do_not_use_source_copy_as_ids() {
    let zh: std::collections::BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_str(include_str!("../locales/zh-CN.yml")).unwrap();
    let en: std::collections::BTreeMap<String, serde_yaml::Value> =
        serde_yaml::from_str(include_str!("../locales/en.yml")).unwrap();
    assert_eq!(zh.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
    for (id, value) in &zh {
        assert!(
            id.is_ascii() && !id.contains(' '),
            "non-ID catalog key: {id}"
        );
        if id == "_version" {
            continue;
        }
        assert_eq!(
            text("zh-CN", id),
            value.as_str().unwrap(),
            "compiled catalog: {id}"
        );
        assert_eq!(
            text("en", id),
            en[id].as_str().unwrap(),
            "compiled catalog: {id}"
        );
        let placeholders = |s: &str| {
            s.split("%{")
                .skip(1)
                .filter_map(|s| s.split_once('}').map(|(key, _)| key.to_owned()))
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(
            placeholders(value.as_str().unwrap()),
            placeholders(en[id].as_str().unwrap()),
            "{id}"
        );
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for package in std::fs::read_dir(root).unwrap() {
        let source = package.unwrap().path().join("src");
        if !source.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(source).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            for value in std::fs::read_to_string(&file).unwrap().split('"') {
                if [
                    "tg.", "error.", "discord.", "slack.", "nav.", "common.", "state.", "cli.",
                ]
                .iter()
                .any(|prefix| value.starts_with(prefix))
                    && value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                    && !value.ends_with('.')
                    && value != "state.db"
                // A storage filename, not a message ID.
                {
                    assert!(
                        zh.contains_key(value),
                        "missing message ID {value} in {}",
                        file.display()
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_locales_are_isolated_and_parameters_are_not_templates() {
    assert_eq!(text("en", "nav.hosts"), "Hosts");
    assert_eq!(text("zh-CN", "nav.hosts"), "主机");
    let rendered = t!(
        "tg.new.confirm",
        locale = "en",
        title = "%{cwd}",
        cwd = "/project"
    );
    assert!(rendered.contains("%{cwd}"));
}
