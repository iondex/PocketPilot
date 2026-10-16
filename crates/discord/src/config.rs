use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub token: String,
    pub guild_id: u64,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discord")
            .field("guild_id", &self.guild_id)
            .field("token", &"[REDACTED]")
            .finish()
    }
}
