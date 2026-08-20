use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flow_like_executor::ExecutorConfig;
use flow_like_gcp_data::metadata::ensure_no_forbidden_credential_env;
use jsonwebtoken::DecodingKey;
use std::env;

/// Cloud Run terminates a service request at `timeout_seconds`, and the largest
/// value the platform accepts is 3600. A run whose own timeout is at or above
/// that ceiling is therefore always cut by Cloud Run first: hyper drops the
/// handler future when the connection goes away, `execute()` is cancelled
/// mid-flight, and the terminal status callback the API is waiting for is never
/// sent. The executor's timeout has to fire first so the run ends with a
/// reported failure rather than a vanished one.
const CLOUD_RUN_MAX_REQUEST_SECS: u64 = 3_600;
/// Seconds reserved between the executor's own timeout and the Cloud Run
/// ceiling for the timeout error to be reported through the callback batcher.
const TERMINAL_CALLBACK_MARGIN_SECS: u64 = 30;
const MAX_EXECUTION_TIMEOUT_SECS: u64 = CLOUD_RUN_MAX_REQUEST_SECS - TERMINAL_CALLBACK_MARGIN_SECS;

/// The spellings the Kubernetes executor reads as "server mode". Kept identical
/// so a value that selects server mode there selects it here, and a value that
/// would have selected job mode there is refused rather than reinterpreted.
const SERVER_MODE_TRUE: &[&str] = &["1", "true"];

/// Settings that would let the process authenticate as something other than its
/// Cloud Run service account, reach something other than Google, or drop the
/// transport protection in between. The list is gcp-api's, and it is enforced
/// here for the same reason even though this image configures no Google client
/// of its own. Two paths make it live regardless: a request whose `credentials`
/// block is keyless makes `flow_like::credentials` build a bare
/// `GoogleCloudStorageBuilder`, which resolves through object_store's own chain
/// — service-account key, ADC file, then the metadata server — and every
/// `GOOGLE_*` key below is read straight out of the environment on that path;
/// and the catalog's Vertex nodes build Application Default Credentials
/// (`google_credentials::Builder::default()`), which read the key-file variables
/// `flow_like_gcp_data` rejects and then fall back to the metadata server the
/// `GCE_METADATA_*` variables would redirect.
///
/// Presence is fatal even when the value is empty. Whether an empty string
/// parses into something harmless is a property of a third-party parser we do
/// not control, and a variable that reached the deployment at all is evidence
/// that something intended it to take effect.
const FORBIDDEN_GCP_SETTINGS: &[&str] = &[
    // Long-lived service-account key material, in every spelling `object_store`
    // and the Google SDKs accept. This image has no key to configure.
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

/// One tuning knob: the environment variable name paired with the parse that
/// decides whether the configured value is acceptable.
type TuningKnob = (&'static str, fn(&str) -> bool);

/// Every knob `ExecutorConfig::from_env` reads, paired with the parse it
/// applies. That constructor falls back to the default on a value it cannot
/// parse, so `EXECUTOR_TIMEOUT_SECS=48O` would silently become 3600 — the one
/// value this platform can never honour. Refusing the unparsable value here is
/// what makes the ceiling check below mean what it says.
const EXECUTOR_TUNING: &[TuningKnob] = &[
    ("EXECUTOR_BATCH_INTERVAL_MS", parses_u64),
    ("EXECUTOR_MAX_BATCH_SIZE", parses_usize),
    ("EXECUTOR_CALLBACK_TIMEOUT_MS", parses_u64),
    ("EXECUTOR_CALLBACK_RETRIES", parses_u32),
    ("EXECUTOR_TIMEOUT_SECS", parses_u64),
];

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub executor: ExecutorConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Re-run even though main already did: every check below is meaningless
        // if the process can be made to authenticate as someone else, and the
        // scan is cheap enough that ordering it defensively costs nothing.
        reject_forbidden_environment()?;
        require_server_mode()?;
        reject_metrics_port()?;

        // Cloud Run publishes exactly one container port and injects it as
        // `PORT`. There is deliberately no `METRICS_PORT` here — see main.rs.
        let port = parse_port("PORT", 8080)?;

        validate_backend_public_key()?;
        validate_executor_tuning()?;
        let executor = ExecutorConfig::from_env();
        validate_execution_timeout(executor.execution_timeout_secs)?;

        Ok(Self { port, executor })
    }
}

