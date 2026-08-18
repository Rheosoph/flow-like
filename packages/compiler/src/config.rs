use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One explicitly allowlisted storage origin: scheme, host and port together.
/// Only an origin the operator listed here may deviate from the HTTPS-on-443
/// default that every other signed storage URL is held to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedStorageOrigin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl AllowedStorageOrigin {
    pub fn matches(&self, scheme: &str, host: &str, port: u16) -> bool {
        self.scheme == scheme && self.host == host && self.port == port
    }
}

impl std::fmt::Display for AllowedStorageOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}:{}", self.scheme, self.host, self.port)
    }
}

/// Parses a comma-separated `COMPILER_ALLOWED_STORAGE_HOSTS` value. Each entry
/// is either a bare host (`r2.example.com`), which keeps meaning HTTPS on port
/// 443, or a full origin that pins scheme and port (`https://r2.example.com`,
/// `http://minio:9000`). Paths, queries and user information are not accepted.
pub fn parse_allowed_storage_origins(value: &str) -> Vec<AllowedStorageOrigin> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let origin = parse_allowed_storage_origin(entry);
            if origin.is_none() {
                tracing::warn!(
                    entry,
                    "Ignoring COMPILER_ALLOWED_STORAGE_HOSTS entry: expected a bare host or a http(s) origin"
                );
            }
            origin
        })
        .collect()
}

fn parse_allowed_storage_origin(entry: &str) -> Option<AllowedStorageOrigin> {
    let entry = entry.to_ascii_lowercase();
    if !entry.contains("://") {
        let valid = !entry.is_empty()
            && !entry
                .contains(|c: char| c.is_whitespace() || matches!(c, ':' | '/' | '@' | '?' | '#'));
        return valid.then(|| AllowedStorageOrigin {
            scheme: "https".to_string(),
            host: entry,
            port: 443,
        });
    }

    let url = Url::parse(&entry).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(AllowedStorageOrigin {
        scheme: url.scheme().to_string(),
        host: url.host_str()?.to_ascii_lowercase(),
        port: url.port_or_known_default()?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerConfig {
    #[serde(default = "default_callback_timeout_ms")]
    pub callback_timeout_ms: u64,
    #[serde(default = "default_callback_retries")]
    pub callback_retries: u32,
    #[serde(default = "default_compilation_timeout_secs")]
    pub compilation_timeout_secs: u64,
    #[serde(default)]
    pub max_parallel_targets: Option<usize>,
    /// End-to-end timeout for each signed object-store request.
    #[serde(default = "default_storage_timeout_secs")]
    pub storage_timeout_secs: u64,
    /// Maximum raw WASM download accepted by an external compiler.
    #[serde(default = "default_max_wasm_bytes")]
    pub max_wasm_bytes: u64,
    /// Maximum single compiled artifact accepted for upload.
    #[serde(default = "default_max_artifact_bytes")]
    pub max_artifact_bytes: u64,
    /// Exact Azure account/container scope expected by the Azure worker.
    #[serde(default)]
    pub azure_storage_account: Option<String>,
    #[serde(default)]
    pub azure_content_container: Option<String>,
    #[serde(default)]
    pub azure_meta_container: Option<String>,
    /// Optional exact origin allowlist for non-Azure cloud endpoints (for
    /// example Cloudflare R2, MinIO or another S3-compatible endpoint). See
    /// [`parse_allowed_storage_origins`] for the accepted entry forms.
    #[serde(default)]
    pub allowed_storage_hosts: Vec<AllowedStorageOrigin>,
}

const HARD_MAX_WASM_BYTES: u64 = 512 * 1024 * 1024;
const HARD_MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

fn default_callback_timeout_ms() -> u64 {
    10_000
}
fn default_callback_retries() -> u32 {
    3
}
fn default_compilation_timeout_secs() -> u64 {
    600
}
fn default_storage_timeout_secs() -> u64 {
    120
}
fn default_max_wasm_bytes() -> u64 {
    256 * 1024 * 1024
}
fn default_max_artifact_bytes() -> u64 {
    512 * 1024 * 1024
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            callback_timeout_ms: default_callback_timeout_ms(),
            callback_retries: default_callback_retries(),
            compilation_timeout_secs: default_compilation_timeout_secs(),
            max_parallel_targets: None,
            storage_timeout_secs: default_storage_timeout_secs(),
            max_wasm_bytes: default_max_wasm_bytes(),
            max_artifact_bytes: default_max_artifact_bytes(),
            azure_storage_account: None,
            azure_content_container: None,
            azure_meta_container: None,
            allowed_storage_hosts: Vec::new(),
        }
    }
}

impl CompilerConfig {
    pub fn from_env() -> Self {
        Self {
            callback_timeout_ms: std::env::var("COMPILER_CALLBACK_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_timeout_ms),
            callback_retries: std::env::var("COMPILER_CALLBACK_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_callback_retries),
            compilation_timeout_secs: std::env::var("COMPILER_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_compilation_timeout_secs),
            max_parallel_targets: std::env::var("COMPILER_MAX_PARALLEL_TARGETS")
                .ok()
                .and_then(|v| v.parse().ok()),
            storage_timeout_secs: bounded_env_u64(
                "COMPILER_STORAGE_TIMEOUT_SECS",
                default_storage_timeout_secs(),
                5,
                300,
            ),
            max_wasm_bytes: bounded_env_u64(
                "COMPILER_MAX_WASM_BYTES",
                default_max_wasm_bytes(),
                1024,
                HARD_MAX_WASM_BYTES,
            ),
            max_artifact_bytes: bounded_env_u64(
                "COMPILER_MAX_ARTIFACT_BYTES",
                default_max_artifact_bytes(),
                1024,
                HARD_MAX_ARTIFACT_BYTES,
            ),
            azure_storage_account: nonempty_env("AZURE_STORAGE_ACCOUNT_NAME"),
            azure_content_container: nonempty_env("AZURE_CONTENT_CONTAINER"),
            azure_meta_container: nonempty_env("AZURE_META_CONTAINER"),
            allowed_storage_hosts: std::env::var("COMPILER_ALLOWED_STORAGE_HOSTS")
                .ok()
                .map(|value| parse_allowed_storage_origins(&value))
                .unwrap_or_default(),
        }
    }

    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.compilation_timeout_secs = secs;
        self
    }

    pub fn callback_timeout(&self) -> Duration {
        Duration::from_millis(self.callback_timeout_ms)
    }

    pub fn compilation_timeout(&self) -> Duration {
        Duration::from_secs(self.compilation_timeout_secs)
    }

    pub fn storage_timeout(&self) -> Duration {
        Duration::from_secs(self.storage_timeout_secs)
    }

    pub fn allows_storage_origin(&self, scheme: &str, host: &str, port: u16) -> bool {
        self.allowed_storage_hosts
            .iter()
            .any(|allowed| allowed.matches(scheme, host, port))
    }

    /// True only when the operator allowlisted at least one cleartext origin.
    pub fn allows_plaintext_storage(&self) -> bool {
        self.allowed_storage_hosts
            .iter()
            .any(|allowed| allowed.scheme == "http")
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bounded_env_u64(name: &str, default: u64, minimum: u64, maximum: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (minimum..=maximum).contains(value))
        .unwrap_or(default)
}
