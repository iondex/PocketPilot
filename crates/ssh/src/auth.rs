//! OpenSSH askpass responder. Secrets are read from the single root TOML only.
use crate::config::{Authentication, TOTP};
use regex::Regex;
use secrecy::SecretString;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};
use totp_rs::{Algorithm, Builder, Secret};

/// The host process supplies root-file identity; authentication never loads app configuration.
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub data_dir: PathBuf,
    pub source: PathBuf,
    pub source_hash: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, thiserror::Error)]
#[error(
    "SSH authentication challenge is unsupported, invalid or already answered (values redacted)"
)]
pub struct Error;

fn matches(patterns: &[String], prompt: &str) -> Result<bool, Error> {
    for pattern in patterns {
        if Regex::new(pattern).map_err(|_| Error)?.is_match(prompt) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn validate(auth: &Authentication) -> Result<(), Error> {
    if auth.max_attempts == 0
        || auth.max_attempts > 10
        || auth.methods.iter().any(|m| {
            !matches!(
                m.as_str(),
                "publickey" | "password" | "keyboard-interactive" | "hostbased" | "gssapi-with-mic"
            )
        })
    {
        return Err(Error);
    }
    if let Some(totp) = &auth.totp {
        generator(totp)?;
        if totp.minimum_remaining_seconds > 60
            || totp.period_seconds > 300
            || totp.prompt_patterns.is_empty()
        {
            return Err(Error);
        }
        for pattern in totp
            .prompt_patterns
            .iter()
            .chain(&totp.password_prompt_patterns)
        {
            Regex::new(pattern).map_err(|_| Error)?;
        }
    }
    Ok(())
}

fn generator(config: &TOTP) -> Result<totp_rs::Totp, Error> {
    if !matches!(config.digits, 6 | 8) || config.period_seconds == 0 {
        return Err(Error);
    }
    let algorithm = match config.algorithm.as_str() {
        "SHA1" => Algorithm::SHA1,
        "SHA256" => Algorithm::SHA256,
        "SHA512" => Algorithm::SHA512,
        _ => return Err(Error),
    };
    let secret = Secret::try_from_base32(&config.secret_base32).map_err(|_| Error)?;
    if secret.as_ref().is_empty() {
        return Err(Error);
    }
    // Existing authenticators commonly use 80-bit seeds. Preserve those seeds;
    // all algorithm/digit/period inputs remain validated before generation.
    Ok(Builder::new()
        .with_algorithm(algorithm)
        .with_digits(u8::try_from(config.digits).map_err(|_| Error)?)
        .with_step_duration(config.period_seconds)
        .with_secret(secret)
        .build_noncompliant())
}

pub fn helper(
    auth: &Authentication,
    host_id: &str,
    config: &AuthContext,
) -> Result<Option<PathBuf>, Error> {
    if auth.totp.is_none() && auth.password.is_none() && auth.private_key_passphrase.is_none() {
        return Ok(None);
    }
    validate(auth)?;

    #[cfg(not(unix))]
    {
        return Err(Error);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let exists = config.data_dir.exists();
        std::fs::create_dir_all(&config.data_dir).map_err(|_| Error)?;
        if !exists {
            std::fs::set_permissions(&config.data_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| Error)?;
        }
        let directory = config.data_dir.join("ssh-auth");
        std::fs::create_dir_all(&directory).map_err(|_| Error)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Error)?;
        let attempt = uuid::Uuid::new_v4().to_string();
        let script = directory.join(format!("{attempt}.sh"));
        let binary = std::env::current_exe().map_err(|_| Error)?;
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
        let contents = format!(
            "#!/bin/sh\nexec {} --config {} ssh-askpass --host {} --attempt {} --config-hash {} -- \"$@\"\n",
            quote(binary.to_str().ok_or(Error)?),
            quote(config.source.to_str().ok_or(Error)?),
            quote(host_id),
            quote(&attempt),
            quote(&config.source_hash)
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&script)
            .map_err(|_| Error)?;
        file.write_all(contents.as_bytes()).map_err(|_| Error)?;
        file.sync_all().map_err(|_| Error)?;
        Ok(Some(script))
    }
}

pub async fn answer(
    auth: &Authentication,
    config: &AuthContext,
    attempt: &str,
    expected_hash: &str,
    prompt: &str,
) -> Result<SecretString, Error> {
    if config.source_hash != expected_hash {
        return Err(Error);
    }
    if prompt.len() > 4096 || std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm") {
        return Err(Error);
    }
    let attempt = uuid::Uuid::parse_str(attempt).map_err(|_| Error)?;
    validate(auth)?;
    if let Some(totp) = &auth.totp
        && matches(&totp.prompt_patterns, prompt)?
    {
        let mut time = u64::try_from(now()).map_err(|_| Error)?;
        let remaining = totp.period_seconds - time % totp.period_seconds;
        if remaining < totp.minimum_remaining_seconds {
            tokio::time::sleep(std::time::Duration::from_secs(remaining)).await;
            time = u64::try_from(now()).map_err(|_| Error)?;
        }
        record_window(
            config,
            &attempt,
            time / totp.period_seconds,
            auth.max_attempts,
        )?;
        return Ok(generator(totp)?.generate(time).to_string().into());
    }
    if let Some(password) = &auth.password {
        let allowed = if let Some(totp) = &auth.totp {
            matches(&totp.password_prompt_patterns, prompt)?
        } else {
            Regex::new(r"(?i)^(?:[a-z0-9._-]+@[a-z0-9._:-]+['’]s )?password:\s*$")
                .map_err(|_| Error)?
                .is_match(prompt)
        };
        if allowed {
            return Ok(password.clone().into());
        }
    }
    if let Some(passphrase) = &auth.private_key_passphrase
        && Regex::new(r"(?i)^enter passphrase for key '.+':\s*$")
            .map_err(|_| Error)?
            .is_match(prompt)
    {
        return Ok(passphrase.clone().into());
    }
    Err(Error)
}

