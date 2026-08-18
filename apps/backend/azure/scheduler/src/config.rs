//! Startup guard for the Azure scheduler image.
//!
//! The tick itself lives in `flow-like-scheduler-tick`; this module only
//! decides whether the process may start. It refuses every credential-shaped or
//! endpoint-override variable the Azure API refuses (the process reaches Cosmos
//! through the Container Apps managed-identity endpoint and nothing else), pins
//! the state backend to Cosmos, and requires the user-assigned identity that
//! Terraform binds to the job. Everything the tick needs beyond that is read by
//! [`TickConfig::from_env`] and `CosmosTickStore::from_env`.

use flow_like_scheduler_tick::{TickConfig, TickError, require_state_backend};
use std::{env, fmt};

/// Mirrors `FORBIDDEN_AZURE_SETTINGS` in `apps/backend/azure/api/src/config.rs`;
/// keep the two lists identical. Account keys, SAS tokens, client secrets,
/// emulator switches and storage endpoint overrides have no legitimate reading
/// in this image: nothing here talks to Blob Storage or ACS, so any of them
/// present means the job was handed an environment meant for something else.
const FORBIDDEN_AZURE_SETTINGS: &[&str] = &[
    "ACS_EMAIL_ACCESS_KEY",
    "ACS_EMAIL_CONNECTION_STRING",
    "AZURE_COMMUNICATION_CONNECTION_STRING",
    "AZURE_COMMUNICATION_KEY",
    "AZURE_STORAGE_ACCOUNT_KEY",
    "AZURE_STORAGE_ACCESS_KEY",
    "AZURE_STORAGE_MASTER_KEY",
    "AZURE_STORAGE_CLIENT_SECRET",
    "AZURE_CLIENT_SECRET",
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

/// The token-source selectors and proxies the API's identity guard
/// (`flow_like_azure_data::postgres`) refuses. `azure_identity` chooses where to
/// send the managed-identity request from exactly these names, so any of them
/// present would redirect that request — and the `IDENTITY_HEADER` it carries —
/// away from the Container Apps endpoint; a proxy would see it in transit.
const FORBIDDEN_IDENTITY_SETTINGS: &[&str] = &[
    "MSI_ENDPOINT",
    "MSI_SECRET",
    "IMDS_ENDPOINT",
    "IDENTITY_SERVER_THUMBPRINT",
    "AZURE_AUTHORITY_HOST",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// The only backend this binary links (`flow-like-scheduler-tick/azure`).
pub const STATE_BACKEND: &str = "cosmos";
/// `CosmosClient` also accepts `auto`, `workload_identity` and
/// `developer_tools`; this image pins the one mode Terraform configures so a
/// developer-credential fallback can never write production cron state.
const COSMOS_AUTH_MODE: &str = "managed_identity";
/// Same floor as the API's `SINK_SECRET` and the maintenance token. A compact
/// HS256 JWT carrying the cron claims is several times longer, so anything
/// shorter is a placeholder or a truncated paste, not a token the API will honour.
pub const MIN_JWT_BYTES: usize = 32;

#[derive(Debug)]
pub struct Config {
    pub tick: TickConfig,
    /// Client ID of the job's user-assigned identity, as `CosmosClient` reads it
    /// from `AZURE_CLIENT_ID`. Validated here so a missing or malformed value
    /// fails at startup instead of as one token error per schedule.
    pub client_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        reject_forbidden_settings()?;
        validate_identity_endpoint()?;
        require_state_backend(STATE_BACKEND)?;
        require_managed_identity_auth_mode()?;
        let client_id = required_client_id()?;

        let tick = TickConfig::from_env()?;
        if tick.jwt.expose().len() < MIN_JWT_BYTES {
            return Err(ConfigError::invalid(
                "SINK_TRIGGER_JWT",
                format!("must contain at least {MIN_JWT_BYTES} bytes"),
            ));
        }

        Ok(Self { tick, client_id })
    }
}

/// Presence, not value, is the signal: the variable reaching the deployment at
/// all means something intended it to take effect, and whether an empty string
/// parses into something harmless is a property of a third-party parser.
fn reject_forbidden_settings() -> Result<(), ConfigError> {
    for variable in FORBIDDEN_AZURE_SETTINGS
        .iter()
        .chain(FORBIDDEN_IDENTITY_SETTINGS)
    {
        if env::var_os(variable).is_some() {
            return Err(ConfigError::ForbiddenSetting(variable));
        }
    }
    Ok(())
}

/// Container Apps injects `IDENTITY_ENDPOINT` as a loopback HTTP URL. Anything
/// else is an operator-supplied token service that would receive the rotating
/// SSRF header. Absence is not checked here: with `COSMOS_AUTH_MODE` pinned to
/// managed identity a missing endpoint fails the first token request loudly.
fn validate_identity_endpoint() -> Result<(), ConfigError> {
    match optional_nonempty_env("IDENTITY_ENDPOINT")? {
        Some(value) => validate_identity_endpoint_value(&value),
        None => Ok(()),
    }
}

fn validate_identity_endpoint_value(value: &str) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::invalid(
            "IDENTITY_ENDPOINT",
            "must be the HTTP loopback URL injected by Azure Container Apps, without credentials, query, or fragment",
        )
    };
    let endpoint = url::Url::parse(value).map_err(|_| invalid())?;
    let local_host = endpoint.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if endpoint.scheme() != "http"
        || !local_host
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn require_managed_identity_auth_mode() -> Result<(), ConfigError> {
    match optional_nonempty_env("COSMOS_AUTH_MODE")? {
        Some(mode) if mode == COSMOS_AUTH_MODE => Ok(()),
        Some(_) => Err(ConfigError::invalid(
            "COSMOS_AUTH_MODE",
            format!("must be exactly '{COSMOS_AUTH_MODE}' for the Azure scheduler image"),
        )),
        None => Err(ConfigError::MissingVar("COSMOS_AUTH_MODE")),
    }
}

