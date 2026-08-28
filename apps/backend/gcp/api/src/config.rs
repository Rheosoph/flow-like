use axum::http::{
    HeaderName, HeaderValue, Method,
    header::{
        ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH, RANGE, WWW_AUTHENTICATE,
    },
};
use flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env;
use flow_like_secrets::{GcpSecretManagerProviderConfig, ProviderConfig, SecretStoreConfig};
use std::{collections::HashSet, env, net::Ipv4Addr, time::Duration};
use tower_http::cors::{AllowOrigin, CorsLayer};
use url::Url;

/// The public Secret Manager endpoint, compiled in rather than read from the
/// environment. Every other Google client in this process resolves its endpoint
/// the same way, and the forbidden-settings scan below rejects the override
/// variables that could move one; leaving a knob here would reopen exactly the
/// hole that scan closes.
const SECRET_MANAGER_ENDPOINT: &str = "https://secretmanager.googleapis.com";

/// Settings that would let the process authenticate as something other than its
/// Cloud Run service account, reach something other than Google, or drop the
/// transport protection in between. These are live overrides on this image, not
/// hypothetical ones: `object_store`'s GCS builder recognises every `GOOGLE_*`
/// key below straight out of the environment, and `GcpConfig::from_env` in
/// packages/api reads `GOOGLE_APPLICATION_CREDENTIALS_JSON` (rejected by the
/// shared credential scan) as a service-account key.
///
/// Presence is fatal even when the value is empty. Whether an empty string
/// parses into something harmless is a property of a third-party parser we do
/// not control, and a variable that reached the deployment at all is evidence
/// that something intended it to take effect.
const FORBIDDEN_GCP_SETTINGS: &[&str] = &[
    // Long-lived service-account key material, in every spelling `object_store`
    // and the Google SDKs accept. This port has no key to configure: the
    // instance metadata server mints every token the process holds, and each
    // one expires within the hour.
    "GOOGLE_SERVICE_ACCOUNT",
    "GOOGLE_SERVICE_ACCOUNT_PATH",
    "GOOGLE_SERVICE_ACCOUNT_KEY",
    "SERVICE_ACCOUNT",
    // `GOOGLE_SKIP_SIGNATURE` makes every GCS request anonymous, which turns a
    // permission error into a silent read of whatever is publicly readable. The
    // rest downgrade or interpose the transport the bearer token travels over.
    "GOOGLE_SKIP_SIGNATURE",
    "GOOGLE_ALLOW_HTTP",
    "GOOGLE_ALLOW_INVALID_CERTIFICATES",
    "GOOGLE_PROXY_URL",
    "GOOGLE_PROXY_CA_CERTIFICATE",
    "GOOGLE_PROXY_EXCLUDES",
    // Emulators serve the API surface unauthenticated over plaintext. Pointing
    // a production process at one does not fail — it succeeds against an empty,
    // attacker-writable dataset, and the failure only surfaces as missing data.
    "STORAGE_EMULATOR_HOST",
    "PUBSUB_EMULATOR_HOST",
    "FIRESTORE_EMULATOR_HOST",
    "DATASTORE_EMULATOR_HOST",
    "SECRET_MANAGER_EMULATOR_HOST",
];

/// The provider selectors this image pins. `default_provider_name()` already
/// resolves to "gcp" because the AWS and Azure features are compiled out, so
/// these are not required — but a value naming another cloud means the workload
/// was handed the wrong environment entirely, and failing at startup is far
/// cheaper to diagnose than an "unknown provider" error on a user request.
const GCP_PROVIDER_SETTINGS: &[&str] = &[
    "STORAGE_PROVIDER",
    "RUNTIME_CREDENTIALS_PROVIDER",
    "META_BUCKET_PROVIDER",
    "CONTENT_BUCKET_PROVIDER",
    "LOGS_BUCKET_PROVIDER",
];

