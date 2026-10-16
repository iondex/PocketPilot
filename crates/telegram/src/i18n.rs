//! Per-user locales; never changes rust-i18n's process-global locale.
use crate::{
    api::Input,
    storage::{Store, StoreError},
};

pub fn normalize(locale: &str) -> &'static str {
    if locale.to_ascii_lowercase().starts_with("zh") {
        "zh-CN"
    } else {
        "en"
    }
}

pub fn lang(input: &Input) -> &str {
    input.locale.as_deref().map(normalize).unwrap_or("zh-CN")
}

pub fn text(locale: &str, key: &str) -> String {
    rust_i18n::t!(key, locale = normalize(locale)).to_string()
}

pub fn tr(input: &Input, key: &str) -> String {
    text(lang(input), key)
}

impl Store {
    pub async fn locale(&self, bot: i64, user: u64) -> Result<Option<String>, StoreError> {
        sqlx::query_scalar("SELECT locale FROM preferences WHERE bot_id=? AND user_id=?")
            .bind(bot)
            .bind(user as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| StoreError::Database)
    }

    pub async fn set_locale(&self, bot: i64, user: u64, locale: &str) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO preferences(bot_id,user_id,locale) VALUES(?,?,?) ON CONFLICT(bot_id,user_id) DO UPDATE SET locale=excluded.locale WHERE preferences.locale<>excluded.locale")
            .bind(bot).bind(user as i64).bind(normalize(locale)).execute(&self.pool).await.map_err(|_|StoreError::Database)?;
        Ok(())
    }
}

impl crate::runtime::Service {
    pub(super) async fn ui_text(&self, chat: i64, key: &str) -> String {
        let locale = self
            .store
            .locale(self.bot, chat as u64)
            .await
            .ok()
            .flatten();
        text(
            locale
                .as_deref()
                .unwrap_or(if self.default_locale == "auto" {
                    "zh-CN"
                } else {
                    &self.default_locale
                }),
            key,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_are_explicit_and_do_not_leak_between_users() {
        assert_eq!(text("en", "主机"), "Hosts");
        assert_eq!(text("zh-CN", "主机"), "主机");
        assert_eq!(text("en", "工作区"), "Workspaces");
        assert_eq!(text("zh-CN", "工作区"), "工作区");
    }
}