fn required_client_id() -> Result<String, ConfigError> {
    let client_id = optional_nonempty_env("AZURE_CLIENT_ID")?
        .ok_or(ConfigError::MissingVar("AZURE_CLIENT_ID"))?;
    uuid::Uuid::parse_str(&client_id).map_err(|_| {
        ConfigError::invalid(
            "AZURE_CLIENT_ID",
            "must be the UUID client ID of the job's user-assigned managed identity",
        )
    })?;
    Ok(client_id)
}

fn optional_nonempty_env(variable: &'static str) -> Result<Option<String>, ConfigError> {
    let Ok(value) = env::var(variable) else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(ConfigError::invalid(variable, "must not be empty when set"));
    }
    Ok(Some(value.to_string()))
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar(&'static str),
    InvalidValue {
        variable: &'static str,
        reason: String,
    },
    ForbiddenSetting(&'static str),
    Tick(TickError),
}

impl ConfigError {
    fn invalid(variable: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidValue {
            variable,
            reason: reason.into(),
        }
    }
}

impl From<TickError> for ConfigError {
    fn from(error: TickError) -> Self {
        Self::Tick(error)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            Self::ForbiddenSetting(variable) => write!(
                formatter,
                "{variable} is forbidden: the Azure scheduler must use the Container Apps managed identity"
            ),
            Self::Tick(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tick(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_container_apps_identity_endpoint() {
        assert!(validate_identity_endpoint_value("http://localhost:42356/msi/token").is_ok());
        assert!(validate_identity_endpoint_value("http://127.0.0.1:42356/msi/token").is_ok());
        assert!(validate_identity_endpoint_value("http://[::1]:42356/msi/token").is_ok());
    }

    #[test]
    fn rejects_identity_endpoints_that_could_exfiltrate_the_header() {
        assert!(validate_identity_endpoint_value("https://localhost/msi/token").is_err());
        assert!(validate_identity_endpoint_value("http://169.254.169.254/metadata").is_err());
        assert!(validate_identity_endpoint_value("http://attacker.example/msi/token").is_err());
        assert!(validate_identity_endpoint_value("http://user:pw@localhost/msi/token").is_err());
        assert!(validate_identity_endpoint_value("http://localhost/msi/token?x=1").is_err());
        assert!(validate_identity_endpoint_value("not a url").is_err());
    }

    #[test]
    fn forbidden_lists_do_not_overlap_the_settings_this_image_needs() {
        let required = [
            "API_BASE_URL",
            "API_URL",
            "SINK_TRIGGER_JWT",
            "SCHEDULER_STATE_BACKEND",
            "COSMOS_ENDPOINT",
            "COSMOS_DATABASE",
            "COSMOS_SCHEDULER_CONTAINER",
            "COSMOS_AUTH_MODE",
            "AZURE_CLIENT_ID",
            "IDENTITY_ENDPOINT",
            "IDENTITY_HEADER",
        ];
        for name in required {
            assert!(!FORBIDDEN_AZURE_SETTINGS.contains(&name), "{name}");
            assert!(!FORBIDDEN_IDENTITY_SETTINGS.contains(&name), "{name}");
        }
    }
}
