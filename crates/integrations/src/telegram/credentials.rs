use crate::telegram::config::Config as Telegram;
use engine::config::ConfigError;
use secrecy::SecretString;

pub fn read(config: &Telegram) -> Result<SecretString, ConfigError> {
    let token = &config.token;
    let Some((id, secret)) = token.split_once(':') else {
        return Err(ConfigError::Credentials);
    };
    if id.is_empty()
        || !id.bytes().all(|c| c.is_ascii_digit())
        || secret.len() < 20
        || !secret
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err(ConfigError::Credentials);
    }
    Ok(SecretString::from(token.clone()))
}

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod tests;
