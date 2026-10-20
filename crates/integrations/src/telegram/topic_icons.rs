//! Retryable icon discovery. Only a successful catalog can confirm an icon edit.
use super::{Api, Error};
use std::time::Duration;
use teloxide::{prelude::Requester, types::CustomEmojiId};
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Cache {
    entries: Vec<(String, CustomEmojiId)>,
    refreshed_at: Option<Instant>,
    pub(super) retry_at: Option<Instant>,
    rate_limited: bool,
    failures: u32,
}

impl Api {
    pub(super) async fn icon(&self, symbol: &str) -> Result<CustomEmojiId, Error> {
        let mut cache = self.icons.lock().await;
        if let Some(retry) = cache.retry_at.filter(|at| *at > Instant::now()) {
            let seconds = retry
                .saturating_duration_since(Instant::now())
                .as_secs()
                .saturating_add(1);
            return Err(if cache.rate_limited {
                Error::RetryAfter(seconds)
            } else {
                Error::IconUnavailable(seconds)
            });
        }
        if cache
            .refreshed_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3600))
        {
            let started = Instant::now();
            let result = tokio::time::timeout(
                Duration::from_secs(15),
                self.call(self.bot.get_forum_topic_icon_stickers()),
            )
            .await
            .map_err(|_| Error::Timeout)
            .and_then(|r| r);
            let loaded = result.and_then(|stickers| {
                let entries = stickers
                    .into_iter()
                    .filter_map(|s| Some((s.emoji?, s.kind.custom_emoji_id()?.clone())))
                    .collect::<Vec<_>>();
                if entries.is_empty() {
                    Err(Error::Protocol)
                } else {
                    Ok(entries)
                }
            });
            match loaded {
                Ok(entries) => {
                    tracing::info!(
                        entries = entries.len(),
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Telegram topic icon catalog loaded"
                    );
                    cache.entries = entries;
                    cache.refreshed_at = Some(Instant::now());
                    cache.retry_at = None;
                    cache.rate_limited = false;
                    cache.failures = 0;
                }
                Err(error) => {
                    cache.failures = cache.failures.saturating_add(1);
                    let delay = if let Error::RetryAfter(seconds) = &error {
                        (*seconds).max(1)
                    } else {
                        (30_u64.saturating_mul(1 << cache.failures.saturating_sub(1).min(4)))
                            .min(300)
                    };
                    cache.retry_at = Some(Instant::now() + Duration::from_secs(delay));
                    cache.rate_limited = matches!(error, Error::RetryAfter(_));
                    tracing::warn!(
                        error_id = ::i18n::LocalizedError::message(&error).id,
                        attempt = cache.failures,
                        retry_after_seconds = delay,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Telegram topic icon catalog fetch failed"
                    );
                    if matches!(error, Error::RetryAfter(_) | Error::Authentication) {
                        return Err(error);
                    }
                    return Err(Error::IconUnavailable(delay));
                }
            }
        }
        let normalize = |s: &str| s.chars().filter(|c| *c != '\u{fe0f}').collect::<String>();
        if let Some((_, icon)) = cache
            .entries
            .iter()
            .find(|(emoji, _)| normalize(emoji) == normalize(symbol))
        {
            return Ok(icon.clone());
        }
        cache.refreshed_at = None;
        cache.retry_at = Some(Instant::now() + Duration::from_secs(300));
        cache.rate_limited = false;
        tracing::warn!(
            emoji = symbol,
            entries = cache.entries.len(),
            retry_after_seconds = 300,
            "Telegram topic icon has no catalog match"
        );
        Err(Error::IconUnavailable(300))
    }
}
