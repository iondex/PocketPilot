use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub proxy: crate::network::Proxy,
    pub token: String,
    /// The only user allowed to use this bot, in their own private chat.
    pub owner_user_id: u64,
    #[serde(default = "default_locale")]
    pub default_locale: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Telegram([REDACTED])")
    }
}

fn default_locale() -> String {
    "zh-CN".into()
}
