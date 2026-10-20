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
                ("start", "tg.cmd.start"),
                ("workspaces", "tg.cmd.workspaces"),
                ("topics", "tg.cmd.topics"),
                ("new", "tg.cmd.new"),
                ("status", "tg.cmd.status"),
                ("history", "tg.cmd.history"),
                ("rename", "tg.cmd.rename"),
                ("refresh", "tg.cmd.refresh"),
                ("language", "tg.cmd.language"),
                ("help", "tg.cmd.help"),
            ]
            .into_iter()
            .map(|(command, key)| {
                BotCommand::new(
                    command,
                    crate::telegram::i18n::text(
                        if locale == "auto" { "zh-CN" } else { locale },
                        key,
                    ),
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
