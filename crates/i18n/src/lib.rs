//! Shared message IDs and explicit locales; never changes a process-global language.
// Make catalog-only edits invalidate normal/release builds, not only unit tests.
const _: &str = include_str!("../locales/en.yml");

const _: &str = include_str!("../locales/zh-CN.yml");

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
mod tests;
