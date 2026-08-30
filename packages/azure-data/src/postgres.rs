//! Passwordless Azure PostgreSQL connectivity.
//!
//! SQLx 0.8 stores one password in `PgConnectOptions`; it does not expose an
//! asynchronous credential callback when the pool opens a new connection.
//! An Entra access token therefore cannot be refreshed safely inside an
//! existing SeaORM pool. This module obtains a managed-identity token at
//! startup and exposes a fail-closed lifecycle: the process drains and
//! terminates before that token expires, allowing Container Apps to restart it
//! with a new token. Static PostgreSQL passwords and connection strings are
//! rejected. Shared by the Azure API and the Azure queue workers.

use azure_core::credentials::TokenCredential;
use azure_identity::{ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId};
use sea_orm::DatabaseConnection;
use sea_orm::sqlx::{
    ConnectOptions as _,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{env, time::Duration};

const POSTGRES_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";
const AZURE_POSTGRES_SUFFIX: &str = ".postgres.database.azure.com";

// The process starts draining five to eight minutes before token expiry. After
// a readiness propagation window, Axum gets another minute to finish in-flight
// requests. The hard stop still occurs more than three minutes before expiry.
const ROTATION_MARGIN_SECONDS: i64 = 5 * 60;
const MAX_ROTATION_JITTER_SECONDS: i64 = 3 * 60;
pub const READINESS_DRAIN_SECONDS: u64 = 45;
pub const GRACEFUL_SHUTDOWN_SECONDS: u64 = 60;
const MINIMUM_TOKEN_LIFETIME_SECONDS: i64 = 10 * 60;

const REQUIRED_SETTINGS: &[&str] = &[
    "AZURE_POSTGRES_AUTH_MODE",
    "AZURE_POSTGRES_HOST",
    "AZURE_POSTGRES_DATABASE",
    "AZURE_POSTGRES_USER",
    "IDENTITY_ENDPOINT",
    "IDENTITY_HEADER",
];

const FORBIDDEN_SETTINGS: &[&str] = &[
    "DATABASE_URL",
    "AZURE_POSTGRES_PASSWORD",
    "AZURE_POSTGRES_CONNECTION_STRING",
    "AZURE_POSTGRESQL_CONNECTIONSTRING",
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedIdentityPostgresConfig {
    pub host: String,
    pub database: String,
    pub user: String,
    pub client_id: String,
}

impl ManagedIdentityPostgresConfig {
    pub fn from_env(client_id: Option<&str>) -> Result<Self, PostgresAuthError> {
        Self::parse(client_id, |name| match env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => Err(PostgresAuthError::InvalidSetting {
                name: name.to_string(),
                reason: "must contain valid UTF-8".to_string(),
            }),
        })
    }

    fn parse<F>(client_id: Option<&str>, lookup: F) -> Result<Self, PostgresAuthError>
    where
        F: Fn(&str) -> Result<Option<String>, PostgresAuthError>,
    {
        for name in FORBIDDEN_SETTINGS {
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
        if auth_mode != "managed_identity" {
            return Err(PostgresAuthError::InvalidSetting {
                name: REQUIRED_SETTINGS[0].to_string(),
                reason: "must be exactly 'managed_identity'".to_string(),
            });
        }

        let host = required(REQUIRED_SETTINGS[1])?;
        validate_azure_postgres_host(&host)?;

        let database = required(REQUIRED_SETTINGS[2])?;
        validate_postgres_name(REQUIRED_SETTINGS[2], &database)?;

        let user = required(REQUIRED_SETTINGS[3])?;
        validate_postgres_name(REQUIRED_SETTINGS[3], &user)?;

        let identity_endpoint = required(REQUIRED_SETTINGS[4])?;
        validate_identity_endpoint(&identity_endpoint)?;
        let identity_header = required(REQUIRED_SETTINGS[5])?;
        validate_identity_header(&identity_header)?;

        let client_id = client_id.ok_or(PostgresAuthError::MissingSetting("AZURE_CLIENT_ID"))?;
        uuid::Uuid::parse_str(client_id).map_err(|_| PostgresAuthError::InvalidSetting {
            name: "AZURE_CLIENT_ID".to_string(),
            reason: "must be a UUID client ID for a user-assigned managed identity".to_string(),
        })?;

        Ok(Self {
            host,
            database,
            user,
            client_id: client_id.to_string(),
        })
    }
}

fn validate_identity_endpoint(value: &str) -> Result<(), PostgresAuthError> {
    let endpoint = url::Url::parse(value).map_err(|_| PostgresAuthError::InvalidSetting {
        name: "IDENTITY_ENDPOINT".to_string(),
        reason: "must be the local URL injected by Azure Container Apps".to_string(),
    })?;
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
        return Err(PostgresAuthError::InvalidSetting {
            name: "IDENTITY_ENDPOINT".to_string(),
            reason: "must be an HTTP loopback URL without credentials, query, or fragment"
                .to_string(),
        });
    }

    Ok(())
}

fn validate_identity_header(value: &str) -> Result<(), PostgresAuthError> {
    if value.len() < 16 || value.len() > 512 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(PostgresAuthError::InvalidSetting {
            name: "IDENTITY_HEADER".to_string(),
            reason: "must be the non-empty, platform-injected SSRF header".to_string(),
        });
    }
    Ok(())
}

