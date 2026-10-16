use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Authentication {
    pub methods: Vec<String>,
    pub private_key_passphrase: Option<String>,
    pub password: Option<String>,
    pub max_attempts: u32,
    pub totp: Option<TOTP>,
}

impl Default for Authentication {
    fn default() -> Self {
        Self {
            methods: vec![],
            private_key_passphrase: None,
            password: None,
            max_attempts: 2,
            totp: None,
        }
    }
}

impl std::fmt::Debug for Authentication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authentication([REDACTED])")
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TOTP {
    pub secret_base32: String,
    pub algorithm: String,
    pub digits: usize,
    pub period_seconds: u64,
    pub minimum_remaining_seconds: u64,
    pub prompt_patterns: Vec<String>,
    #[serde(default)]
    pub password_prompt_patterns: Vec<String>,
}

impl std::fmt::Debug for TOTP {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TOTP([REDACTED])")
    }
}
