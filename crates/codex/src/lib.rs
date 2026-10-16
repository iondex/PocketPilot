//! Typed observation and four explicit mutation methods on existing Codex app servers.
pub mod discovery;

#[cfg(test)]
mod live_tests;

pub mod protocol;

pub mod provider;

pub mod rpc;

pub mod transport;

pub use provider::Codex;
pub use rpc::{Client, Error, Event, ReadRequest};
pub use transport::{Connection, SSHOptions};
