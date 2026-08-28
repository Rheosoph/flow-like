use axum::http::{
    HeaderName, HeaderValue, Method,
    header::{
        ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH, RANGE, WWW_AUTHENTICATE,
    },
};
use flow_like_secrets::{
    AzureCredentialConfig, AzureKeyVaultProviderConfig, AzureManagedIdentityConfig,
    AzureManagedIdentityId, ProviderConfig, SecretStoreConfig,
};
use std::{collections::HashSet, env, time::Duration};
use tower_http::cors::{AllowOrigin, CorsLayer};
use url::Url;

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

const AZURE_PROVIDER_SETTINGS: &[&str] = &[
    "STORAGE_PROVIDER",
    "RUNTIME_CREDENTIALS_PROVIDER",
    "META_BUCKET_PROVIDER",
    "CONTENT_BUCKET_PROVIDER",
    "LOGS_BUCKET_PROVIDER",
];

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub metrics_port: u16,
    pub storage_account_name: String,
    pub content_container: String,
    pub meta_container: String,
    pub cdn_container: String,
    pub log_container: String,
    pub key_vault_url: String,
    pub secret_prefix: String,
    pub managed_identity_client_id: Option<String>,
    cors_allowed_origins: Vec<HeaderValue>,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        reject_forbidden_azure_settings()?;
        require_azure_providers()?;

        let port = parse_port("PORT", 8080)?;
        let metrics_port = parse_port("METRICS_PORT", 9090)?;
        if port == metrics_port {
            return Err(ConfigError::invalid(
                "METRICS_PORT",
                "must differ from PORT",
            ));
        }

        let storage_account_name = required_env("AZURE_STORAGE_ACCOUNT_NAME")?;
        validate_storage_account_name(&storage_account_name)?;

        let content_container = required_container("AZURE_CONTENT_CONTAINER")?;
        let meta_container = required_container("AZURE_META_CONTAINER")?;
        let cdn_container = required_container("AZURE_CDN_CONTAINER")?;
        let log_container = required_container("AZURE_LOG_CONTAINER")?;

        let key_vault_url = required_env("AZURE_KEY_VAULT_URL")?;
        validate_key_vault_url(&key_vault_url)?;

        let secret_prefix = required_env("SECRET_PREFIX")?;
        validate_secret_prefix(&secret_prefix)?;

        let managed_identity_client_id = optional_nonempty_env("AZURE_CLIENT_ID")?;
        if let Some(client_id) = &managed_identity_client_id {
            uuid::Uuid::parse_str(client_id).map_err(|_| {
                ConfigError::invalid("AZURE_CLIENT_ID", "must be a valid UUID client ID")
            })?;
        }

        validate_acs_email_settings()?;
        validate_channel_settings()?;

        let cors_allowed_origins = parse_allowed_origins(&required_env("CORS_ALLOWED_ORIGINS")?)?;

        Ok(Self {
            port,
            metrics_port,
            storage_account_name,
            content_container,
            meta_container,
            cdn_container,
            log_container,
            key_vault_url,
            secret_prefix,
            managed_identity_client_id,
            cors_allowed_origins,
        })
    }

    pub fn secret_store_config(&self) -> SecretStoreConfig {
        let user_assigned_id = self
            .managed_identity_client_id
            .clone()
            .map(AzureManagedIdentityId::ClientId);

        SecretStoreConfig::default()
            .with_allow_env_override(false)
            .with_provider(ProviderConfig::AzureKeyVault(AzureKeyVaultProviderConfig {
                vault_url: self.key_vault_url.clone(),
                prefix: Some(self.secret_prefix.clone()),
                credential: AzureCredentialConfig::ManagedIdentity(AzureManagedIdentityConfig {
                    user_assigned_id,
                }),
                verify_challenge_resource: true,
                fallback_to_unprefixed: false,
                ..Default::default()
            }))
    }

    pub fn cors_layer(&self) -> CorsLayer {
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(self.cors_allowed_origins.clone()))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
                Method::HEAD,
            ])
            .allow_headers([
                AUTHORIZATION,
                CONTENT_TYPE,
                ACCEPT,
                RANGE,
                IF_MATCH,
                IF_NONE_MATCH,
                HeaderName::from_static("x-api-key"),
                HeaderName::from_static("x-flow-like-app-id"),
                HeaderName::from_static("x-flow-like-event-authorization"),
                HeaderName::from_static("x-request-id"),
                HeaderName::from_static("idempotency-key"),
                HeaderName::from_static("flowlike-idempotency-ack"),
                HeaderName::from_static("mcp-session-id"),
                HeaderName::from_static("mcp-protocol-version"),
                HeaderName::from_static("last-event-id"),
                HeaderName::from_static("traceparent"),
                HeaderName::from_static("tracestate"),
                HeaderName::from_static("baggage"),
            ])
            .expose_headers([
                ETAG,
                WWW_AUTHENTICATE,
                HeaderName::from_static("content-range"),
                HeaderName::from_static("x-error-id"),
                HeaderName::from_static("x-request-id"),
                HeaderName::from_static("flowlike-idempotency-ack"),
                HeaderName::from_static("mcp-session-id"),
                HeaderName::from_static("mcp-protocol-version"),
            ])
            .max_age(Duration::from_secs(3600))
    }

    pub fn cors_origin_count(&self) -> usize {
        self.cors_allowed_origins.len()
    }
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

