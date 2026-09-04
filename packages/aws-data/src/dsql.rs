//! Aurora DSQL connectivity with IAM tokens.
//!
//! DSQL closes every connection after 60 minutes and a token is valid for
//! `DSQL_TOKEN_DURATION_SECS` (60 minutes by default). A token is only
//! checked when a connection is opened, so the pool's connect options are
//! swapped for freshly minted ones whenever the current token is older than
//! 80% of its lifetime, and the pool retires connections well before the
//! server would. Refresh happens on demand ([`DsqlDatabase::refresh_token_if_stale`])
//! rather than on a timer, because a frozen Lambda's timers do not tick and
//! its first connection after a long thaw would otherwise carry an expired
//! token.

use aurora_dsql_sqlx_connector::{DsqlConnectOptions, DsqlConnectOptionsBuilder, Region};
use sea_orm::DatabaseConnection;
use sea_orm::sqlx::{
    ConnectOptions as _,
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode},
};
use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// DSQL cluster endpoint, e.g. `abc0def1ghi2jkl3.dsql.eu-west-1.on.aws`.
/// Its presence selects DSQL for the process.
pub const ENDPOINT_ENV: &str = "DSQL_CLUSTER_ENDPOINT";
pub const REGION_ENV: &str = "DSQL_REGION";
pub const USER_ENV: &str = "DSQL_USER";
pub const TOKEN_DURATION_ENV: &str = "DSQL_TOKEN_DURATION_SECS";
pub const MAX_CONNECTIONS_ENV: &str = "DSQL_MAX_CONNECTIONS";

pub const DEFAULT_APPLICATION_NAME: &str = "flow-like-aws-api";

const DSQL_HOST_SUFFIX: &str = ".on.aws";
const DATABASE: &str = "postgres";
const DEFAULT_USER: &str = "admin";
/// Rotation happens at half-life, so the remaining half must outlast the
/// longest Lambda invocation (15 minutes) that could open a connection late.
const DEFAULT_TOKEN_DURATION_SECS: u64 = 3_600;
const MIN_TOKEN_DURATION_SECS: u64 = 1_800;
const MAX_TOKEN_DURATION_SECS: u64 = 604_800;
/// One Lambda instance serves one invocation at a time; a few extra
/// connections cover the sub-tasks a request spawns. ECS-style processes set
/// `DSQL_MAX_CONNECTIONS` explicitly.
const DEFAULT_MAX_CONNECTIONS: u32 = 4;
/// Below DSQL's 60-minute server-side connection limit.
const CONNECTION_MAX_LIFETIME: Duration = Duration::from_secs(25 * 60);
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(8);

/// Static database credentials and libpq's own connection sources are refused
/// alongside a DSQL endpoint, even when empty, because any of them could
/// redirect where the token goes or replace it with a password.
pub const FORBIDDEN_SETTINGS: &[&str] = &[
    "DATABASE_URL",
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
];

#[derive(Debug, Clone)]
pub struct DsqlConfig {
    pub host: String,
    pub region: Option<String>,
    pub user: String,
    pub token_duration_secs: u64,
    pub max_connections: u32,
}

