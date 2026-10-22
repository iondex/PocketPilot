//! Shared authorization, catalog and durable execution state; independent of chat platforms.

pub mod bootstrap;

pub mod catalog;

pub mod config;

pub mod control;

pub mod model;

pub mod observation;

pub mod operations;

pub mod reload;

pub mod retry;

pub mod runtimes;

pub mod storage;

pub mod uploads;

pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
