//! A pure rendering interface: semantic input becomes bounded, persistable native parts.
use serde::{Serialize, de::DeserializeOwned};
use session::markdown::{Format, TooLarge};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFormat {
    TelegramRich,
    FeishuCard,
    SlackBlocks,
    DiscordMarkdown,
    DiscordComponentsV2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Native,
    Text,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub format: MessageFormat,
    pub text_budget: usize,
    pub block_budget: usize,
    pub table_columns: usize,
    pub tables: Support,
    pub mathematics: Support,
    pub footnotes: Support,
    pub collapsible: Support,
}

impl Capabilities {
    pub const fn for_platform(platform: crate::Platform) -> Self {
        match platform {
            crate::Platform::Feishu => Self {
                format: MessageFormat::FeishuCard,
                text_budget: 6000,
                block_budget: 128,
                table_columns: 20,
                tables: Support::Native,
                mathematics: Support::Text,
                footnotes: Support::Text,
                collapsible: Support::Text,
            },
            crate::Platform::Telegram => Self {
                format: MessageFormat::TelegramRich,
                text_budget: 12_000,
                block_budget: 400,
                table_columns: 20,
                tables: Support::Native,
                mathematics: Support::Native,
                footnotes: Support::Native,
                collapsible: Support::Native,
            },
            crate::Platform::Slack => Self {
                format: MessageFormat::SlackBlocks,
                text_budget: 2800,
                block_budget: 50,
                table_columns: 20,
                tables: Support::Native,
                mathematics: Support::Text,
                footnotes: Support::Text,
                collapsible: Support::Native,
            },
            crate::Platform::Discord => Self {
                format: MessageFormat::DiscordComponentsV2,
                text_budget: 3600,
                block_budget: 40,
                table_columns: 0,
                tables: Support::Text,
                mathematics: Support::Text,
                footnotes: Support::Text,
                collapsible: Support::Text,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct RenderRequest<'a> {
    pub source: &'a str,
    pub format: Format,
    pub heading: Option<&'a str>,
}

/// Exact native payloads are frozen before a write; rendering never performs I/O.
pub struct RenderBatch<T> {
    pub format: MessageFormat,
    pub parts: Vec<T>,
}

pub trait Renderer: Send + Sync {
    type Part: Serialize + DeserializeOwned + Send + Sync;

    fn capabilities(&self) -> Capabilities;

    fn render(&self, request: RenderRequest<'_>) -> Result<RenderBatch<Self::Part>, TooLarge>;
}
