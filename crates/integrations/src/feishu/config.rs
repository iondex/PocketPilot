//! Domestic Feishu application scope; credentials never enter Debug output.
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub app_id: String,
    pub app_secret: String,
    pub tenant_key: String,
    pub owner_open_id: String,
    pub groups: Vec<Group>,
    #[serde(default)]
    pub proxy: crate::network::Proxy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub chat_id: String,
    pub name: String,
}

impl Config {
    pub fn valid(&self) -> bool {
        self.app_id.starts_with("cli_")
            && identifier(&self.app_id)
            && !self.app_secret.trim().is_empty()
            && identifier(&self.tenant_key)
            && self.owner_open_id.starts_with("ou_")
            && identifier(&self.owner_open_id)
            && !self.groups.is_empty()
            && self.groups.len() <= 32
            && self.groups.iter().all(|g| {
                g.chat_id.starts_with("oc_")
                    && identifier(&g.chat_id)
                    && !g.name.trim().is_empty()
                    && g.name.len() <= 200
            })
            && self
                .groups
                .iter()
                .enumerate()
                .all(|(i, g)| self.groups[..i].iter().all(|p| p.chat_id != g.chat_id))
    }

    pub fn permits(&self, tenant: &str, owner: &str) -> bool {
        tenant == self.tenant_key && owner == self.owner_open_id
    }

    pub fn group(&self, chat: &str) -> bool {
        self.groups.iter().any(|g| g.chat_id == chat)
    }

    pub fn namespace(&self) -> String {
        format!(
            "feishu:{}:{}:{}",
            self.app_id, self.tenant_key, self.owner_open_id
        )
    }
}

pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feishu")
            .field("app_id", &self.app_id)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}
