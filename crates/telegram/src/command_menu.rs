//! Register localized, private-chat command suggestions. Authorization is unchanged.
use super::*;
use teloxide::{
    payloads::SetMyCommandsSetters,
    types::{BotCommand, BotCommandScope},
};

impl Api {
    pub async fn register_commands(&self, default_locale: &str) -> Result<(), Error> {
        for (language, locale) in [("", default_locale), ("zh", "zh-CN"), ("en", "en")] {
            let commands = [
                ("start", "cmd_start"),
                ("workspaces", "cmd_workspaces"),
                ("topics", "cmd_topics"),
                ("cleartopics", "cmd_cleartopics"),
                ("new", "cmd_new"),
                ("status", "cmd_status"),
                ("history", "cmd_history"),
                ("rename", "cmd_rename"),
                ("unbind", "cmd_unbind"),
                ("refresh", "cmd_refresh"),
                ("language", "cmd_language"),
                ("help", "cmd_help"),
            ]
            .into_iter()
            .map(|(command, key)| {
                BotCommand::new(
                    command,
                    crate::i18n::text(if locale == "auto" { "zh-CN" } else { locale }, key),
                )
            });
            tokio::time::timeout(
                Duration::from_secs(5),
                self.call(
                    self.bot
                        .set_my_commands(commands)
                        .scope(BotCommandScope::AllPrivateChats)
                        .language_code(language),
                ),
            )
            .await
            .map_err(|_| Error::Timeout)??;
        }
        Ok(())
    }
}
