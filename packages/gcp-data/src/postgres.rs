//! Passwordless Cloud SQL for PostgreSQL connectivity.
//!
//! SQLx 0.9 stores one password in `PgConnectOptions`; it does not expose an
//! asynchronous credential callback when the pool opens a new connection.
//! A Cloud SQL IAM access token therefore cannot be refreshed safely inside an
//! existing SeaORM pool. This module obtains a metadata-server token at startup
//! and exposes a fail-closed lifecycle: the process drains and terminates before
//! that token expires, allowing Cloud Run to restart it with a new token. Static
//! PostgreSQL passwords and connection strings are rejected — under Cloud SQL
//! IAM database authentication no password exists to leak. Shared by the GCP API
//! and the GCP queue workers.

use crate::metadata::{
    FORBIDDEN_CREDENTIAL_SETTINGS, SQL_LOGIN_SCOPE, TokenSource, unix_timestamp,
};
use sea_orm::DatabaseConnection;
use sea_orm::sqlx::{
    ConnectOptions as _,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{env, fmt::Debug, net::Ipv4Addr, time::Duration};

/// The DNS suffix Cloud SQL gives an instance. A private-IP deployment normally
/// dials the PSA address directly, so both shapes are accepted and nothing else
/// is — a public hostname here would mean the connection had left the VPC.
const CLOUD_SQL_DNS_SUFFIX: &str = ".cloudsql.goog";
/// Cloud SQL's IAM database user is the service-account email with this suffix
/// removed. Leaving it on produces a plain "password authentication failed",
/// which is indistinguishable from a broken token, so it is checked by name.
const SERVICE_ACCOUNT_SUFFIX: &str = ".gserviceaccount.com";
const IAM_USER_SUFFIX: &str = ".iam";

// The process starts draining five to eight minutes before token expiry. After
// a readiness propagation window, Axum gets another minute to finish in-flight
// requests. The hard stop still occurs more than three minutes before expiry.
const ROTATION_MARGIN_SECONDS: i64 = 5 * 60;
const MAX_ROTATION_JITTER_SECONDS: i64 = 3 * 60;
pub const READINESS_DRAIN_SECONDS: u64 = 45;
pub const GRACEFUL_SHUTDOWN_SECONDS: u64 = 60;
const MINIMUM_TOKEN_LIFETIME_SECONDS: i64 = 10 * 60;

const MAX_SERVER_CA_BYTES: usize = 64 * 1024;
const PEM_CERTIFICATE_HEADER: &str = "-----BEGIN CERTIFICATE-----";
const PEM_CERTIFICATE_FOOTER: &str = "-----END CERTIFICATE-----";

const REQUIRED_SETTINGS: &[&str] = &[
    "GCP_POSTGRES_AUTH_MODE",
    "GCP_POSTGRES_HOST",
    "GCP_POSTGRES_DATABASE",
    "GCP_POSTGRES_USER",
    "GCP_POSTGRES_SERVER_CA",
];

/// Everything libpq would otherwise read from the environment, plus every way to
/// reintroduce a static password or to interpose the Cloud SQL Auth Proxy. The
/// proxy is not merely redundant here — it terminates TLS on localhost, which
/// would turn `verify-full` into a check against a certificate the proxy chose.
const FORBIDDEN_SETTINGS: &[&str] = &[
    "DATABASE_URL",
    "GCP_POSTGRES_PASSWORD",
    "GCP_POSTGRES_CONNECTION_STRING",
    "POSTGRES_PASSWORD",
    "PGPASSWORD",
    "PGPASSFILE",
    "PGSERVICE",
    "PGSERVICEFILE",
    "PGHOST",
    "PGHOSTADDR",
    "PGPORT",
    "PGUSER",
    "PGDATABASE",
    "PGSSLMODE",
    "PGSSLROOTCERT",
    "PGSSLCERT",
    "PGSSLKEY",
    "PGOPTIONS",
    "PGAPPNAME",
    "INSTANCE_CONNECTION_NAME",
    "CLOUD_SQL_CONNECTION_NAME",
    "CLOUD_SQL_PROXY_PATH",
    "CSQL_PROXY_ADDRESS",
    "CSQL_PROXY_PORT",
    "CSQL_PROXY_UNIX_SOCKET",
    "CSQL_PROXY_TOKEN",
    "CSQL_PROXY_CREDENTIALS_FILE",
    "CSQL_PROXY_JSON_CREDENTIALS",
    "CSQL_PROXY_AUTO_IAM_AUTHN",
    "CLOUDSDK_API_ENDPOINT_OVERRIDES_SQLADMIN",
];

#[derive(Clone, Eq, PartialEq)]
pub struct IamPostgresConfig {
    pub host: String,
    pub database: String,
    pub user: String,
    /// The instance's server CA in PEM form. Not a secret — a CA certificate is
    /// public by construction — but it is bulky, so it stays out of `Debug`.
    pub server_ca_pem: String,
}

impl Debug for IamPostgresConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IamPostgresConfig")
            .field("host", &self.host)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("server_ca_pem_bytes", &self.server_ca_pem.len())
            .finish()
    }
}

