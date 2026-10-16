//! Native OpenSSH profiles, challenge authentication and TOTP.
pub mod auth;

pub mod command;

pub mod config;

pub mod migration;

pub mod resolve;
pub use command::{Options, command};
pub use resolve::{ResolvedHost, discover, resolve};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SSH configuration or endpoint is invalid")]
    Endpoint,
    #[error("SSH configuration response is incompatible")]
    Protocol,
    #[error("SSH connection failed")]
    Disconnected,
    #[error("SSH connection timed out")]
    Timeout,
}