fn validate_azure_postgres_host(host: &str) -> Result<(), PostgresAuthError> {
    let Some(private_labels) = host.strip_suffix(AZURE_POSTGRES_SUFFIX) else {
        return Err(PostgresAuthError::InvalidSetting {
            name: "AZURE_POSTGRES_HOST".to_string(),
            reason: format!("must end in {AZURE_POSTGRES_SUFFIX}"),
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
            name: "AZURE_POSTGRES_HOST".to_string(),
            reason: "must be a lowercase Azure PostgreSQL DNS name without a scheme, port, or path"
                .to_string(),
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
        // Convert the absolute Entra expiry into monotonic deadlines once. A
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

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

pub struct ManagedIdentityDatabase {
    pub connection: DatabaseConnection,
    pub lifecycle: DatabaseTokenLifecycle,
}

pub async fn connect(
    config: &ManagedIdentityPostgresConfig,
) -> Result<ManagedIdentityDatabase, PostgresAuthError> {
    connect_as(config, "flow-like-azure-api").await
}

/// Connect with an explicit `application_name`, so each Azure process is
/// distinguishable in `pg_stat_activity` and the server logs.
pub async fn connect_as(
    config: &ManagedIdentityPostgresConfig,
    application_name: &str,
) -> Result<ManagedIdentityDatabase, PostgresAuthError> {
    let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
        user_assigned_id: Some(UserAssignedId::ClientId(config.client_id.clone())),
        ..Default::default()
    }))
    .map_err(|error| PostgresAuthError::Credential(error.to_string()))?;

    let access_token = credential
        .get_token(&[POSTGRES_SCOPE], None)
        .await
        .map_err(|error| PostgresAuthError::Credential(error.to_string()))?;
    let lifecycle = DatabaseTokenLifecycle::new(access_token.expires_on.unix_timestamp())?;

    // Environment-derived libpq settings are rejected above. Every security-
    // relevant option is then set explicitly, including hostname and CA
    // validation through SQLx's bundled WebPKI roots.
    let pg_options = PgConnectOptions::new_without_pgpass()
        .host(&config.host)
        .port(5432)
        .username(&config.user)
        .database(&config.database)
        .password(access_token.token.secret())
        .ssl_mode(PgSslMode::VerifyFull)
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

    Ok(ManagedIdentityDatabase {
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
                "{name} is forbidden: Azure PostgreSQL must use managed identity without a static password"
            ),
            Self::InvalidSetting { name, reason } => {
                write!(formatter, "invalid {name}: {reason}")
            }
            Self::TokenLifetimeTooShort(seconds) => write!(
                formatter,
                "managed-identity database token lifetime is only {seconds}s; at least {MINIMUM_TOKEN_LIFETIME_SECONDS}s is required for safe draining"
            ),
            Self::Credential(error) => {
                write!(
                    formatter,
                    "managed-identity token acquisition failed: {error}"
                )
            }
            Self::Connection(error) => {
                write!(formatter, "Azure PostgreSQL connection failed: {error}")
            }
        }
    }
}

