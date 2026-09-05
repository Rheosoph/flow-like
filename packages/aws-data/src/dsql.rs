//! Aurora DSQL connectivity with IAM tokens.
//!
//! DSQL closes every connection after 60 minutes and a token is valid for
//! `DSQL_TOKEN_DURATION_SECS` (60 minutes by default). A token is only
//! checked when a connection is opened, so the pool's connect options are
//! swapped for freshly minted ones before either the token or the IAM
//! credentials that signed it expire, and the pool retires connections before the
//! server would. Refresh happens on demand ([`DsqlDatabase::refresh_token_if_stale`])
//! rather than on a timer, because a frozen Lambda's timers do not tick and
//! its first connection after a long thaw would otherwise carry an expired
//! token.

use aurora_dsql_sqlx_connector::{DsqlConnectOptions, DsqlConnectOptionsBuilder, Region};
use aws_credential_types::{
    Credentials,
    provider::{ProvideCredentials, SharedCredentialsProvider},
};
use sea_orm::DatabaseConnection;
use sea_orm::sqlx::{
    ConnectOptions as _,
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode},
};
use std::{
    env,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;

/// Public or PrivateLink DSQL cluster endpoint, e.g.
/// `abc0def1ghi2jkl3.dsql-fnh4.eu-west-1.on.aws`.
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
/// Leave time for a full Lambda invocation to open a connection after its initial refresh.
const CREDENTIAL_EXPIRY_MARGIN: Duration = Duration::from_secs(15 * 60);
const BACKGROUND_REFRESH_PERIOD: Duration = Duration::from_secs(30);

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
        let endpoint_region = validate_host(&host)?;
        for name in FORBIDDEN_SETTINGS {
            if env::var_os(name).is_some() {
                return Err(DsqlConfigError::Forbidden(name));
            }
        }
        let region = env::var(REGION_ENV)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        validate_region(endpoint_region, region.as_deref())?;
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

    fn connect_options(
        &self,
        application_name: &str,
        credentials: Credentials,
    ) -> Result<DsqlConnectOptions, DsqlError> {
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
            // Use exactly the credentials whose expiry the rotor inspected. Resolving a
            // provider again inside the connector could sign with a different credential set.
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .build()
            .map_err(|error| DsqlError::Config(error.to_string()))
    }
}

struct TokenRotor {
    config: DsqlConfig,
    application_name: String,
    provider: SharedCredentialsProvider,
    pool: PgPool,
    minted: Mutex<MintedToken>,
}

struct MintedToken {
    credentials: Credentials,
    refresh_at: Instant,
}

fn same_credentials(a: &Credentials, b: &Credentials) -> bool {
    a.access_key_id() == b.access_key_id()
        && a.secret_access_key() == b.secret_access_key()
        && a.session_token() == b.session_token()
        && a.expiry() == b.expiry()
}

fn refresh_delay(
    config: &DsqlConfig,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<Duration, DsqlError> {
    match credentials.expiry() {
        Some(expiry) => {
            let remaining = expiry
                .duration_since(now)
                .ok()
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| DsqlError::Token("AWS signing credentials have expired".into()))?;
            Ok(config
                .refresh_after()
                .min(remaining.saturating_sub(CREDENTIAL_EXPIRY_MARGIN)))
        }
        // Lambda environment credentials carry a session token but the SDK environment
        // provider exposes no expiration. Resolve and mint again at every invocation.
        None if credentials.session_token().is_some() => Ok(Duration::ZERO),
        None => Ok(config.refresh_after()),
    }
}

