//! Terminal failures that an explicitly enabled retry monitor may recover.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    ConcurrencyLimit,
    ServerOverloaded,
    RateLimit,
    ServerError,
    Transport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryableFailure {
    pub turn_id: String,
    pub reason: RetryReason,
}