fn require_azure_providers() -> Result<(), ConfigError> {
    for variable in AZURE_PROVIDER_SETTINGS {
        if let Some(value) = optional_nonempty_env(variable)?
            && !value.eq_ignore_ascii_case("azure")
            && !value.eq_ignore_ascii_case("blob")
        {
            return Err(ConfigError::invalid(
                variable,
                "must be 'azure' for the Azure API image",
            ));
        }
    }

    Ok(())
}

fn parse_port(variable: &'static str, default: u16) -> Result<u16, ConfigError> {
    let raw = env::var(variable).unwrap_or_else(|_| default.to_string());
    let port = raw
        .parse::<u16>()
        .map_err(|_| ConfigError::invalid(variable, "must be a valid TCP port"))?;

    if port == 0 {
        return Err(ConfigError::invalid(variable, "must be greater than zero"));
    }

    Ok(port)
}

fn required_env(variable: &'static str) -> Result<String, ConfigError> {
    let value = env::var(variable).map_err(|_| ConfigError::MissingVar(variable))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(ConfigError::invalid(variable, "must not be empty"));
    }

    Ok(value.to_string())
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

fn validate_storage_account_name(value: &str) -> Result<(), ConfigError> {
    if !(3..=24).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(ConfigError::invalid(
            "AZURE_STORAGE_ACCOUNT_NAME",
            "must contain 3-24 lowercase ASCII letters or digits",
        ));
    }

    Ok(())
}

fn required_container(variable: &'static str) -> Result<String, ConfigError> {
    let value = required_env(variable)?;
    validate_container_name(variable, &value)?;
    Ok(value)
}

fn validate_container_name(variable: &'static str, value: &str) -> Result<(), ConfigError> {
    let valid_characters = value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    let valid_edges = value
        .as_bytes()
        .first()
        .zip(value.as_bytes().last())
        .is_some_and(|(first, last)| {
            (first.is_ascii_lowercase() || first.is_ascii_digit())
                && (last.is_ascii_lowercase() || last.is_ascii_digit())
        });

    if !(3..=63).contains(&value.len()) || !valid_characters || !valid_edges || value.contains("--")
    {
        return Err(ConfigError::invalid(
            variable,
            "must be a valid Azure container name (3-63 lowercase letters, digits, or single hyphens)",
        ));
    }

    Ok(())
}

fn validate_key_vault_url(value: &str) -> Result<(), ConfigError> {
    let parsed = Url::parse(value)
        .map_err(|_| ConfigError::invalid("AZURE_KEY_VAULT_URL", "must be a valid URL"))?;
    let valid_path = parsed.path().is_empty() || parsed.path() == "/";

    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !valid_path
    {
        return Err(ConfigError::invalid(
            "AZURE_KEY_VAULT_URL",
            "must be an HTTPS vault origin without credentials, path, query, or fragment",
        ));
    }

    Ok(())
}

fn validate_acs_email_settings() -> Result<(), ConfigError> {
    let provider = required_env("MAIL_PROVIDER")?;
    if !provider.eq_ignore_ascii_case("azure_communication_services") {
        return Err(ConfigError::invalid(
            "MAIL_PROVIDER",
            "must be 'azure_communication_services' for the Azure API image",
        ));
    }

    let endpoint = required_env("ACS_EMAIL_ENDPOINT")?;
    validate_acs_email_endpoint(&endpoint)?;

    let sender = required_env("ACS_EMAIL_SENDER")?;
    validate_acs_email_sender(&sender)?;

    if optional_nonempty_env("AZURE_CLIENT_ID")?.is_none() {
        return Err(ConfigError::invalid(
            "AZURE_CLIENT_ID",
            "is required for the ACS user-assigned managed identity",
        ));
    }

    Ok(())
}