/// Reject every environment setting that could redirect this process's
/// credentials or its traffic.
///
/// Exposed separately from `Config::from_env` so main can run it before the
/// first socket is opened: the OTLP exporter honours the proxy variables, so a
/// process that started telemetry first would already have handed its first
/// spans to an operator-supplied proxy by the time configuration parsing
/// rejected it.
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
    // by shape means a service a flow starts using tomorrow is covered without
    // anyone remembering to extend a list.
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

/// This image is server-only. The Kubernetes executor treats anything other
/// than `1`/`true` as "run one job and exit", and an environment that says so
/// was written for that binary — starting a server under it would silently do
/// something other than what the operator configured, so the mismatch is named
/// at boot instead. An unset variable requests nothing and is accepted: there
/// is no other mode for it to have selected.
fn require_server_mode() -> Result<(), ConfigError> {
    let Some(value) = env::var_os("EXECUTOR_SERVER_MODE") else {
        return Ok(());
    };
    let value = value.to_string_lossy();
    // Untrimmed on purpose: `EXECUTOR_SERVER_MODE="true "` selects job mode on
    // Kubernetes, and the point of this check is to agree with that binary about
    // what the value means.
    if SERVER_MODE_TRUE
        .iter()
        .any(|accepted| value.eq_ignore_ascii_case(accepted))
    {
        return Ok(());
    }

    Err(ConfigError::JobModeRequested(value.into_owned()))
}

/// Cloud Run publishes one container port, so a second Prometheus listener
/// would bind a socket nothing can reach. Ignoring the variable would leave an
/// operator porting the Azure or Kubernetes executor environment believing a
/// scrape endpoint exists where it does not; refusing it says where `/metrics`
/// actually is.
fn reject_metrics_port() -> Result<(), ConfigError> {
    if env::var_os("METRICS_PORT").is_some() {
        return Err(ConfigError::MetricsPortSet);
    }
    Ok(())
}

/// The executor verifies every `executor_jwt` against `BACKEND_PUB` before it
/// touches the request. `flow_like_executor::jwt` reads the variable lazily and
/// falls back to fetching JWKS from `API_URL`, which this deployment does not
/// set (the API's address arrives as `API_BASE_URL`), so a missing or malformed
/// key would surface as a 500 on the first execution rather than at boot. The
/// value is decoded exactly as that module decodes it — standard base64,
/// untrimmed, then parsed as an EC PEM public key.
fn validate_backend_public_key() -> Result<(), ConfigError> {
    let encoded = env::var("BACKEND_PUB").map_err(|_| ConfigError::MissingVar("BACKEND_PUB"))?;
    if encoded.trim().is_empty() {
        return Err(ConfigError::invalid("BACKEND_PUB", "must not be empty"));
    }

    // Untrimmed on purpose: the strict base64 decoder rejects a trailing newline,
    // and the executor decodes the raw value, so a value that fails here would
    // fail every request.
    let pem = BASE64_STANDARD.decode(encoded.as_bytes()).map_err(|_| {
        ConfigError::invalid(
            "BACKEND_PUB",
            "must be the standard-base64 encoding of a PEM public key, with no surrounding \
             whitespace or line breaks",
        )
    })?;
    DecodingKey::from_ec_pem(&pem).map_err(|error| {
        ConfigError::invalid(
            "BACKEND_PUB",
            format!(
                "must decode to a PEM-encoded EC public key; ES256 verification would reject \
                 every executor JWT ({error})"
            ),
        )
    })?;

    Ok(())
}