impl IamPostgresConfig {
    pub fn from_env() -> Result<Self, PostgresAuthError> {
        Self::parse(|name| match env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(PostgresAuthError::InvalidSetting {
                name: name.to_string(),
                reason: "must contain valid UTF-8".to_string(),
            }),
        })
    }

    fn parse<F>(lookup: F) -> Result<Self, PostgresAuthError>
    where
        F: Fn(&str) -> Result<Option<String>, PostgresAuthError>,
    {
        for &name in FORBIDDEN_SETTINGS
            .iter()
            .chain(FORBIDDEN_CREDENTIAL_SETTINGS.iter())
        {
            if lookup(name)?.is_some() {
                return Err(PostgresAuthError::ForbiddenSetting(name));
            }
        }

        let required = |name: &'static str| -> Result<String, PostgresAuthError> {
            let value = lookup(name)?.ok_or(PostgresAuthError::MissingSetting(name))?;
            if value.is_empty() || value.trim() != value {
                return Err(PostgresAuthError::InvalidSetting {
                    name: name.to_string(),
                    reason: "must be non-empty and have no surrounding whitespace".to_string(),
                });
            }
            Ok(value)
        };

        let auth_mode = required(REQUIRED_SETTINGS[0])?;
        if auth_mode != "iam" {
            return Err(PostgresAuthError::InvalidSetting {
                name: REQUIRED_SETTINGS[0].to_string(),
                reason: "must be exactly 'iam'".to_string(),
            });
        }

        let host = required(REQUIRED_SETTINGS[1])?;
        validate_cloud_sql_host(&host)?;

        let database = required(REQUIRED_SETTINGS[2])?;
        validate_postgres_name(REQUIRED_SETTINGS[2], &database)?;

        let user = required(REQUIRED_SETTINGS[3])?;
        validate_iam_database_user(&user)?;

        // The PEM is the one setting that legitimately contains newlines, so it
        // bypasses the whitespace-trimming `required` helper.
        let server_ca_pem = lookup(REQUIRED_SETTINGS[4])?
            .ok_or(PostgresAuthError::MissingSetting(REQUIRED_SETTINGS[4]))?;
        let server_ca_pem = normalize_pem(&server_ca_pem);
        validate_server_ca(&server_ca_pem)?;

        Ok(Self {
            host,
            database,
            user,
            server_ca_pem,
        })
    }
}

/// Terraform and Secret Manager both round-trip PEM material through
/// single-line values with escaped newlines. Accepting that form avoids an
/// operator having to discover that a valid certificate was rejected purely by
/// how it travelled.
fn normalize_pem(value: &str) -> String {
    let normalized = if value.contains('\n') {
        value.to_string()
    } else {
        value.replace("\\n", "\n")
    };
    normalized.trim().to_string()
}

