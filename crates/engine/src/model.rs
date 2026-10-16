//! Persisted agent instances and project entries, independent of root TOML.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub host_id: String,
    pub kind: String,
    pub connection_mode: String,
    pub compatibility_profile: String,
    /// Operator attestation for this exact shared runtime after the lifecycle probe.
    #[serde(default)]
    pub control_verified: bool,
    #[serde(default)]
    pub endpoint: Option<codex::discovery::Endpoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub short_name: String,
    pub cwd: String,
    #[serde(default)]
    pub additional_cwds: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}
