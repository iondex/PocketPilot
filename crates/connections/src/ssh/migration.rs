//! Offline Google Authenticator export decoding. Never logs seeds or codes.
use crate::ssh::config::TOTP;
use base64::{Engine, engine::general_purpose::STANDARD};
use prost::Message;

#[derive(Debug)]
pub struct Account {
    pub name: String,
    pub issuer: String,
    pub totp: TOTP,
}

#[derive(Debug, thiserror::Error)]
#[error("unsupported or invalid TOTP export (values redacted)")]
pub struct Error;

impl ::i18n::LocalizedError for Error {
    fn message(&self) -> ::i18n::Message {
        ::i18n::Message::new("error.ssh.totp_export")
    }
}

// The export wire format is protobuf, distinct from an ordinary otpauth URI.
#[derive(Message)]
struct Payload {
    #[prost(message, repeated, tag = "1")]
    accounts: Vec<Parameters>,
    #[prost(int32, tag = "2")]
    version: i32,
    #[prost(int32, tag = "3")]
    batch_size: i32,
    #[prost(int32, tag = "4")]
    batch_index: i32,
}

#[derive(Message)]
struct Parameters {
    #[prost(bytes = "vec", tag = "1")]
    secret: Vec<u8>,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, tag = "3")]
    issuer: String,
    #[prost(int32, tag = "4")]
    algorithm: i32,
    #[prost(int32, tag = "5")]
    digits: i32,
    #[prost(int32, tag = "6")]
    kind: i32,
}

/// Decode a complete, single-batch export. The caller must choose prompt patterns
/// and match the account to a configured host before installing its secret.
pub fn decode(uri: &str) -> Result<Vec<Account>, Error> {
    if uri.len() > 64 * 1024 {
        return Err(Error);
    }
    let url = url::Url::parse(uri).map_err(|_| Error)?;
    if url.scheme() != "otpauth-migration"
        || url.host_str() != Some("offline")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
    {
        return Err(Error);
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.len() != 1 || pairs[0].0 != "data" {
        return Err(Error);
    }
    let bytes = STANDARD.decode(pairs[0].1.as_bytes()).map_err(|_| Error)?;
    let payload = Payload::decode(bytes.as_slice()).map_err(|_| Error)?;
    if !matches!(payload.version, 1 | 2)
        || payload.batch_size != 1
        || payload.batch_index != 0
        || payload.accounts.is_empty()
        || payload.accounts.len() > 64
    {
        return Err(Error);
    }
    payload.accounts.into_iter().map(account).collect()
}

fn account(value: Parameters) -> Result<Account, Error> {
    if value.kind != 2 || value.secret.is_empty() || value.secret.len() > 1024 {
        return Err(Error);
    }
    let algorithm = match value.algorithm {
        1 => "SHA1",
        2 => "SHA256",
        3 => "SHA512",
        _ => return Err(Error),
    };
    let digits = match value.digits {
        1 => 6,
        2 => 8,
        _ => return Err(Error),
    };
    Ok(Account {
        name: value.name,
        issuer: value.issuer,
        totp: TOTP {
            secret_base32: totp_rs::Secret::from(value.secret).to_base32(),
            algorithm: algorithm.into(),
            digits,
            period_seconds: 30,
            minimum_remaining_seconds: 5,
            prompt_patterns: vec![],
            password_prompt_patterns: vec![],
        },
    })
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