impl DsqlConfig {
    /// Read the DSQL settings, or `None` when no endpoint is configured and the
    /// process should fall back to its other database configuration.
    pub fn from_env() -> Result<Option<Self>, DsqlConfigError> {
        let Some(host) = env::var(ENDPOINT_ENV).ok().map(|v| v.trim().to_owned()) else {
            return Ok(None);
        };
        if host.is_empty() {
            return Err(DsqlConfigError::Invalid {
                name: ENDPOINT_ENV,
                reason: "must not be empty".into(),
            });
        }
        validate_host(&host)?;
        for name in FORBIDDEN_SETTINGS {
            if env::var_os(name).is_some() {
                return Err(DsqlConfigError::Forbidden(name));
            }
        }
        let region = env::var(REGION_ENV)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        let user = env::var(USER_ENV)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_USER.to_owned());
        validate_postgres_name(USER_ENV, &user)?;
        let token_duration_secs = match env::var(TOKEN_DURATION_ENV) {
            Ok(raw) => parse_bounded(
                TOKEN_DURATION_ENV,
                &raw,
                MIN_TOKEN_DURATION_SECS,
                MAX_TOKEN_DURATION_SECS,
            )?,
            Err(_) => DEFAULT_TOKEN_DURATION_SECS,
        };
        let max_connections = match env::var(MAX_CONNECTIONS_ENV) {
            Ok(raw) => parse_bounded(MAX_CONNECTIONS_ENV, &raw, 1, 1_000)? as u32,
            Err(_) => DEFAULT_MAX_CONNECTIONS,
        };
        Ok(Some(Self {
            host,
            region,
            user,
            token_duration_secs,
            max_connections,
        }))
    }

    pub fn is_admin(&self) -> bool {
        self.user == DEFAULT_USER
    }

    /// Mint a token this long after the previous one; well inside the
    /// token's lifetime so a connection opened right at the boundary still
    /// authenticates.
    fn refresh_after(&self) -> Duration {
        Duration::from_secs((self.token_duration_secs / 2).max(1))
    }

    fn connect_options(&self, application_name: &str) -> Result<DsqlConnectOptions, DsqlError> {
        // Hostname and CA validation through SQLx's bundled WebPKI roots; DSQL
        // endpoints carry publicly trusted certificates. The connector prefixes
        // `application_name` with the value given as `orm_prefix`.
        let pg = PgConnectOptions::new_without_pgpass()
            .host(&self.host)
            .port(5432)
            .username(&self.user)
            .database(DATABASE)
            .ssl_mode(PgSslMode::VerifyFull)
            .disable_statement_logging();
        DsqlConnectOptionsBuilder::default()
            .pg_connect_options(pg)
            .region(self.region.clone().map(Region::new))
            .token_duration_secs(self.token_duration_secs)
            .orm_prefix(application_name.to_owned())
            .build()
            .map_err(|error| DsqlError::Config(error.to_string()))
    }
}

struct TokenRotor {
    options: DsqlConnectOptions,
    pool: PgPool,
    refresh_after: Duration,
    last_minted: Mutex<Instant>,
}

impl TokenRotor {
    /// Mint a fresh token (a local SigV4 presign) and hand it to the pool when
    /// the current one is older than `refresh_after`. Callers racing here
    /// serialize on the timestamp lock so one token is minted, not several.
    async fn refresh_if_stale(&self) -> Result<bool, DsqlError> {
        let mut last_minted = self.last_minted.lock().await;
        if last_minted.elapsed() < self.refresh_after {
            return Ok(false);
        }
        let options = self
            .options
            .authenticated_pg_options()
            .await
            .map_err(|error| DsqlError::Token(error.to_string()))?;
        self.pool.set_connect_options(options);
        *last_minted = Instant::now();
        tracing::debug!("rotated the Aurora DSQL connection token");
        Ok(true)
    }
}

/// A sea-orm connection to Aurora DSQL whose pool keeps a valid IAM token.
pub struct DsqlDatabase {
    pub connection: DatabaseConnection,
    rotor: Arc<TokenRotor>,
}

impl DsqlDatabase {
    /// Refresh the pool's token if it is near expiry; a cheap timestamp
    /// compare otherwise. Call once per Lambda invocation before any query.
    pub async fn refresh_token_if_stale(&self) -> Result<(), DsqlError> {
        self.rotor.refresh_if_stale().await.map(|_| ())
    }

