use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Device enrollment limits. Enabling enrollment requires the device registry migration.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct StandaloneConfig {
    pub enabled: bool,
    pub max_devices_per_user: u32,
    pub max_pending_enrollments_per_user: u32,
    pub enrollment_ttl_seconds: u64,
    /// Canonical public API base, including `/api/v1`. Defaults to the hub origin.
    pub api_base_url: Option<String>,
    /// Release authority is configured independently of downloaded release manifests.
    pub release_trust: Option<StandaloneReleaseConfig>,
    /// Encrypted retained telemetry limits per account tier. Missing tiers cannot store history.
    pub telemetry_tiers: std::collections::BTreeMap<String, StandaloneTelemetryLimit>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct StandaloneTelemetryLimit {
    pub max_bytes: u64,
    pub retention_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StandaloneReleaseConfig {
    pub manifest_url: String,
    /// Canonical base64url Ed25519 public keys, 32 bytes each.
    pub public_keys: Vec<String>,
    #[serde(default)]
    pub minimum_sequence: u64,
}

impl Default for StandaloneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_devices_per_user: 100,
            max_pending_enrollments_per_user: 10,
            enrollment_ttl_seconds: 86_400,
            api_base_url: None,
            release_trust: None,
            telemetry_tiers: Default::default(),
        }
    }
}
