//! Read-only connection to existing Codex app servers.
pub mod protocol;
pub mod provider;
pub mod rpc;
pub mod ssh;
pub mod transport;

pub use provider::Codex;
pub use rpc::{Client, Error, Event, ReadRequest};
pub use transport::{Connection, SshOptions};