/// `BucketConfig::from_env` in packages/api prefers the GCP-specific name for
/// the meta, content and logs buckets — but prefers the *generic*
/// `CDN_BUCKET_NAME` over `GCP_CDN_BUCKET`. That inversion means a disagreement
/// between the two CDN variables makes this process build its CDN store from
/// one bucket while the rest of the API signs URLs against another. Nothing
/// errors; assets simply 404. Requiring the aliases to agree removes the
/// asymmetry rather than documenting it.
const BUCKET_ALIASES: &[(&str, &str)] = &[
    ("GCP_META_BUCKET", "META_BUCKET"),
    ("GCP_CONTENT_BUCKET", "CONTENT_BUCKET"),
    ("GCP_LOG_BUCKET", "LOG_BUCKET"),
    ("GCP_CDN_BUCKET", "CDN_BUCKET_NAME"),
];

/// Secret Manager caps a secret ID at 255 characters. The prefix is joined to
/// the secret name with a hyphen, so it is capped well below that to leave room
/// for the longest key this API resolves.
const MAX_SECRET_PREFIX_LENGTH: usize = 200;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub project_id: String,
    pub content_bucket: String,
    pub meta_bucket: String,
    pub cdn_bucket: String,
    pub log_bucket: String,
    pub secret_prefix: String,
    cors_allowed_origins: Vec<HeaderValue>,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Re-run even though main already did: every check below is meaningless
        // if the process can be made to authenticate as someone else, and the
        // scan is cheap enough that ordering it defensively costs nothing.
        reject_forbidden_environment()?;
        require_gcp_providers()?;

        // Cloud Run publishes exactly one container port and injects it as
        // `PORT`. There is deliberately no `METRICS_PORT` here — see main.rs.
        let port = parse_port("PORT", 8080)?;

        let project_id = required_env("GCP_PROJECT_ID")?;
        validate_project_id(&project_id)?;

        let content_bucket = required_bucket("GCP_CONTENT_BUCKET")?;
        let meta_bucket = required_bucket("GCP_META_BUCKET")?;
        let cdn_bucket = required_bucket("GCP_CDN_BUCKET")?;
        let log_bucket = required_bucket("GCP_LOG_BUCKET")?;

        // `GcpRuntimeCredentials::scoped_credentials` narrows a downscoped token
        // by *object prefix within a bucket*. Meta and content share the same
        // `apps/<app_id>/` prefix space, so pointing both at one bucket collapses
        // the `ReadAppContent` boundary into `ReadApp` and hands a content-only
        // holder the app's `.board` and `.event` definitions. The two buckets
        // being distinct is what makes the boundary mean anything.
        if meta_bucket == content_bucket {
            return Err(ConfigError::invalid(
                "GCP_META_BUCKET",
                "must be a different bucket from GCP_CONTENT_BUCKET; scoped credentials separate \
                 them by object prefix, which only isolates them across distinct buckets",
            ));
        }

        for &(canonical, alias) in BUCKET_ALIASES {
            require_matching_alias(canonical, alias)?;
        }

        let secret_prefix = required_env("SECRET_PREFIX")?;
        validate_secret_prefix(&secret_prefix)?;

        validate_mail_settings()?;
        validate_channel_settings()?;

        let cors_allowed_origins = parse_allowed_origins(&required_env("CORS_ALLOWED_ORIGINS")?)?;

        Ok(Self {
            port,
            project_id,
            content_bucket,
            meta_bucket,
            cdn_bucket,
            log_bucket,
            secret_prefix,
            cors_allowed_origins,
        })
    }

    pub fn secret_store_config(&self) -> SecretStoreConfig {
        // `allow_env_override(false)` is the point of this constructor. The
        // default store lets an exact-name environment variable shadow every
        // configured provider, which would mean anything that can write this
        // process's environment — a mis-templated Cloud Run revision, a
        // compromised sidecar contract — could substitute `BACKEND_KEY` or
        // `SINK_TOKEN_ENCRYPTION_KEY` without touching Secret Manager, and
        // without leaving an access log entry behind.
        SecretStoreConfig::default()
            .with_allow_env_override(false)
            .with_provider(ProviderConfig::GcpSecretManager(
                GcpSecretManagerProviderConfig {
                    project_id: Some(self.project_id.clone()),
                    endpoint: SECRET_MANAGER_ENDPOINT.to_string(),
                    prefix: Some(self.secret_prefix.clone()),
                },
            ))
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

/// Reject every environment setting that could redirect this process's
/// credentials or its traffic.
///
/// Exposed separately from `Config::from_env` so main can run it before the
/// first socket is opened: the Sentry transport honours the proxy variables, so
/// a process that started telemetry first would already have handed its DSN and
/// its first events to an operator-supplied proxy by the time configuration
/// parsing rejected it.
pub fn reject_forbidden_environment() -> Result<(), ConfigError> {
    ensure_no_forbidden_credential_env()
        .map_err(|error| ConfigError::ForbiddenCredentialSetting(error.to_string()))?;
    reject_forbidden_gcp_settings()
}

fn reject_forbidden_gcp_settings() -> Result<(), ConfigError> {
    for variable in FORBIDDEN_GCP_SETTINGS {
        if env::var_os(variable).is_some() {
            return Err(ConfigError::ForbiddenGcpSetting((*variable).to_string()));
        }
    }

    // `CLOUDSDK_API_ENDPOINT_OVERRIDES_<SERVICE>` and
    // `GOOGLE_<SERVICE>_CUSTOM_ENDPOINT` are open-ended families: one member
    // exists per Google service, and new services keep adding members. Matching
    // by shape means a service this image starts using tomorrow is covered
    // without anyone remembering to extend a list.
    for (name, _) in env::vars_os() {
        let name = name.to_string_lossy();
        if is_forbidden_endpoint_override(&name) {
            return Err(ConfigError::ForbiddenGcpSetting(name.into_owned()));
        }
    }

    Ok(())
}

fn is_forbidden_endpoint_override(name: &str) -> bool {
    name.starts_with("CLOUDSDK_API_ENDPOINT_OVERRIDES_")
        || (name.starts_with("GOOGLE_") && name.ends_with("_CUSTOM_ENDPOINT"))
}

fn require_gcp_providers() -> Result<(), ConfigError> {
    for variable in GCP_PROVIDER_SETTINGS {
        // The aliases are the ones `StorageProvider::from_str` and
        // `provider_to_runtime_credentials` accept; rejecting a spelling those
        // functions would have honoured only produces confusing failures.
        if let Some(value) = optional_nonempty_env(variable)?
            && !value.eq_ignore_ascii_case("gcp")
            && !value.eq_ignore_ascii_case("gcs")
            && !value.eq_ignore_ascii_case("google")
        {
            return Err(ConfigError::invalid(
                variable,
                "must be 'gcp' for the GCP API image",
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

fn require_matching_alias(canonical: &'static str, alias: &'static str) -> Result<(), ConfigError> {
    let Some(alias_value) = optional_nonempty_env(alias)? else {
        return Ok(());
    };
    let canonical_value = required_env(canonical)?;

    if alias_value != canonical_value {
        return Err(ConfigError::invalid(
            alias,
            "must name the same bucket as its GCP_-prefixed counterpart",
        ));
    }

    Ok(())
}

/// Validate the project ID's shape.
///
/// The value is interpolated into `projects/<id>/secrets/<name>/versions/latest`
/// resource names. A templating slip that produced a syntactically valid but
/// different project would resolve secrets somewhere else entirely, so the shape
/// is checked here rather than discovered as a permission error later.
fn validate_project_id(value: &str) -> Result<(), ConfigError> {
    let valid = (6..=30).contains(&value.len())
        && value.starts_with(|character: char| character.is_ascii_lowercase())
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');

    if !valid {
        return Err(ConfigError::invalid(
            "GCP_PROJECT_ID",
            "must be 6-30 characters of lowercase letters, digits or hyphens, starting with a \
             letter and not ending in a hyphen",
        ));
    }

    Ok(())
}

fn required_bucket(variable: &'static str) -> Result<String, ConfigError> {
    let value = required_env(variable)?;
    validate_bucket_name(variable, &value)?;
    Ok(value)
}

/// Validate a Cloud Storage bucket name.
///
/// The name is concatenated into every object URL this process builds and into
/// the resource strings a downscoped token's Credential Access Boundary is
/// written against. A value that is not a legal bucket name cannot be the bucket
/// Terraform created, so accepting one only defers the failure to the first
/// request — after the credential boundary has already been minted for it.
fn validate_bucket_name(variable: &'static str, value: &str) -> Result<(), ConfigError> {
    let valid_characters = value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    });
    let valid_edges = value
        .as_bytes()
        .first()
        .zip(value.as_bytes().last())
        .is_some_and(|(first, last)| first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric());
    // Google reserves the `goog` prefix and anything containing `google`, so a
    // name carrying one can never be a bucket that exists.
    let reserved = value.starts_with("goog") || value.contains("google");

    if !(3..=63).contains(&value.len())
        || !valid_characters
        || !valid_edges
        || value.contains("..")
        || reserved
        || value.parse::<Ipv4Addr>().is_ok()
    {
        return Err(ConfigError::invalid(
            variable,
            "must be a valid Cloud Storage bucket name (3-63 lowercase letters, digits, hyphens, \
             underscores or single dots, not an IP address, and not Google-reserved)",
        ));
    }

    Ok(())
}

/// Cloud Run mounts SMTP relay credentials from Secret Manager into the process
/// environment under the names the compiled-in hub configuration points at, so
/// nothing about the relay itself is checkable here. What is checkable is the
/// selector: `create_mail_client` treats `MAIL_PROVIDER` as a hard error for any
/// unrecognised value, and GCP has no first-party transactional email service —
/// there is no correct value other than `smtp` on this image, and a wrong one
/// takes the whole process down at the first password-reset email rather than at
/// startup.
fn validate_mail_settings() -> Result<(), ConfigError> {
    let provider = required_env("MAIL_PROVIDER")?;
    if !provider.eq_ignore_ascii_case("smtp") {
        return Err(ConfigError::invalid(
            "MAIL_PROVIDER",
            "must be 'smtp' for the GCP API image; GCP has no first-party transactional email \
             service, so the deployment brings its own relay",
        ));
    }

    Ok(())
}

/// Validate the Secret Manager prefix.
///
/// `GcpSecretManagerProvider` flattens a path-style prefix onto the secret name
/// with hyphens, so `/flow-like/gcp-dev` and `flow-like-gcp-dev` address the same
/// secret. It also always appends the *unprefixed* name as a fallback candidate,
/// which means the prefix isolates environments only as far as the service
/// account's per-secret IAM does — see the README. Validating the normalized form
/// here keeps a prefix that Secret Manager would reject from silently degrading
/// every lookup into that unprefixed fallback.
fn validate_secret_prefix(value: &str) -> Result<(), ConfigError> {
    let normalized = value.trim_matches('/').replace('/', "-");
    if normalized.is_empty()
        || normalized.len() > MAX_SECRET_PREFIX_LENGTH
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ConfigError::invalid(
            "SECRET_PREFIX",
            "must normalize to 1-200 alphanumeric, hyphen or underscore characters",
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

/// `CHANNEL_TRANSPORT` may only name a transport this image carries, and the Realtime Database
/// transport needs its database and Web API key up front. The signing service account lives in
/// Secret Manager (`CHANNEL_FIREBASE_SERVICE_ACCOUNT`) and is checked once the secret store is
/// up (see `validate_security_prerequisites` in main.rs).
fn validate_channel_settings() -> Result<(), ConfigError> {
    let transport = optional_nonempty_env("CHANNEL_TRANSPORT")?;
    let backend =
        flow_like_api::channel::ChannelBackend::parse(transport.as_deref()).map_err(|_| {
            ConfigError::invalid(
                "CHANNEL_TRANSPORT",
                "must be 'http' or 'gcp_firebase_rtdb' on the GCP API image",
            )
        })?;
    if backend.is_http() {
        return Ok(());
    }

    let database_url = required_env("CHANNEL_FIREBASE_DATABASE_URL")?;
    validate_firebase_database_url(&database_url)?;
    required_env("CHANNEL_FIREBASE_API_KEY")?;
    if let Some(project_id) = optional_nonempty_env("CHANNEL_FIREBASE_PROJECT_ID")? {
        validate_project_id(&project_id)?;
    }

    Ok(())
}

fn validate_firebase_database_url(value: &str) -> Result<(), ConfigError> {
    let url = Url::parse(value).map_err(|_| {
        ConfigError::invalid("CHANNEL_FIREBASE_DATABASE_URL", "must be a valid URL")
    })?;
    let valid_host = url.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host.ends_with(".firebasedatabase.app") || host.ends_with(".firebaseio.com")
    });
    let valid_path = url.path().is_empty() || url.path() == "/";

    if url.scheme() != "https"
        || !valid_host
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_path
    {
        return Err(ConfigError::invalid(
            "CHANNEL_FIREBASE_DATABASE_URL",
            "must be a Firebase Realtime Database HTTPS origin without credentials, port, path, query, or fragment",
        ));
    }

    Ok(())
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar(&'static str),
    InvalidValue {
        variable: &'static str,
        reason: &'static str,
    },
    ForbiddenGcpSetting(String),
    ForbiddenCredentialSetting(String),
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
            Self::ForbiddenGcpSetting(variable) => write!(
                formatter,
                "{variable} is forbidden: the GCP API must reach Google's own endpoints with a \
                 metadata-server token and no static credential"
            ),
            Self::ForbiddenCredentialSetting(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_firebase_database_url() {
        assert!(
            validate_firebase_database_url(
                "https://demo-default-rtdb.europe-west1.firebasedatabase.app"
            )
            .is_ok()
        );
        assert!(validate_firebase_database_url("https://demo.firebaseio.com/").is_ok());
        assert!(validate_firebase_database_url("http://demo.firebaseio.com").is_err());
        assert!(validate_firebase_database_url("https://firebaseio.com.attacker.example").is_err());
        assert!(validate_firebase_database_url("https://demo.firebaseio.com/channels").is_err());
    }

    #[test]
    fn accepts_valid_gcp_names() {
        assert!(validate_project_id("flow-like-dev").is_ok());
        assert!(validate_bucket_name("TEST", "flowlike-dev-content").is_ok());
        assert!(validate_bucket_name("TEST", "flowlike.dev.content").is_ok());
    }

    #[test]
    fn rejects_invalid_gcp_names() {
        assert!(validate_project_id("Flow-Like-Dev").is_err());
        assert!(validate_project_id("dev-").is_err());
        assert!(validate_project_id("1dev-project").is_err());
        assert!(validate_bucket_name("TEST", "FlowLike").is_err());
        assert!(validate_bucket_name("TEST", "-flowlike").is_err());
        assert!(validate_bucket_name("TEST", "flow..like").is_err());
    }

    #[test]
    fn rejects_bucket_names_google_reserves_or_that_shadow_an_address() {
        assert!(validate_bucket_name("TEST", "goog-flowlike").is_err());
        assert!(validate_bucket_name("TEST", "flowlike-google-dev").is_err());
        assert!(validate_bucket_name("TEST", "192.168.0.1").is_err());
    }

    #[test]
    fn matches_the_open_ended_endpoint_override_families() {
        assert!(is_forbidden_endpoint_override(
            "CLOUDSDK_API_ENDPOINT_OVERRIDES_SECRETMANAGER"
        ));
        assert!(is_forbidden_endpoint_override(
            "GOOGLE_STORAGE_CUSTOM_ENDPOINT"
        ));
        assert!(!is_forbidden_endpoint_override("GOOGLE_PROJECT_ID"));
        assert!(!is_forbidden_endpoint_override("CUSTOM_ENDPOINT"));
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
    fn normalizes_path_style_secret_prefixes_and_rejects_illegal_ones() {
        assert!(validate_secret_prefix("/flow-like/gcp-dev").is_ok());
        assert!(validate_secret_prefix("flow-like-gcp-dev").is_ok());
        assert!(validate_secret_prefix("/").is_err());
        assert!(validate_secret_prefix("flow like").is_err());
        assert!(validate_secret_prefix(&"a".repeat(MAX_SECRET_PREFIX_LENGTH + 1)).is_err());
    }
}
