use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub proxy: crate::network::Proxy,
    pub app_token: String,
    pub bot_token: String,
    pub app_id: String,
    pub team_id: String,
    pub owner_user_id: String,
}

impl Config {
    pub fn valid(&self) -> bool {
        !self.app_token.trim().is_empty()
            && !self.bot_token.trim().is_empty()
            && identifier(&self.app_id, 'A')
            && identifier(&self.team_id, 'T')
            && (identifier(&self.owner_user_id, 'U') || identifier(&self.owner_user_id, 'W'))
    }

    pub fn credentials_valid(&self) -> bool {
        self.app_token.starts_with("xapp-") && self.bot_token.starts_with("xoxb-")
    }

    pub fn permits(&self, team: &str, user: &str) -> bool {
        self.team_id == team && self.owner_user_id == user
    }
}

fn identifier(id: &str, prefix: char) -> bool {
    id.len() >= 2
        && id.len() <= 32
        && id.starts_with(prefix)
        && id
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slack")
            .field("app_id", &self.app_id)
            .field("team_id", &self.team_id)
            .field("owner_user_id", &self.owner_user_id)
            .field("tokens", &"[REDACTED]")
            .finish()
    }
}