fn validate_server_ca(pem: &str) -> Result<(), PostgresAuthError> {
    // Cloud SQL signs its serving certificate with a per-instance CA that is not
    // in any public trust store, so `verify-full` against WebPKI roots can never
    // succeed. Pinning the instance CA is what makes verify-full mean anything
    // here; without it the only options are a weaker SSL mode or a proxy, and
    // both were rejected above.
    if pem.len() > MAX_SERVER_CA_BYTES {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[4].to_string(),
            reason: format!("must be at most {MAX_SERVER_CA_BYTES} bytes"),
        });
    }
    if !pem.starts_with(PEM_CERTIFICATE_HEADER) || !pem.ends_with(PEM_CERTIFICATE_FOOTER) {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[4].to_string(),
            reason: "must be the instance server CA as a PEM certificate".to_string(),
        });
    }
    if pem.contains("PRIVATE KEY") {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[4].to_string(),
            reason: "must contain only certificates; a private key was supplied".to_string(),
        });
    }
    Ok(())
}

/// Accept only an address that stays inside the VPC: a PSA private IP or the
/// instance's Cloud SQL DNS name.
fn validate_cloud_sql_host(host: &str) -> Result<(), PostgresAuthError> {
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        if address.is_private() {
            return Ok(());
        }
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[1].to_string(),
            reason: "must be a private address; Cloud SQL is reached over Private Service Access"
                .to_string(),
        });
    }

    let Some(private_labels) = host.strip_suffix(CLOUD_SQL_DNS_SUFFIX) else {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[1].to_string(),
            reason: format!("must be an RFC1918 address or end in {CLOUD_SQL_DNS_SUFFIX}"),
        });
    };

    let valid = host.len() <= 253
        && !private_labels.is_empty()
        && private_labels.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });

    if !valid {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[1].to_string(),
            reason: "must be a lowercase Cloud SQL DNS name without a scheme, port, or path"
                .to_string(),
        });
    }

    Ok(())
}

/// Validate the shape of the IAM database user.
///
/// The value is read from the environment rather than derived from the runtime
/// service account: Terraform owns the `google_sql_user` resource, and deriving
/// the name here would let the two drift apart silently. Validating the shape
/// keeps the drift loud.
fn validate_iam_database_user(user: &str) -> Result<(), PostgresAuthError> {
    if user.ends_with(SERVICE_ACCOUNT_SUFFIX) {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[3].to_string(),
            reason: format!(
                "must have the {SERVICE_ACCOUNT_SUFFIX} suffix stripped; Cloud SQL names the IAM \
                 user after the truncated service-account email"
            ),
        });
    }

    let Some((account, project)) = user.split_once('@') else {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[3].to_string(),
            reason: "must be a service-account email of the form <account>@<project>.iam"
                .to_string(),
        });
    };

    // PostgreSQL truncates identifiers at 63 bytes, and so does Cloud SQL when
    // it creates the IAM user. A longer value would connect as a different
    // principal than the one the grants were written for.
    let valid = user.len() <= 63
        && !project.contains('@')
        && project.ends_with(IAM_USER_SUFFIX)
        && (1..=30).contains(&account.len())
        && account.starts_with(|character: char| character.is_ascii_lowercase())
        && account
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && project.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b':')
        });

    if !valid {
        return Err(PostgresAuthError::InvalidSetting {
            name: REQUIRED_SETTINGS[3].to_string(),
            reason: "must be at most 63 bytes and shaped like <account>@<project>.iam".to_string(),
        });
    }

    Ok(())
}

fn validate_postgres_name(name: &'static str, value: &str) -> Result<(), PostgresAuthError> {
    if value.len() > 63
        || value
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return Err(PostgresAuthError::InvalidSetting {
            name: name.to_string(),
            reason: "must be at most 63 UTF-8 bytes and contain no control characters".to_string(),
        });
    }

    Ok(())
}

#[derive(Clone, Debug)]
pub struct DatabaseTokenLifecycle {
    expires_at_unix: i64,
    started_at: tokio::time::Instant,
    rotate_after: Duration,
    hard_stop_after: Duration,
}