fn validate_executor_tuning() -> Result<(), ConfigError> {
    for &(variable, parses) in EXECUTOR_TUNING {
        // Read the way `ExecutorConfig::from_env` reads it — untrimmed — so the
        // value accepted here is the value that constructor will parse.
        let Ok(value) = env::var(variable) else {
            continue;
        };
        if !parses(&value) {
            return Err(ConfigError::invalid(
                variable,
                "must be a non-negative integer; ExecutorConfig would silently fall back to its \
                 default otherwise",
            ));
        }
    }
    Ok(())
}

fn validate_execution_timeout(seconds: u64) -> Result<(), ConfigError> {
    if !(1..=MAX_EXECUTION_TIMEOUT_SECS).contains(&seconds) {
        return Err(ConfigError::invalid(
            "EXECUTOR_TIMEOUT_SECS",
            format!(
                "must be between 1 and {MAX_EXECUTION_TIMEOUT_SECS}; Cloud Run cuts a service \
                 request at {CLOUD_RUN_MAX_REQUEST_SECS}s at most, and the executor's own timeout \
                 must fire {TERMINAL_CALLBACK_MARGIN_SECS}s before that so the run ends with a \
                 reported failure instead of a cancelled handler"
            ),
        ));
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

fn parses_u64(value: &str) -> bool {
    value.parse::<u64>().is_ok()
}

fn parses_u32(value: &str) -> bool {
    value.parse::<u32>().is_ok()
}

fn parses_usize(value: &str) -> bool {
    value.parse::<usize>().is_ok()
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar(&'static str),
    InvalidValue {
        variable: &'static str,
        reason: String,
    },
    ForbiddenGcpSetting(String),
    ForbiddenCredentialSetting(String),
    JobModeRequested(String),
    MetricsPortSet,
}

impl ConfigError {
    fn invalid(variable: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidValue {
            variable,
            reason: reason.into(),
        }
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
                "{variable} is forbidden: the GCP executor must reach Google's own endpoints with \
                 the credentials inside each signed request or a metadata-server token, and no \
                 static credential"
            ),
            Self::ForbiddenCredentialSetting(detail) => formatter.write_str(detail),
            Self::JobModeRequested(value) => write!(
                formatter,
                "EXECUTOR_SERVER_MODE={value:?} selects the Kubernetes executor's job-once mode; \
                 this image is server-only and has no such mode — set it to true or unset it"
            ),
            Self::MetricsPortSet => formatter.write_str(
                "METRICS_PORT is set, but Cloud Run publishes exactly one container port; \
                 /metrics is served on PORT alongside the executor routes — unset METRICS_PORT",
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_open_ended_endpoint_override_families() {
        assert!(is_forbidden_endpoint_override(
            "CLOUDSDK_API_ENDPOINT_OVERRIDES_STORAGE"
        ));
        assert!(is_forbidden_endpoint_override(
            "GOOGLE_STORAGE_CUSTOM_ENDPOINT"
        ));
        assert!(!is_forbidden_endpoint_override("GOOGLE_PROJECT_ID"));
        assert!(!is_forbidden_endpoint_override("CUSTOM_ENDPOINT"));
    }

    #[test]
    fn execution_timeout_must_fire_before_the_cloud_run_ceiling() {
        assert!(validate_execution_timeout(1).is_ok());
        assert!(validate_execution_timeout(MAX_EXECUTION_TIMEOUT_SECS).is_ok());
        assert!(validate_execution_timeout(0).is_err());
        assert!(validate_execution_timeout(MAX_EXECUTION_TIMEOUT_SECS + 1).is_err());
        assert!(validate_execution_timeout(CLOUD_RUN_MAX_REQUEST_SECS).is_err());
    }

    #[test]
    fn executor_tuning_parsers_reject_what_from_env_would_silently_default() {
        assert!(parses_u64("480"));
        assert!(!parses_u64("48O"));
        assert!(!parses_u64(" 480"));
        assert!(!parses_u64("-1"));
        assert!(parses_u32("3"));
        assert!(!parses_u32("4294967296"));
        assert!(parses_usize("100"));
    }
}