    /// Keep the token fresh from a timer, for long-running processes whose
    /// clock never freezes. Stops when the pool is closed.
    pub fn spawn_background_refresh(&self) -> tokio::task::JoinHandle<()> {
        let rotor = self.rotor.clone();
        tokio::spawn(async move {
            let period = (rotor.refresh_after / 4).max(Duration::from_secs(1));
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if rotor.pool.is_closed() {
                    break;
                }
                if let Err(error) = rotor.refresh_if_stale().await {
                    tracing::warn!(%error, "background Aurora DSQL token refresh failed");
                }
            }
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.rotor.pool
    }
}

pub async fn connect(config: &DsqlConfig) -> Result<DsqlDatabase, DsqlError> {
    connect_as(config, DEFAULT_APPLICATION_NAME).await
}

/// Connect with an explicit `application_name`, so each AWS process is
/// distinguishable in the cluster's connection view.
///
/// The pool opens connections lazily and keeps none warm: every Lambda
/// instance reconnecting at once after a deploy would otherwise burst past
/// the cluster's connection rate. One ping verifies the token and network
/// before the process accepts work.
pub async fn connect_as(
    config: &DsqlConfig,
    application_name: &str,
) -> Result<DsqlDatabase, DsqlError> {
    let options = config.connect_options(application_name)?;
    let authenticated = options
        .authenticated_pg_options()
        .await
        .map_err(|error| DsqlError::Token(error.to_string()))?;
    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(0)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .idle_timeout(CONNECTION_IDLE_TIMEOUT)
        .max_lifetime(CONNECTION_MAX_LIFETIME)
        .test_before_acquire(true)
        .connect_lazy_with(authenticated);
    let rotor = Arc::new(TokenRotor {
        options,
        pool: pool.clone(),
        refresh_after: config.refresh_after(),
        last_minted: Mutex::new(Instant::now()),
    });
    let connection = DatabaseConnection::from(pool);
    connection
        .ping()
        .await
        .map_err(|error| DsqlError::Connection(error.to_string()))?;
    tracing::info!(
        host = %config.host,
        user = %config.user,
        admin = config.is_admin(),
        token_duration_secs = config.token_duration_secs,
        max_connections = config.max_connections,
        "connected to Aurora DSQL"
    );
    Ok(DsqlDatabase { connection, rotor })
}

fn validate_host(host: &str) -> Result<(), DsqlConfigError> {
    let valid = host.ends_with(DSQL_HOST_SUFFIX)
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    if !valid {
        return Err(DsqlConfigError::Invalid {
            name: ENDPOINT_ENV,
            reason: format!(
                "must be a lowercase DSQL endpoint ending in {DSQL_HOST_SUFFIX} without a scheme, port, or path"
            ),
        });
    }
    Ok(())
}

fn validate_postgres_name(name: &'static str, value: &str) -> Result<(), DsqlConfigError> {
    if value.len() > 63 || value.chars().any(char::is_control) {
        return Err(DsqlConfigError::Invalid {
            name,
            reason: "must be at most 63 bytes and contain no control characters".into(),
        });
    }
    Ok(())
}

fn parse_bounded(
    name: &'static str,
    raw: &str,
    min: u64,
    max: u64,
) -> Result<u64, DsqlConfigError> {
    let value = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| DsqlConfigError::Invalid {
            name,
            reason: "must be a positive integer".into(),
        })?;
    if value < min || value > max {
        return Err(DsqlConfigError::Invalid {
            name,
            reason: format!("must be between {min} and {max}"),
        });
    }
    Ok(value)
}

#[derive(Debug, thiserror::Error)]
pub enum DsqlConfigError {
    #[error("invalid {name}: {reason}")]
    Invalid { name: &'static str, reason: String },
    #[error("{0} must not be set alongside {ENDPOINT_ENV}")]
    Forbidden(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum DsqlError {
    #[error("invalid DSQL connection configuration: {0}")]
    Config(String),
    #[error("failed to mint an Aurora DSQL IAM token: {0}")]
    Token(String),
    #[error("failed to connect to Aurora DSQL: {0}")]
    Connection(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_cluster_endpoint_and_rejects_urls() {
        assert!(validate_host("abc0def1ghi2jkl3mno4pqr5stu6.dsql.eu-west-1.on.aws").is_ok());
        assert!(validate_host("postgres://abc.dsql.eu-west-1.on.aws").is_err());
        assert!(validate_host("abc.dsql.eu-west-1.on.aws:5432").is_err());
        assert!(validate_host("Abc.dsql.eu-west-1.on.aws").is_err());
        assert!(validate_host("example.com").is_err());
    }

    #[test]
    fn bounds_are_enforced() {
        assert_eq!(
            parse_bounded(TOKEN_DURATION_ENV, " 900 ", 60, 604_800).unwrap(),
            900
        );
        assert!(parse_bounded(TOKEN_DURATION_ENV, "30", 60, 604_800).is_err());
        assert!(parse_bounded(MAX_CONNECTIONS_ENV, "zero", 1, 1_000).is_err());
    }

    #[test]
    fn tokens_rotate_at_half_of_their_lifetime() {
        let config = DsqlConfig {
            host: "abc.dsql.eu-west-1.on.aws".into(),
            region: None,
            user: DEFAULT_USER.into(),
            token_duration_secs: 900,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        };
        assert_eq!(config.refresh_after(), Duration::from_secs(450));
        assert!(config.is_admin());
    }
}