impl DatabaseTokenLifecycle {
    fn new(expires_at_unix: i64) -> Result<Self, PostgresAuthError> {
        // Replicas normally start together and can receive tokens with the same
        // expiry. Per-process jitter keeps every production replica from
        // entering its drain window at once.
        let rotation_jitter_seconds =
            (uuid::Uuid::new_v4().as_u128() % MAX_ROTATION_JITTER_SECONDS as u128) as i64;
        let schedule =
            lifecycle_schedule(unix_timestamp(), expires_at_unix, rotation_jitter_seconds)?;
        tracing::info!(
            rotate_after_seconds = schedule.rotate_after_seconds,
            hard_stop_after_seconds = schedule.hard_stop_after_seconds,
            rotation_jitter_seconds,
            "configured fail-closed database token lifecycle"
        );
        // Convert the absolute token expiry into monotonic deadlines once. A
        // later wall-clock correction cannot extend the serving window.
        Ok(Self {
            expires_at_unix,
            started_at: tokio::time::Instant::now(),
            rotate_after: Duration::from_secs(schedule.rotate_after_seconds),
            hard_stop_after: Duration::from_secs(schedule.hard_stop_after_seconds),
        })
    }

    pub async fn wait_until_drain(&self) {
        tokio::time::sleep_until(self.started_at + self.rotate_after).await;
    }

    pub async fn wait_until_hard_stop(&self) {
        tokio::time::sleep_until(self.started_at + self.hard_stop_after).await;
    }

    pub fn remaining_seconds(&self) -> i64 {
        self.expires_at_unix.saturating_sub(unix_timestamp())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LifecycleSchedule {
    rotate_after_seconds: u64,
    hard_stop_after_seconds: u64,
}

fn lifecycle_schedule(
    now_unix: i64,
    expires_at_unix: i64,
    rotation_jitter_seconds: i64,
) -> Result<LifecycleSchedule, PostgresAuthError> {
    let lifetime = expires_at_unix.saturating_sub(now_unix);
    if lifetime < MINIMUM_TOKEN_LIFETIME_SECONDS {
        return Err(PostgresAuthError::TokenLifetimeTooShort(lifetime.max(0)));
    }

    let rotate_after = lifetime - ROTATION_MARGIN_SECONDS - rotation_jitter_seconds;
    Ok(LifecycleSchedule {
        rotate_after_seconds: rotate_after as u64,
        hard_stop_after_seconds: (rotate_after
            + READINESS_DRAIN_SECONDS as i64
            + GRACEFUL_SHUTDOWN_SECONDS as i64) as u64,
    })
}

pub struct IamDatabase {
    pub connection: DatabaseConnection,
    pub lifecycle: DatabaseTokenLifecycle,
}

pub async fn connect(config: &IamPostgresConfig) -> Result<IamDatabase, PostgresAuthError> {
    connect_as(config, "flow-like-gcp-api").await
}

/// Connect with an explicit `application_name`, so each GCP process is
/// distinguishable in `pg_stat_activity` and the server logs.
pub async fn connect_as(
    config: &IamPostgresConfig,
    application_name: &str,
) -> Result<IamDatabase, PostgresAuthError> {
    // The login scope is the narrowest one that satisfies Cloud SQL. This token
    // becomes a PostgreSQL password, so it is copied into connection state and
    // may surface in a driver trace; a cloud-platform token in that position
    // would be a project-wide credential sitting in a database handle.
    let tokens = TokenSource::metadata(&[SQL_LOGIN_SCOPE])
        .map_err(|error| PostgresAuthError::Credential(error.to_string()))?;
    let access_token = tokens
        .token()
        .await
        .map_err(|error| PostgresAuthError::Credential(error.to_string()))?;
    let lifecycle = DatabaseTokenLifecycle::new(access_token.expires_at_unix())?;

    // Environment-derived libpq settings are rejected above. Every security-
    // relevant option is then set explicitly, including hostname validation and
    // chain validation against the pinned Cloud SQL instance CA.
    let pg_options = PgConnectOptions::new_without_pgpass()
        .host(&config.host)
        .port(5432)
        .username(&config.user)
        .database(&config.database)
        .password(access_token.secret())
        .ssl_mode(PgSslMode::VerifyFull)
        .ssl_root_cert_from_pem(config.server_ca_pem.as_bytes().to_vec())
        .application_name(application_name)
        .disable_statement_logging();

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(5 * 60))
        .max_lifetime(Duration::from_secs(30 * 60))
        .test_before_acquire(true)
        .connect_with(pg_options)
        .await
        .map_err(|error| PostgresAuthError::Connection(error.to_string()))?;

    let connection = DatabaseConnection::from(pool);
    connection
        .ping()
        .await
        .map_err(|error| PostgresAuthError::Connection(error.to_string()))?;

    Ok(IamDatabase {
        connection,
        lifecycle,
    })
}

#[derive(Debug)]
pub enum PostgresAuthError {
    MissingSetting(&'static str),
    ForbiddenSetting(&'static str),
    InvalidSetting { name: String, reason: String },
    TokenLifetimeTooShort(i64),
    Credential(String),
    Connection(String),
}

impl std::fmt::Display for PostgresAuthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSetting(name) => {
                write!(formatter, "missing required environment variable {name}")
            }
            Self::ForbiddenSetting(name) => write!(
                formatter,
                "{name} is forbidden: Cloud SQL must use IAM database authentication, where no \
                 password exists to configure"
            ),
            Self::InvalidSetting { name, reason } => {
                write!(formatter, "invalid {name}: {reason}")
            }
            Self::TokenLifetimeTooShort(seconds) => write!(
                formatter,
                "Cloud SQL IAM database token lifetime is only {seconds}s; at least \
                 {MINIMUM_TOKEN_LIFETIME_SECONDS}s is required for safe draining. The metadata \
                 server serves a cached token until five minutes before it expires, so this is \
                 transient: failing closed lets the platform restart the process onto a fresh one"
            ),
            Self::Credential(error) => {
                write!(formatter, "Cloud SQL IAM token acquisition failed: {error}")
            }
            Self::Connection(error) => {
                write!(formatter, "Cloud SQL PostgreSQL connection failed: {error}")
            }
        }
    }
}