/// `CHANNEL_TRANSPORT` may only name a transport this image carries, and the Web PubSub
/// transport needs its endpoint up front. The access key lives in Key Vault and is checked once
/// the secret store is up (see `validate_security_prerequisites` in main.rs).
fn validate_channel_settings() -> Result<(), ConfigError> {
    let transport = optional_nonempty_env("CHANNEL_TRANSPORT")?;
    let backend =
        flow_like_api::channel::ChannelBackend::parse(transport.as_deref()).map_err(|_| {
            ConfigError::invalid(
                "CHANNEL_TRANSPORT",
                "must be 'http' or 'azure_web_pubsub' on the Azure API image",
            )
        })?;
    if backend.is_http() {
        return Ok(());
    }

    let endpoint = required_env("CHANNEL_WEBPUBSUB_ENDPOINT")?;
    validate_webpubsub_endpoint(&endpoint)?;
    if let Some(hub) = optional_nonempty_env("CHANNEL_WEBPUBSUB_HUB")? {
        validate_webpubsub_hub(&hub)?;
    }

    Ok(())
}

fn validate_webpubsub_endpoint(value: &str) -> Result<(), ConfigError> {
    let endpoint = Url::parse(value)
        .map_err(|_| ConfigError::invalid("CHANNEL_WEBPUBSUB_ENDPOINT", "must be a valid URL"))?;
    let valid_host = endpoint
        .host_str()
        .is_some_and(|host| host.to_ascii_lowercase().ends_with(".webpubsub.azure.com"));
    let valid_path = endpoint.path().is_empty() || endpoint.path() == "/";

    if endpoint.scheme() != "https"
        || !valid_host
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.port().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !valid_path
    {
        return Err(ConfigError::invalid(
            "CHANNEL_WEBPUBSUB_ENDPOINT",
            "must be a public-cloud Web PubSub HTTPS origin without credentials, port, path, query, or fragment",
        ));
    }

    Ok(())
}

fn validate_webpubsub_hub(value: &str) -> Result<(), ConfigError> {
    let valid = (1..=127).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');

    if !valid {
        return Err(ConfigError::invalid(
            "CHANNEL_WEBPUBSUB_HUB",
            "must start with a letter and contain only letters, digits, or underscores (1-127 characters)",
        ));
    }

    Ok(())
}

fn validate_acs_email_endpoint(value: &str) -> Result<(), ConfigError> {
    let endpoint = Url::parse(value)
        .map_err(|_| ConfigError::invalid("ACS_EMAIL_ENDPOINT", "must be a valid URL"))?;
    let valid_host = endpoint.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host.ends_with(".communication.azure.com") || host.ends_with(".communications.azure.com")
    });
    let valid_path = endpoint.path().is_empty() || endpoint.path() == "/";

    if endpoint.scheme() != "https"
        || !valid_host
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.port().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !valid_path
    {
        return Err(ConfigError::invalid(
            "ACS_EMAIL_ENDPOINT",
            "must be a public-cloud ACS HTTPS origin without credentials, port, path, query, or fragment",
        ));
    }

    Ok(())
}

fn validate_acs_email_sender(value: &str) -> Result<(), ConfigError> {
    let mut parts = value.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    let valid = !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && parts.next().is_none()
        && value.len() <= 254
        && value.is_ascii()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());

    if !valid {
        return Err(ConfigError::invalid(
            "ACS_EMAIL_SENDER",
            "must be a single valid ASCII sender address",
        ));
    }

    Ok(())
}

fn validate_secret_prefix(value: &str) -> Result<(), ConfigError> {
    let normalized = value.trim_matches('/').replace('/', "-");
    if normalized.is_empty()
        || normalized.len() > 64
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(ConfigError::invalid(
            "SECRET_PREFIX",
            "must normalize to 1-64 alphanumeric or hyphen characters",
        ));
    }

    Ok(())
}

