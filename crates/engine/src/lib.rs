//! Shared execution runtime, independent of chat-platform adapters.
pub mod catalog;

pub mod config;

pub mod runtimes;

pub mod control;

pub mod uploads;

pub mod model;

pub mod navigation;

pub mod operations;

pub mod observation;

pub mod reload;

pub mod retry;

pub mod storage;

pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