fn record_window(
    config: &AuthContext,
    attempt: &uuid::Uuid,
    window: u64,
    max: u32,
) -> Result<(), Error> {
    let path = config
        .data_dir
        .join("ssh-auth")
        .join(format!("{attempt}.windows"));
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| Error)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| Error)?;
    let mut data = String::new();
    Read::by_ref(&mut file)
        .take(4096)
        .read_to_string(&mut data)
        .map_err(|_| Error)?;
    let windows = data
        .lines()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error)?;
    if windows.len() >= max as usize || windows.contains(&window) {
        return Err(Error);
    }
    file.seek(SeekFrom::End(0)).map_err(|_| Error)?;
    writeln!(file, "{window}").map_err(|_| Error)?;
    file.sync_all().map_err(|_| Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vector_is_generated_without_reimplementing_totp() {
        let config = TOTP {
            secret_base32: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into(),
            algorithm: "SHA1".into(),
            digits: 8,
            period_seconds: 30,
            minimum_remaining_seconds: 5,
            prompt_patterns: vec!["(?i)^otp: *$".into()],
            password_prompt_patterns: vec![],
        };
        assert_eq!(
            generator(&config).unwrap().generate(59).to_string(),
            "94287082"
        );
        assert!(matches(&config.prompt_patterns, "OTP: ").unwrap());
        assert!(!matches(&config.prompt_patterns, "Approve your login (yes/no)?").unwrap());
    }

    #[test]
    fn duplicate_challenge_in_the_same_window_is_not_answered_again() {
        let dir = tempfile::tempdir().unwrap();
        let config = AuthContext {
            data_dir: dir.path().join("data"),
            source: dir.path().join("pocketpilot.toml"),
            source_hash: "fixture".into(),
        };
        std::fs::create_dir_all(config.data_dir.join("ssh-auth")).unwrap();
        let attempt = uuid::Uuid::new_v4();
        record_window(&config, &attempt, 1, 2).unwrap();
        assert!(record_window(&config, &attempt, 1, 2).is_err());
        record_window(&config, &attempt, 2, 2).unwrap();
        assert!(record_window(&config, &attempt, 3, 2).is_err());
    }
}
