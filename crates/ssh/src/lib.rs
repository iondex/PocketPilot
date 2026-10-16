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

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        match self {
            Self::Endpoint => ::i18n::Message::new("error.ssh.invalid"),
            Self::Protocol => ::i18n::Message::new("error.ssh.protocol"),
            Self::Disconnected => ::i18n::Message::new("error.ssh.connection"),
            Self::Timeout => ::i18n::Message::new("error.ssh.timeout"),
        }
    }
}