fn parse_allowed_origins(value: &str) -> Result<Vec<HeaderValue>, ConfigError> {
    let mut origins = Vec::new();
    let mut seen = HashSet::new();

    for raw_origin in value.split(',') {
        let raw_origin = raw_origin.trim();
        if raw_origin.is_empty() || raw_origin == "*" {
            return Err(ConfigError::invalid(
                "CORS_ALLOWED_ORIGINS",
                "must contain only explicit HTTPS origins",
            ));
        }

        let parsed = Url::parse(raw_origin)
            .map_err(|_| ConfigError::invalid("CORS_ALLOWED_ORIGINS", "contains an invalid URL"))?;
        let valid_path = parsed.path().is_empty() || parsed.path() == "/";
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !valid_path
        {
            return Err(ConfigError::invalid(
                "CORS_ALLOWED_ORIGINS",
                "must contain only HTTPS origins without credentials, paths, queries, or fragments",
            ));
        }

        let origin = parsed.origin().ascii_serialization();
        if seen.insert(origin.clone()) {
            origins.push(HeaderValue::from_str(&origin).map_err(|_| {
                ConfigError::invalid("CORS_ALLOWED_ORIGINS", "contains an invalid HTTP origin")
            })?);
        }
    }

    if origins.is_empty() {
        return Err(ConfigError::invalid(
            "CORS_ALLOWED_ORIGINS",
            "must contain at least one HTTPS origin",
        ));
    }

    Ok(origins)
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
                "{variable} is forbidden: the Azure API must use managed identity"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_azure_names() {
        assert!(validate_storage_account_name("flowlikedev01").is_ok());
        assert!(validate_container_name("TEST", "flow-like-content").is_ok());
    }

    #[test]
    fn rejects_invalid_azure_names() {
        assert!(validate_storage_account_name("FlowLike_Dev").is_err());
        assert!(validate_container_name("TEST", "flow--like").is_err());
        assert!(validate_container_name("TEST", "-flow-like").is_err());
    }

    #[test]
    fn parses_and_deduplicates_exact_https_origins() {
        let origins = parse_allowed_origins(
            "https://app.example.com, https://admin.example.com/, https://app.example.com",
        )
        .expect("valid origins must parse");

        assert_eq!(origins.len(), 2);
        assert_eq!(
            origins[0],
            HeaderValue::from_static("https://app.example.com")
        );
        assert_eq!(
            origins[1],
            HeaderValue::from_static("https://admin.example.com")
        );
    }

    #[test]
    fn rejects_wildcard_http_and_path_origins() {
        assert!(parse_allowed_origins("*").is_err());
        assert!(parse_allowed_origins("http://app.example.com").is_err());
        assert!(parse_allowed_origins("https://app.example.com/path").is_err());
    }

    #[test]
    fn validates_acs_email_endpoint_origin() {
        assert!(validate_acs_email_endpoint("https://flowlike.communication.azure.com").is_ok());
        assert!(
            validate_acs_email_endpoint("https://flowlike.westeurope.communications.azure.com/")
                .is_ok()
        );
        assert!(
            validate_acs_email_endpoint("https://communication.azure.com.attacker.example")
                .is_err()
        );
        assert!(
            validate_acs_email_endpoint("https://flowlike.communication.azure.com/path").is_err()
        );
    }

    #[test]
    fn validates_webpubsub_settings() {
        assert!(validate_webpubsub_endpoint("https://flowlike.webpubsub.azure.com").is_ok());
        assert!(validate_webpubsub_endpoint("https://flowlike.webpubsub.azure.com/").is_ok());
        assert!(validate_webpubsub_endpoint("http://flowlike.webpubsub.azure.com").is_err());
        assert!(
            validate_webpubsub_endpoint("https://webpubsub.azure.com.attacker.example").is_err()
        );
        assert!(validate_webpubsub_endpoint("https://flowlike.webpubsub.azure.com/hub").is_err());
        assert!(validate_webpubsub_hub("channels").is_ok());
        assert!(validate_webpubsub_hub("run_channels2").is_ok());
        assert!(validate_webpubsub_hub("1channels").is_err());
        assert!(validate_webpubsub_hub("chan-nels").is_err());
        assert!(validate_webpubsub_hub("").is_err());
    }

    #[test]
    fn validates_acs_sender_as_one_ascii_mailbox() {
        assert!(validate_acs_email_sender("DoNotReply@example.azurecomm.net").is_ok());
        assert!(validate_acs_email_sender("first@example.com,second@example.com").is_err());
        assert!(validate_acs_email_sender("sender@example.com\r\nBcc:victim@example.com").is_err());
    }
}