impl std::error::Error for PostgresAuthError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SERVER_CA: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";

    fn valid_settings() -> HashMap<&'static str, String> {
        HashMap::from([
            ("GCP_POSTGRES_AUTH_MODE", "iam".to_string()),
            ("GCP_POSTGRES_HOST", "10.24.0.3".to_string()),
            ("GCP_POSTGRES_DATABASE", "flow_like".to_string()),
            (
                "GCP_POSTGRES_USER",
                "flowlike-dev-api@flow-like-dev.iam".to_string(),
            ),
            ("GCP_POSTGRES_SERVER_CA", SERVER_CA.to_string()),
        ])
    }

    fn parse(
        values: &HashMap<&'static str, String>,
    ) -> Result<IamPostgresConfig, PostgresAuthError> {
        IamPostgresConfig::parse(|name| Ok(values.get(name).cloned()))
    }

    #[test]
    fn accepts_only_iam_configuration() {
        let config = parse(&valid_settings()).expect("valid settings must parse");
        assert_eq!(config.database, "flow_like");
        assert_eq!(config.user, "flowlike-dev-api@flow-like-dev.iam");
        assert!(config.server_ca_pem.starts_with(PEM_CERTIFICATE_HEADER));
    }

    #[test]
    fn rejects_passwords_and_connection_strings_even_when_empty() {
        for forbidden in ["DATABASE_URL", "PGPASSWORD", "GCP_POSTGRES_PASSWORD"] {
            let mut values = valid_settings();
            values.insert(forbidden, String::new());
            assert!(matches!(
                parse(&values),
                Err(PostgresAuthError::ForbiddenSetting(name)) if name == forbidden
            ));
        }
    }

    #[test]
    fn rejects_the_cloud_sql_proxy_and_alternate_credential_sources() {
        for name in [
            "CSQL_PROXY_ADDRESS",
            "INSTANCE_CONNECTION_NAME",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GCE_METADATA_HOST",
            "HTTPS_PROXY",
        ] {
            let mut values = valid_settings();
            values.insert(name, "anything".to_string());
            assert!(matches!(
                parse(&values),
                Err(PostgresAuthError::ForbiddenSetting(forbidden)) if forbidden == name
            ));
        }
    }

    #[test]
    fn rejects_hosts_that_would_leave_the_vpc() {
        for host in [
            "34.76.10.4",
            "database.example.com",
            "10.24.0.3:5432",
            "HTTPS://10.24.0.3",
            "-instance.cloudsql.goog",
        ] {
            let mut values = valid_settings();
            values.insert("GCP_POSTGRES_HOST", host.to_string());
            assert!(parse(&values).is_err(), "host should be rejected: {host}");
        }

        let mut values = valid_settings();
        values.insert(
            "GCP_POSTGRES_HOST",
            "flow-like-dev.europe-west1.flow-like-dev.cloudsql.goog".to_string(),
        );
        assert!(parse(&values).is_ok());
    }

    #[test]
    fn rejects_a_user_that_still_carries_the_service_account_suffix() {
        let mut values = valid_settings();
        values.insert(
            "GCP_POSTGRES_USER",
            "flowlike-dev-api@flow-like-dev.iam.gserviceaccount.com".to_string(),
        );
        assert!(matches!(
            parse(&values),
            Err(PostgresAuthError::InvalidSetting { ref name, .. })
                if name == "GCP_POSTGRES_USER"
        ));
    }

    #[test]
    fn rejects_user_shapes_that_are_not_iam_principals() {
        for user in [
            "flowlike",
            "flowlike-dev-api@flow-like-dev",
            "Flowlike@flow-like-dev.iam",
            "a-very-long-service-account-name-that-postgres-would-truncate@flow-like-dev.iam",
        ] {
            let mut values = valid_settings();
            values.insert("GCP_POSTGRES_USER", user.to_string());
            assert!(parse(&values).is_err(), "user should be rejected: {user}");
        }
    }

    #[test]
    fn rejects_static_auth_mode() {
        let mut values = valid_settings();
        values.insert("GCP_POSTGRES_AUTH_MODE", "password".to_string());
        assert!(parse(&values).is_err());
    }

    #[test]
    fn server_ca_accepts_escaped_newlines_and_rejects_key_material() {
        let mut values = valid_settings();
        values.insert(
            "GCP_POSTGRES_SERVER_CA",
            "-----BEGIN CERTIFICATE-----\\nMIIB\\n-----END CERTIFICATE-----".to_string(),
        );
        let config = parse(&values).expect("escaped PEM must be accepted");
        assert!(config.server_ca_pem.contains('\n'));
        assert!(!config.server_ca_pem.contains("\\n"));

        let mut values = valid_settings();
        values.insert(
            "GCP_POSTGRES_SERVER_CA",
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n-----BEGIN PRIVATE KEY-----\nx\n-----END CERTIFICATE-----".to_string(),
        );
        assert!(parse(&values).is_err());

        let mut values = valid_settings();
        values.insert("GCP_POSTGRES_SERVER_CA", "not a certificate".to_string());
        assert!(parse(&values).is_err());
    }

    #[test]
    fn missing_server_ca_is_fatal_because_verify_full_cannot_work_without_it() {
        let mut values = valid_settings();
        values.remove("GCP_POSTGRES_SERVER_CA");
        assert!(matches!(
            parse(&values),
            Err(PostgresAuthError::MissingSetting("GCP_POSTGRES_SERVER_CA"))
        ));
    }

    #[test]
    fn lifecycle_stops_well_before_expiry() {
        let schedule = lifecycle_schedule(1_000, 4_600, 0).expect("one-hour token is valid");
        assert_eq!(schedule.rotate_after_seconds, 3_300);
        assert_eq!(schedule.hard_stop_after_seconds, 3_405);
        assert!(schedule.hard_stop_after_seconds < 3_600);
    }

    #[test]
    fn rejects_tokens_too_short_to_drain_safely() {
        assert!(matches!(
            lifecycle_schedule(1_000, 1_599, 0),
            Err(PostgresAuthError::TokenLifetimeTooShort(599))
        ));
    }

    #[test]
    fn jitter_staggers_replica_rotation_without_approaching_expiry() {
        let schedule = lifecycle_schedule(1_000, 4_600, 179).expect("one-hour token is valid");
        assert_eq!(schedule.rotate_after_seconds, 3_121);
        assert_eq!(schedule.hard_stop_after_seconds, 3_226);
        assert!(schedule.hard_stop_after_seconds < 3_600);
    }
}