impl TokenRotor {
    /// Resolve the current signing credentials and rotate before the refresh deadline
    /// or whenever their identity changes. The lock serializes pool option updates.
    async fn refresh_if_stale(&self) -> Result<bool, DsqlError> {
        let mut minted = self.minted.lock().await;
        let credentials = self
            .provider
            .provide_credentials()
            .await
            .map_err(|error| DsqlError::Token(error.to_string()))?;
        let delay = refresh_delay(&self.config, &credentials, SystemTime::now())?;
        if Instant::now() < minted.refresh_at && same_credentials(&credentials, &minted.credentials)
        {
            return Ok(false);
        }
        let options = self
            .config
            .connect_options(&self.application_name, credentials.clone())?
            .authenticated_pg_options()
            .await
            .map_err(|error| DsqlError::Token(error.to_string()))?;
        self.pool.set_connect_options(options);
        *minted = MintedToken {
            credentials,
            refresh_at: Instant::now() + delay,
        };
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
    /// Refresh after a signing-credential change or before either expiry.
    /// Call once per Lambda invocation before any query.
    pub async fn refresh_token_if_stale(&self) -> Result<(), DsqlError> {
        self.rotor.refresh_if_stale().await.map(|_| ())
    }

    /// Keep the token fresh from a timer, for long-running processes whose
    /// clock never freezes. Stops when the pool is closed.
    pub fn spawn_background_refresh(&self) -> tokio::task::JoinHandle<()> {
        let rotor = self.rotor.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(BACKGROUND_REFRESH_PERIOD);
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
    let sdk = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let provider = sdk
        .credentials_provider()
        .ok_or_else(|| DsqlError::Token("no AWS credentials provider configured".into()))?;
    let credentials = provider
        .provide_credentials()
        .await
        .map_err(|error| DsqlError::Token(error.to_string()))?;
    let delay = refresh_delay(config, &credentials, SystemTime::now())?;
    let options = config.connect_options(application_name, credentials.clone())?;
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
        config: config.clone(),
        application_name: application_name.to_owned(),
        provider,
        pool: pool.clone(),
        minted: Mutex::new(MintedToken {
            credentials,
            refresh_at: Instant::now() + delay,
        }),
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

/// Return the Region carried by a public or PrivateLink connection hostname.
fn validate_host(host: &str) -> Result<&str, DsqlConfigError> {
    let invalid = || DsqlConfigError::Invalid {
        name: ENDPOINT_ENV,
        reason: concat!(
            "must be a bare public <id>.dsql.<region>.on.aws or PrivateLink ",
            "<id>.dsql-<service-id>.<region>.on.aws endpoint ",
            "without a scheme, port, or path"
        )
        .into(),
    };
    let labels: Vec<_> = host.split('.').collect();
    if labels.len() != 5
        || !host.ends_with(DSQL_HOST_SUFFIX)
        || labels
            .iter()
            .any(|label| label.is_empty() || label.len() > 63)
    {
        return Err(invalid());
    }
    let alphanumeric = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    let service_valid = labels[1] == "dsql"
        || labels[1]
            .strip_prefix("dsql-")
            .is_some_and(|suffix| suffix.split('-').all(alphanumeric));
    let region_parts: Vec<_> = labels[2].split('-').collect();
    let region_valid = region_parts.len() >= 3
        && region_parts[0].len() == 2
        && region_parts[..region_parts.len() - 1]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase()))
        && region_parts
            .last()
            .is_some_and(|part| part.len() == 1 && part.bytes().all(|b| b.is_ascii_digit()));
    if !alphanumeric(labels[0]) || !service_valid || !region_valid {
        return Err(invalid());
    }
    Ok(labels[2])
}

fn validate_region(endpoint_region: &str, region: Option<&str>) -> Result<(), DsqlConfigError> {
    if region.is_some_and(|region| region != endpoint_region) {
        return Err(DsqlConfigError::Invalid {
            name: REGION_ENV,
            reason: format!("must match the endpoint's region {endpoint_region}"),
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
    fn accepts_public_and_private_endpoints_with_matching_regions() {
        for host in [
            "abc0def1ghi2jkl3mno4pqr5stu6.dsql.eu-west-1.on.aws",
            "abc0def1ghi2jkl3mno4pqr5stu6.dsql-fnh4.eu-west-1.on.aws",
        ] {
            let region = validate_host(host).unwrap();
            assert_eq!(region, "eu-west-1");
            assert!(validate_region(region, None).is_ok());
            assert!(validate_region(region, Some("eu-west-1")).is_ok());
            assert!(validate_region(region, Some("us-east-1")).is_err());
        }
    }

    #[test]
    fn rejects_malformed_private_endpoints_and_other_aws_services() {
        for host in [
            "abc.rds.eu-west-1.on.aws",
            "abc.dsql-.eu-west-1.on.aws",
            "abc.dsql--fnh4.eu-west-1.on.aws",
            "abc.dsql-fnh4-.eu-west-1.on.aws",
            "abc.dsql-fnh4.us-east-12.on.aws",
            "abc.dsql-fnh4.eu-west-1.on.aws:5432",
            "abc.dsql-fnh4.eu-west-1.on.aws/postgres",
        ] {
            assert!(validate_host(host).is_err(), "accepted {host}");
        }
        assert!(validate_host(&format!("abc.dsql-{}.eu-west-1.on.aws", "a".repeat(59))).is_err());
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

    fn config_for_refresh_tests() -> DsqlConfig {
        DsqlConfig {
            host: "abc.dsql.eu-west-1.on.aws".into(),
            region: None,
            user: DEFAULT_USER.into(),
            token_duration_secs: 3_600,
            max_connections: 4,
        }
    }

    #[test]
    fn signing_credential_expiry_bounds_the_refresh_deadline() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let credentials = Credentials::new(
            "test-key",
            "test-secret",
            Some("session".into()),
            Some(now + Duration::from_secs(20 * 60)),
            "test",
        );
        assert_eq!(
            refresh_delay(&config_for_refresh_tests(), &credentials, now).unwrap(),
            Duration::from_secs(5 * 60)
        );
        let near_expiry = now + Duration::from_secs(10 * 60);
        assert_eq!(
            refresh_delay(&config_for_refresh_tests(), &credentials, near_expiry).unwrap(),
            Duration::ZERO
        );
        assert!(
            refresh_delay(
                &config_for_refresh_tests(),
                &credentials,
                now + Duration::from_secs(20 * 60)
            )
            .is_err()
        );
    }

    #[test]
    fn opaque_session_credentials_refresh_each_invocation() {
        let credentials = Credentials::new(
            "test-key",
            "test-secret",
            Some("session".into()),
            None,
            "test",
        );
        assert_eq!(
            refresh_delay(&config_for_refresh_tests(), &credentials, SystemTime::now()).unwrap(),
            Duration::ZERO
        );
        let permanent = Credentials::new("test-key", "test-secret", None, None, "test");
        assert_eq!(
            refresh_delay(&config_for_refresh_tests(), &permanent, SystemTime::now()).unwrap(),
            Duration::from_secs(1_800)
        );
    }

    #[test]
    fn credential_rotation_invalidates_a_token_before_its_deadline() {
        let old = Credentials::new(
            "test-key",
            "test-secret",
            Some("old-session".into()),
            None,
            "test",
        );
        let replacement = Credentials::new(
            "test-key",
            "test-secret",
            Some("new-session".into()),
            None,
            "test",
        );
        assert!(same_credentials(&old, &old.clone()));
        assert!(!same_credentials(&old, &replacement));
    }
}