impl std::error::Error for PostgresAuthError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const CLIENT_ID: &str = "11111111-2222-4333-8444-555555555555";

    fn valid_settings() -> HashMap<&'static str, String> {
        HashMap::from([
            ("AZURE_POSTGRES_AUTH_MODE", "managed_identity".to_string()),
            (
                "AZURE_POSTGRES_HOST",
                "flow-like-dev.flow-like.postgres.database.azure.com".to_string(),
            ),
            ("AZURE_POSTGRES_DATABASE", "flow_like".to_string()),
            (
                "AZURE_POSTGRES_USER",
                "flowlike-dev-api-identity".to_string(),
            ),
            (
                "IDENTITY_ENDPOINT",
                "http://localhost:42356/msi/token".to_string(),
            ),
            (
                "IDENTITY_HEADER",
                "11111111-2222-4333-8444-555555555555".to_string(),
            ),
        ])
    }

    fn parse(
        values: &HashMap<&'static str, String>,
    ) -> Result<ManagedIdentityPostgresConfig, PostgresAuthError> {
        ManagedIdentityPostgresConfig::parse(Some(CLIENT_ID), |name| Ok(values.get(name).cloned()))
    }

    #[test]
    fn accepts_only_managed_identity_configuration() {
        let config = parse(&valid_settings()).expect("valid settings must parse");
        assert_eq!(config.client_id, CLIENT_ID);
        assert_eq!(config.database, "flow_like");
        assert_eq!(config.user, "flowlike-dev-api-identity");
    }

    #[test]
    fn rejects_passwords_and_connection_strings_even_when_empty() {
        for forbidden in ["DATABASE_URL", "PGPASSWORD", "AZURE_POSTGRES_PASSWORD"] {
            let mut values = valid_settings();
            values.insert(forbidden, String::new());
            assert!(matches!(
                parse(&values),
                Err(PostgresAuthError::ForbiddenSetting(name)) if name == forbidden
            ));
        }
    }

    #[test]
    fn rejects_non_azure_or_ambiguous_hosts() {
        for host in [
            "database.example.com",
            "HTTPS://server.postgres.database.azure.com",
            "server.postgres.database.azure.com:5432",
            "-server.postgres.database.azure.com",
        ] {
            let mut values = valid_settings();
            values.insert("AZURE_POSTGRES_HOST", host.to_string());
            assert!(parse(&values).is_err(), "host should be rejected: {host}");
        }
    }

    #[test]
    fn rejects_static_auth_mode_and_missing_client_id() {
        let mut values = valid_settings();
        values.insert("AZURE_POSTGRES_AUTH_MODE", "password".to_string());
        assert!(parse(&values).is_err());

        assert!(matches!(
            ManagedIdentityPostgresConfig::parse(None, |name| {
                Ok(valid_settings().get(name).cloned())
            }),
            Err(PostgresAuthError::MissingSetting("AZURE_CLIENT_ID"))
        ));
    }

    #[test]
    fn rejects_non_local_identity_endpoint_and_alternate_credential_sources() {
        let mut remote = valid_settings();
        remote.insert(
            "IDENTITY_ENDPOINT",
            "https://identity.example.com/token".to_string(),
        );
        assert!(parse(&remote).is_err());

        for (name, value) in [
            ("IMDS_ENDPOINT", "http://localhost:1234"),
            ("HTTPS_PROXY", "https://proxy.example.com"),
        ] {
            let mut override_settings = valid_settings();
            override_settings.insert(name, value.to_string());
            assert!(matches!(
                parse(&override_settings),
                Err(PostgresAuthError::ForbiddenSetting(forbidden)) if forbidden == name
            ));
        }
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
