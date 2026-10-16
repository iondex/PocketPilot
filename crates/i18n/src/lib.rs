//! Shared message IDs and explicit locales; never changes a process-global language.
rust_i18n::i18n!("locales", fallback = "zh-CN");

pub fn normalize(locale: &str) -> &'static str {
    if locale.to_ascii_lowercase().starts_with("zh") {
        "zh-CN"
    } else {
        "en"
    }
}

pub fn text(locale: &str, id: &str) -> String {
    rust_i18n::t!(id, locale = normalize(locale)).to_string()
}

/// Uses rust-i18n's single-pass interpolation; user values are never re-expanded.
pub fn format(locale: &str, id: &str, names: &[&str], values: &[String]) -> String {
    rust_i18n::replace_patterns(&text(locale, id), names, values)
}

#[macro_export]
macro_rules! t {
    ($id:expr, locale = $locale:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let values = [$($value.to_string()),*];
        $crate::format($locale, $id, &[$(stringify!($name)),*], &values)
    }};
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: &'static str,
    names: Vec<&'static str>,
    values: Vec<String>,
}

impl Message {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            names: Vec::new(),
            values: Vec::new(),
        }
    }

    pub fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.names.push(name);
        self.values.push(value.to_string());
        self
    }

    pub fn render(&self, locale: &str) -> String {
        format(locale, self.id, &self.names, &self.values)
    }
}

impl From<&'static str> for Message {
    fn from(id: &'static str) -> Self {
        Self::new(id)
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

impl std::error::Error for Message {}

pub trait LocalizedError {
    fn message(&self) -> Message;

    fn localized(&self, locale: &str) -> String {
        self.message().render(locale)
    }
}

impl LocalizedError for Message {
    fn message(&self) -> Message {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
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
                        "tg.", "error.", "discord.", "nav.", "common.", "state.", "cli.",
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
}
