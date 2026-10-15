use crate::config::{ConfigError, Telegram};
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
mod tests {
    use super::*;
    #[test]
    fn accepts_only_a_direct_configured_token() {
        assert!(
            read(&Telegram {
                token: "123:abcdefghijklmnopqrstuvwxyz".into()
            })
            .is_ok()
        );
        assert!(
            read(&Telegram {
                token: "$TELEGRAM_BOT_TOKEN".into()
            })
            .is_err()
        );
        assert!(read(&Telegram { token: "".into() }).is_err());
    }
}
