use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_executor::jwt::BACKEND_PUB_ENV;
use jsonwebtoken::DecodingKey;
use std::env;

/// The Azure API's list plus the queue worker's storage entries and the
/// client-certificate pair. The executor runs untrusted flows in-process, and
/// the object_store / azure_identity builders those flows can reach read
/// account keys, SAS tokens, client secrets and endpoint overrides straight
/// from the environment. Every per-run permission arrives inside the signed
/// `ExecutionRequest` (presigned URLs, RuntimeCredentials), so a process-wide
/// credential here is never needed and would be exfiltrable by any node.
const FORBIDDEN_AZURE_SETTINGS: &[&str] = &[
    "ACS_EMAIL_ACCESS_KEY",
    "ACS_EMAIL_CONNECTION_STRING",
    "AZURE_COMMUNICATION_CONNECTION_STRING",
    "AZURE_COMMUNICATION_KEY",
    "AZURE_STORAGE_CONNECTION_STRING",
    "AZURE_STORAGE_ACCOUNT_KEY",
    "AZURE_STORAGE_ACCESS_KEY",
    "AZURE_STORAGE_KEY",
    "AZURE_STORAGE_MASTER_KEY",
    "AZURE_STORAGE_CLIENT_SECRET",
    "AZURE_CLIENT_SECRET",
    "AZURE_CLIENT_CERTIFICATE_PATH",
    "AZURE_CLIENT_CERTIFICATE_PASSWORD",
    "AZURE_STORAGE_SAS_KEY",
    "AZURE_STORAGE_SAS_TOKEN",
    "AZURE_STORAGE_TOKEN",
    "AZURE_STORAGE_USE_EMULATOR",
    "AZURE_USE_EMULATOR",
    "AZURE_USE_AZURE_CLI",
    "COMMUNICATION_SERVICES_CONNECTION_STRING",
    "AZURE_SKIP_SIGNATURE",
    "AZURE_STORAGE_SKIP_SIGNATURE",
    "AZURE_STORAGE_ENDPOINT",
    "AZURE_ENDPOINT",
    "AZURITE_BLOB_STORAGE_URL",
];

const SERVER_MODE_ENV: &str = "EXECUTOR_SERVER_MODE";

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub metrics_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        require_server_mode()?;
        reject_forbidden_azure_settings()?;

        let port = parse_port("PORT", 8080)?;
        let metrics_port = parse_port("METRICS_PORT", 9090)?;
        if port == metrics_port {
            return Err(ConfigError::invalid(
                "METRICS_PORT",
                "must differ from PORT",
            ));
        }

        let encoded =
            env::var(BACKEND_PUB_ENV).map_err(|_| ConfigError::MissingVar(BACKEND_PUB_ENV))?;
        check_backend_public_key(&encoded)
            .map_err(|reason| ConfigError::invalid(BACKEND_PUB_ENV, reason))?;

        Ok(Self { port, metrics_port })
    }
}

/// The Kubernetes executor falls back to a job-once mode when this is not
/// true; this image has no such mode, so anything but an explicit opt-in is
/// refused rather than silently served. The Dockerfile bakes `true` in, so a
/// different value only ever reaches here when an operator overrode it.
fn require_server_mode() -> Result<(), ConfigError> {
    if is_server_mode(&env::var(SERVER_MODE_ENV).unwrap_or_default()) {
        return Ok(());
    }

    Err(ConfigError::invalid(
        SERVER_MODE_ENV,
        "must be 'true': the Azure executor image is server-only and has no job-once mode",
    ))
}

fn is_server_mode(value: &str) -> bool {
    let value = value.trim();
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn reject_forbidden_azure_settings() -> Result<(), ConfigError> {
    for variable in FORBIDDEN_AZURE_SETTINGS {
        if env::var(variable)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(ConfigError::ForbiddenAzureSetting(variable));
        }
    }

    Ok(())
}

/// The executor decodes `BACKEND_PUB` lazily on the first request and caches
/// the result for the life of the process, and the Terraform variable behind
/// it defaults to an empty string. An empty or malformed value would therefore
/// pass startup and fail every run with a JWT error, so it is decoded here
/// exactly the way `flow_like_executor::jwt` decodes it: standard base64 of
/// the untrimmed value, then a P-256 public key PEM.
fn check_backend_public_key(encoded: &str) -> Result<(), &'static str> {
    if encoded.trim().is_empty() {
        return Err("must not be empty");
    }

    let pem = STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| "must be standard base64 without surrounding whitespace")?;
    DecodingKey::from_ec_pem(&pem).map_err(|_| "must decode to a P-256 (ES256) public key PEM")?;

    Ok(())
}

fn parse_port(variable: &'static str, default: u16) -> Result<u16, ConfigError> {
    let raw = env::var(variable).unwrap_or_else(|_| default.to_string());
    let port = raw
        .trim()
        .parse::<u16>()
        .map_err(|_| ConfigError::invalid(variable, "must be a valid TCP port"))?;

    if port == 0 {
        return Err(ConfigError::invalid(variable, "must be greater than zero"));
    }

    Ok(port)
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar(&'static str),
    InvalidValue {
        variable: &'static str,
        reason: &'static str,
    },
    ForbiddenAzureSetting(&'static str),
}

impl ConfigError {
    fn invalid(variable: &'static str, reason: &'static str) -> Self {
        Self::InvalidValue { variable, reason }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingVar(variable) => {
                write!(
                    formatter,
                    "missing required environment variable {variable}"
                )
            }
            Self::InvalidValue { variable, reason } => {
                write!(formatter, "invalid {variable}: {reason}")
            }
            Self::ForbiddenAzureSetting(variable) => write!(
                formatter,
                "{variable} is forbidden: the Azure executor runs untrusted flows and must hold no Azure credential or endpoint override"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway P-256 public key generated for this test; no private half exists.
    const VALID_BACKEND_PUB: &str = "LS0tLS1CRUdJTiBQVUJMSUMgS0VZLS0tLS0KTUZrd0V3WUhLb1pJemowQ0FRWUlLb1pJemowREFRY0RRZ0FFWjdKL2ZkMUM5d29jKzgrajQzNUFLdktxODUwVAowb05qTGpDeGhsUFpmdEpQQWwxN3JaTDR2TFRnK0p0Q0VseFN5ME1QUEozYXR4aWRzUnhEZ29sUy9RPT0KLS0tLS1FTkQgUFVCTElDIEtFWS0tLS0tCg==";

    #[test]
    fn server_mode_accepts_only_explicit_opt_in() {
        assert!(is_server_mode("true"));
        assert!(is_server_mode("TRUE"));
        assert!(is_server_mode(" 1 "));
        assert!(!is_server_mode(""));
        assert!(!is_server_mode("false"));
        assert!(!is_server_mode("0"));
        assert!(!is_server_mode("yes"));
    }

    #[test]
    fn backend_public_key_must_decode_like_the_executor_does() {
        assert!(check_backend_public_key(VALID_BACKEND_PUB).is_ok());
        assert!(check_backend_public_key("").is_err());
        assert!(check_backend_public_key("   ").is_err());
        assert!(check_backend_public_key(&format!("{VALID_BACKEND_PUB}\n")).is_err());
        assert!(check_backend_public_key("bm90IGEgcGVt").is_err());
        assert!(check_backend_public_key("not base64!").is_err());
    }
}
