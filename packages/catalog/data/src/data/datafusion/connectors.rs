//! Table providers for external databases, built from plain parameters so a flow node and a
//! headless caller open a connection the same way.
//!
//! Server-side (`ExecutionEnvironment::Server`) every connection is vetted first with the
//! egress policy the other outbound nodes use (`flow_like::flow::execution::egress`): hosts and
//! resolved addresses on the executor's own network plane (loopback, link-local, cloud
//! metadata and credential endpoints) are refused. Private ranges stay a deployment decision
//! (VPC / NetworkPolicy), as everywhere else. File engines read only files the caller staged,
//! and ODBC connection strings may not borrow the host's identity (DSNs, Kerberos or Windows
//! integrated authentication). Locally nothing is filtered: the host is the user's own machine.

use flow_like::flow::execution::{ExecutionEnvironment, egress};
use flow_like_storage::datafusion::catalog::TableProvider;
use flow_like_types::Result;
use flow_like_types::egress::is_public_ip;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(any(feature = "postgres", feature = "mysql", feature = "clickhouse"))]
use datafusion_table_providers::util::secrets::to_secret_map;
#[cfg(feature = "mysql")]
use datafusion_table_providers::{
    mysql::MySQLTableFactory, sql::db_connection_pool::mysqlpool::MySQLConnectionPool,
};
#[cfg(feature = "postgres")]
use datafusion_table_providers::{
    postgres::PostgresTableFactory, sql::db_connection_pool::postgrespool::PostgresConnectionPool,
};

pub type Provider = Arc<dyn TableProvider>;

/// Where a connection may go. `environment` decides whether the executor's own network plane
/// is off limits (server only); `public_only` additionally refuses private, shared and
/// reserved ranges, for connections a bot opens on behalf of a user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectPolicy {
    pub environment: ExecutionEnvironment,
    pub public_only: bool,
}

impl ConnectPolicy {
    pub fn new(environment: ExecutionEnvironment) -> Self {
        Self {
            environment,
            public_only: false,
        }
    }

    pub fn public_only(environment: ExecutionEnvironment) -> Self {
        Self {
            environment,
            public_only: true,
        }
    }

    fn guarded(self) -> bool {
        self.environment == ExecutionEnvironment::Server
    }
}

impl From<ExecutionEnvironment> for ConnectPolicy {
    fn from(environment: ExecutionEnvironment) -> Self {
        Self::new(environment)
    }
}

#[derive(Clone, Debug)]
pub struct PostgresSource {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub schema: String,
    pub table: String,
    /// `disable`, `prefer`, `require`, `verify-ca` or `verify-full`.
    pub ssl_mode: String,
    pub readonly: bool,
}

#[derive(Clone, Debug)]
pub struct MysqlSource {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub table: String,
    /// `disabled`, `preferred` or `required`.
    pub ssl_mode: String,
    pub readonly: bool,
}

#[derive(Clone, Debug)]
pub struct ClickhouseSource {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub table: String,
}

#[derive(Clone, Debug)]
pub struct FlightSqlSource {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// The SQL the remote runs; its result is the table.
    pub query: String,
    pub use_tls: bool,
}

/// The resolved inputs of an ODBC table.
#[derive(Clone, Debug)]
pub struct OdbcSource {
    pub connection_string: String,
    pub username: String,
    pub password: String,
    pub catalog: String,
    pub schema: String,
    pub source_table: String,
    pub table_name: String,
    pub max_text_size: i64,
}

#[derive(Clone, Debug)]
pub struct FileDatabaseSource {
    /// A database file, or `:memory:` for DuckDB.
    pub path: String,
    pub table: String,
    pub readonly: bool,
    /// Server-side, the directory the caller staged the file in; the path must lie inside it.
    pub staged_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum ConnectorSource {
    Postgres(PostgresSource),
    Mysql(MysqlSource),
    Clickhouse(ClickhouseSource),
    FlightSql(FlightSqlSource),
    Odbc(OdbcSource),
    Oracle(OracleSource),
    Sqlite(FileDatabaseSource),
    Duckdb(FileDatabaseSource),
}

/// A live provider for `source`, after the egress checks of `environment`.
pub async fn connect_table(
    source: &ConnectorSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    let policy = policy.into();
    match source {
        ConnectorSource::Postgres(source) => postgres_provider(source, policy).await,
        ConnectorSource::Mysql(source) => mysql_provider(source, policy).await,
        ConnectorSource::Clickhouse(source) => clickhouse_provider(source, policy).await,
        ConnectorSource::FlightSql(source) => flight_sql_provider(source, policy).await,
        ConnectorSource::Odbc(source) => odbc_provider(source, policy).await,
        ConnectorSource::Sqlite(source) => sqlite_provider(source, policy.environment).await,
        ConnectorSource::Oracle(source) => oracle_provider(source, policy).await,
        ConnectorSource::Duckdb(source) => duckdb_provider(source, policy.environment).await,
    }
}

/// Fails when the host is not a plain host name or IP literal: drivers splice it into a
/// connection string or URL, where spaces, `@`, `/` or `=` would add parameters of their own.
fn ensure_host_syntax(driver: &str, host: &str) -> Result<()> {
    let plain = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'));
    if plain {
        Ok(())
    } else {
        Err(flow_like_types::anyhow!(
            "{driver}: host '{host}' must be a host name or IP address (letters, digits, '-', '.', '_' or ':')"
        ))
    }
}

/// Vets a connection target. Server-side the host and every address it resolves to must be
/// off the executor's network plane; the check runs again per connection only for HTTP
/// clients, so a driver that re-resolves later is guarded by this pre-flight alone.
pub async fn ensure_connection_allowed(
    policy: impl Into<ConnectPolicy>,
    driver: &str,
    host: &str,
    port: u16,
) -> Result<()> {
    let policy = policy.into();
    ensure_host_syntax(driver, host)?;
    if !policy.guarded() {
        return Ok(());
    }
    let addresses = egress::resolve_socket_addrs(policy.environment, host, port).await?;
    if policy.public_only
        && let Some(private) = addresses.iter().find(|address| !is_public_ip(address.ip()))
    {
        return Err(flow_like_types::anyhow!(
            "{driver}: connection to '{host}' refused: it resolves to {}, which is not a public address",
            private.ip()
        ));
    }
    Ok(())
}

/// Server-side, a database file must be one the caller staged. `:memory:` has no file.
pub fn ensure_file_allowed(
    environment: ExecutionEnvironment,
    engine: &str,
    source: &FileDatabaseSource,
) -> Result<()> {
    if environment != ExecutionEnvironment::Server || source.path == ":memory:" {
        return Ok(());
    }
    let refused = || {
        flow_like_types::anyhow!(
            "{engine} file '{}' refused: server-side runs open only files staged for the run, never host paths",
            source.path
        )
    };
    let root = source
        .staged_root
        .as_ref()
        .and_then(|root| root.canonicalize().ok())
        .ok_or_else(refused)?;
    let path = PathBuf::from(&source.path)
        .canonicalize()
        .map_err(|_| refused())?;
    if path.starts_with(&root) {
        Ok(())
    } else {
        Err(refused())
    }
}

/// Key/value pairs of an ODBC connection string; braced values may hold `;`.
fn odbc_attributes(connection_string: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut rest = connection_string;
    while !rest.trim().is_empty() {
        let Some((key, after)) = rest.split_once('=') else {
            break;
        };
        let after = after.trim_start();
        let (value, remainder) = match after.strip_prefix('{') {
            Some(braced) => {
                let mut end = braced.len();
                let mut chars = braced.char_indices().peekable();
                while let Some((index, c)) = chars.next() {
                    if c == '}' {
                        if chars.peek().is_some_and(|&(_, next)| next == '}') {
                            chars.next();
                        } else {
                            end = index;
                            break;
                        }
                    }
                }
                let value = braced[..end].replace("}}", "}");
                let remainder = braced.get(end + 1..).unwrap_or("");
                (value, remainder.trim_start_matches([' ', ';']))
            }
            None => match after.split_once(';') {
                Some((value, remainder)) => (value.trim().to_string(), remainder),
                None => (after.trim().to_string(), ""),
            },
        };
        attributes.push((key.trim().to_ascii_lowercase().replace(' ', ""), value));
        rest = remainder;
    }
    attributes
}

/// The `(host, port)` an ODBC connection string points at, if it names one.
fn odbc_target(attributes: &[(String, String)]) -> Option<(String, Option<u16>)> {
    let value = [
        "server",
        "host",
        "hostname",
        "address",
        "addr",
        "datasource",
    ]
    .iter()
    .find_map(|key| {
        attributes
            .iter()
            .find(|(name, value)| name == key && !value.is_empty())
            .map(|(_, value)| value.as_str())
    })?;
    let value = value.strip_prefix("tcp:").unwrap_or(value);
    let port_of = |text: &str| text.trim().parse::<u16>().ok();
    if let Some((host, port)) = value.split_once(',') {
        return Some((host.to_string(), port_of(port)));
    }
    match value.matches(':').count() {
        1 => value
            .split_once(':')
            .map(|(host, port)| (host.to_string(), port_of(port))),
        _ => Some((value.to_string(), None)),
    }
}

/// Server-side ODBC: no DSNs or integrated authentication (they borrow the executor's own
/// identity), and the connection string must name the server so it can be vetted.
async fn ensure_odbc_allowed(policy: ConnectPolicy, connection_string: &str) -> Result<()> {
    if !policy.guarded() {
        return Ok(());
    }
    let attributes = odbc_attributes(connection_string);
    for (key, value) in &attributes {
        let value = value.to_ascii_lowercase();
        let ambient = matches!(key.as_str(), "dsn" | "filedsn" | "savefile")
            || (matches!(key.as_str(), "trusted_connection" | "integratedsecurity")
                && matches!(value.as_str(), "yes" | "true" | "sspi"))
            || (key == "authentication" && value.contains("integrated"));
        if ambient {
            return Err(flow_like_types::anyhow!(
                "ODBC: '{key}' borrows the executor's own identity and is not available server-side; give the server and credentials explicitly"
            ));
        }
    }
    let (host, port) = odbc_target(&attributes).ok_or_else(|| {
        flow_like_types::anyhow!(
            "ODBC: server-side runs need an explicit Server=, Host= or Address= in the connection string so the target can be checked"
        )
    })?;
    ensure_connection_allowed(policy, "ODBC", &host, port.unwrap_or(0)).await
}

fn not_enabled(driver: &str, feature: &str) -> flow_like_types::Error {
    flow_like_types::anyhow!(
        "{driver} support is not enabled. Rebuild with the '{feature}' feature flag."
    )
}

/// Product binaries compile rustls with both ring and aws-lc-rs, so rustls cannot pick a
/// process-wide provider itself, and drivers that build their TLS config internally
/// panic until one is installed. Mirrors the catalog-web TLS nodes.
#[cfg(any(feature = "postgres", feature = "mysql"))]
fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

pub async fn postgres_provider(
    source: &PostgresSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    ensure_connection_allowed(policy, "PostgreSQL", &source.host, source.port).await?;
    connect_postgres(source).await
}

#[cfg(feature = "postgres")]
async fn connect_postgres(source: &PostgresSource) -> Result<Provider> {
    use flow_like_storage::datafusion::common::TableReference;

    let params = std::collections::HashMap::from([
        ("host".to_string(), source.host.clone()),
        ("port".to_string(), source.port.to_string()),
        ("user".to_string(), source.username.clone()),
        ("pass".to_string(), source.password.clone()),
        ("db".to_string(), source.database.clone()),
        ("sslmode".to_string(), source.ssl_mode.clone()),
    ]);

    ensure_crypto_provider();
    let pool = PostgresConnectionPool::new(to_secret_map(params))
        .await
        .map_err(|e| flow_like_types::anyhow!("PostgreSQL connection failed: {}", e))?;

    let factory = PostgresTableFactory::new(Arc::new(pool));
    let table_ref = TableReference::full(
        source.database.clone(),
        source.schema.clone(),
        source.table.clone(),
    );
    let provider = if source.readonly {
        factory.table_provider(table_ref).await
    } else {
        factory.read_write_table_provider(table_ref).await
    }
    .map_err(|e| flow_like_types::anyhow!("Failed to create PostgreSQL table provider: {}", e))?;

    // The provider reads columns from the catalog, so a missing table comes back
    // as a table without columns instead of an error.
    if provider.schema().fields().is_empty() {
        return Err(flow_like_types::anyhow!(
            "PostgreSQL table '{}.{}' was not found in database '{}', or none of its columns has a supported type",
            source.schema,
            source.table,
            source.database
        ));
    }
    Ok(provider)
}

#[cfg(not(feature = "postgres"))]
async fn connect_postgres(_source: &PostgresSource) -> Result<Provider> {
    Err(not_enabled("PostgreSQL", "postgres"))
}

pub async fn mysql_provider(
    source: &MysqlSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    ensure_connection_allowed(policy, "MySQL", &source.host, source.port).await?;
    connect_mysql(source).await
}

#[cfg(feature = "mysql")]
async fn connect_mysql(source: &MysqlSource) -> Result<Provider> {
    use flow_like_storage::datafusion::common::TableReference;

    let params = std::collections::HashMap::from([
        ("host".to_string(), source.host.clone()),
        ("tcp_port".to_string(), source.port.to_string()),
        ("user".to_string(), source.username.clone()),
        ("pass".to_string(), source.password.clone()),
        ("db".to_string(), source.database.clone()),
        ("sslmode".to_string(), source.ssl_mode.clone()),
    ]);

    ensure_crypto_provider();
    let pool = MySQLConnectionPool::new(to_secret_map(params))
        .await
        .map_err(|e| flow_like_types::anyhow!("MySQL connection failed: {}", e))?;

    // The pool is already bound to the database, and the provider's writer resolves
    // the table by its full display name, so only a bare name reads and writes.
    let factory = MySQLTableFactory::new(Arc::new(pool));
    let table_ref = TableReference::bare(source.table.clone());
    if source.readonly {
        factory.table_provider(table_ref).await
    } else {
        factory.read_write_table_provider(table_ref).await
    }
    .map_err(|e| flow_like_types::anyhow!("Failed to create MySQL table provider: {}", e))
}

#[cfg(not(feature = "mysql"))]
async fn connect_mysql(_source: &MysqlSource) -> Result<Provider> {
    Err(not_enabled("MySQL", "mysql"))
}

pub async fn clickhouse_provider(
    source: &ClickhouseSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    ensure_connection_allowed(policy, "ClickHouse", &source.host, source.port).await?;
    connect_clickhouse(source).await
}

#[cfg(feature = "clickhouse")]
async fn connect_clickhouse(source: &ClickhouseSource) -> Result<Provider> {
    use datafusion_table_providers::clickhouse::ClickHouseTableFactory;
    use datafusion_table_providers::sql::db_connection_pool::clickhousepool::ClickHouseConnectionPool;

    let mut params = std::collections::HashMap::from([
        (
            "url".to_string(),
            format!("http://{}:{}", source.host, source.port),
        ),
        ("database".to_string(), source.database.clone()),
        ("user".to_string(), source.username.clone()),
        // ClickHouse otherwise ships String columns as Arrow Binary, so text surfaces
        // in rows as byte arrays.
        (
            "option_output_format_arrow_string_as_string".to_string(),
            "1".to_string(),
        ),
    ]);
    if !source.password.is_empty() {
        params.insert("password".to_string(), source.password.clone());
    }

    let pool = ClickHouseConnectionPool::new(to_secret_map(params))
        .await
        .map_err(|e| flow_like_types::anyhow!("ClickHouse connection failed: {}", e))?;
    let table_ref = flow_like_storage::datafusion::common::TableReference::partial(
        source.database.clone(),
        source.table.clone(),
    );
    ClickHouseTableFactory::new(Arc::new(pool))
        .table_provider(table_ref, None)
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to create ClickHouse table provider: {}", e))
}

#[cfg(not(feature = "clickhouse"))]
async fn connect_clickhouse(_source: &ClickhouseSource) -> Result<Provider> {
    Err(not_enabled("ClickHouse", "clickhouse"))
}

pub async fn flight_sql_provider(
    source: &FlightSqlSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    ensure_connection_allowed(policy, "Flight SQL", &source.host, source.port).await?;
    connect_flight_sql(source).await
}

/// The URL a Flight SQL source connects to.
pub fn flight_sql_endpoint(source: &FlightSqlSource) -> String {
    let scheme = if source.use_tls { "https" } else { "http" };
    format!("{scheme}://{}:{}", source.host, source.port)
}

#[cfg(feature = "flight")]
async fn connect_flight_sql(source: &FlightSqlSource) -> Result<Provider> {
    use datafusion_table_providers::flight::FlightTableFactory;
    use datafusion_table_providers::flight::sql::{FlightSqlDriver, PASSWORD, QUERY, USERNAME};

    let mut options = std::collections::HashMap::from([(QUERY.to_string(), source.query.clone())]);
    if !source.username.is_empty() {
        options.insert(USERNAME.to_string(), source.username.clone());
    }
    if !source.password.is_empty() {
        options.insert(PASSWORD.to_string(), source.password.clone());
    }
    let factory = FlightTableFactory::new(Arc::new(FlightSqlDriver::new()));
    let table = factory
        .open_table(&flight_sql_endpoint(source), options)
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to create FlightSQL table: {}", e))?;
    Ok(Arc::new(table))
}

#[cfg(not(feature = "flight"))]
async fn connect_flight_sql(_source: &FlightSqlSource) -> Result<Provider> {
    Err(not_enabled("FlightSQL", "flight"))
}

/// ODBC connection-string values containing `;`, braces or edge whitespace must be
/// wrapped in braces, with closing braces doubled.
pub fn odbc_attribute(key: &str, value: &str) -> String {
    if value.contains([';', '{', '}']) || value.trim() != value {
        format!("{key}={{{}}}", value.replace('}', "}}"))
    } else {
        format!("{key}={value}")
    }
}

impl OdbcSource {
    /// The connection string with credentials appended as `UID` and `PWD`.
    pub fn full_connection_string(&self) -> String {
        let mut full = self
            .connection_string
            .trim()
            .trim_end_matches(';')
            .to_string();
        for (key, value) in [("UID", &self.username), ("PWD", &self.password)] {
            if !value.is_empty() {
                full.push(';');
                full.push_str(&odbc_attribute(key, value));
            }
        }
        full
    }

    /// Built from parts rather than parsed, so source identifiers keep their case.
    pub fn table_reference(&self) -> flow_like_storage::datafusion::common::TableReference {
        use flow_like_storage::datafusion::common::TableReference;
        let table = self.source_table.clone();
        match (self.catalog.is_empty(), self.schema.is_empty()) {
            (true, true) => TableReference::bare(table),
            (true, false) => TableReference::partial(self.schema.clone(), table),
            (false, _) => TableReference::full(self.catalog.clone(), self.schema.clone(), table),
        }
    }
}

pub async fn odbc_provider(
    source: &OdbcSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    if source.max_text_size <= 0 {
        return Err(flow_like_types::anyhow!(
            "Max Text Size must be a positive number of bytes, got {}",
            source.max_text_size
        ));
    }
    ensure_odbc_allowed(policy.into(), &source.full_connection_string()).await?;
    connect_odbc(source).await
}

#[cfg(feature = "odbc")]
async fn connect_odbc(source: &OdbcSource) -> Result<Provider> {
    open_odbc(
        source.full_connection_string(),
        Some(source.max_text_size),
        source.table_reference(),
    )
    .await
}

#[cfg(feature = "odbc")]
async fn open_odbc(
    connection_string: String,
    max_text_size: Option<i64>,
    table_ref: flow_like_storage::datafusion::common::TableReference,
) -> Result<Provider> {
    use datafusion_table_providers::odbc::ODBCTableFactory;
    use datafusion_table_providers::sql::db_connection_pool::odbcpool::ODBCPool;
    use datafusion_table_providers::util::secrets::to_secret_map;

    let mut params =
        std::collections::HashMap::from([("connection_string".to_string(), connection_string)]);
    if let Some(size) = max_text_size {
        params.insert("max_text_size".to_string(), size.to_string());
    }
    let pool = ODBCPool::new(to_secret_map(params))
        .map_err(|e| flow_like_types::anyhow!("ODBC connection failed: {}", e))?;

    ODBCTableFactory::new(Arc::new(pool))
        .table_provider(table_ref.clone(), None)
        .await
        .map_err(|e| {
            flow_like_types::anyhow!("Failed to create ODBC table provider for '{table_ref}': {e}")
        })
}

#[cfg(not(feature = "odbc"))]
async fn connect_odbc(_source: &OdbcSource) -> Result<Provider> {
    Err(not_enabled("ODBC", "odbc"))
}

#[derive(Clone, Debug)]
pub struct OracleSource {
    pub host: String,
    pub port: u16,
    pub service_name: String,
    pub username: String,
    pub password: String,
    /// Defaults to the upper-cased user name.
    pub schema: String,
    pub source_table: String,
    pub odbc_driver: String,
}

impl OracleSource {
    pub fn effective_schema(&self) -> String {
        if self.schema.is_empty() {
            self.username.to_uppercase()
        } else {
            self.schema.to_uppercase()
        }
    }

    pub fn connection_string(&self) -> String {
        let target = format!("{}:{}/{}", self.host, self.port, self.service_name);
        [
            format!("Driver={{{}}}", self.odbc_driver),
            odbc_attribute("DBQ", &target),
            odbc_attribute("UID", &self.username),
            odbc_attribute("PWD", &self.password),
        ]
        .join(";")
    }
}

pub async fn oracle_provider(
    source: &OracleSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<Provider> {
    ensure_connection_allowed(policy, "Oracle", &source.host, source.port).await?;
    connect_oracle(source).await
}

#[cfg(feature = "odbc")]
async fn connect_oracle(source: &OracleSource) -> Result<Provider> {
    use flow_like_storage::datafusion::common::TableReference;

    let table_ref = TableReference::full(
        source.service_name.clone(),
        source.effective_schema(),
        source.source_table.clone(),
    );
    open_odbc(source.connection_string(), None, table_ref).await
}

#[cfg(not(feature = "odbc"))]
async fn connect_oracle(_source: &OracleSource) -> Result<Provider> {
    Err(not_enabled("Oracle/ODBC", "odbc"))
}

pub async fn sqlite_provider(
    source: &FileDatabaseSource,
    environment: ExecutionEnvironment,
) -> Result<Provider> {
    ensure_file_allowed(environment, "SQLite", source)?;
    connect_sqlite(source).await
}

#[cfg(feature = "sqlite")]
async fn connect_sqlite(source: &FileDatabaseSource) -> Result<Provider> {
    use datafusion_table_providers::{
        sql::db_connection_pool::{Mode, sqlitepool::SqliteConnectionPoolFactory},
        sqlite::SqliteTableFactory,
    };
    use flow_like_storage::datafusion::common::TableReference;

    let pool = SqliteConnectionPoolFactory::new(
        &source.path,
        Mode::File,
        std::time::Duration::from_secs(30),
    )
    .build()
    .await
    .map_err(|e| flow_like_types::anyhow!("SQLite connection failed: {}", e))?;

    if !source.readonly {
        tracing::warn!("SQLite read-write mode not supported in this version, using read-only");
    }
    SqliteTableFactory::new(Arc::new(pool))
        .table_provider(TableReference::bare(source.table.clone()))
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to create SQLite table provider: {}", e))
}

#[cfg(not(feature = "sqlite"))]
async fn connect_sqlite(_source: &FileDatabaseSource) -> Result<Provider> {
    Err(not_enabled("SQLite", "sqlite"))
}

pub async fn duckdb_provider(
    source: &FileDatabaseSource,
    environment: ExecutionEnvironment,
) -> Result<Provider> {
    ensure_file_allowed(environment, "DuckDB", source)?;
    connect_duckdb(source).await
}

#[cfg(feature = "duckdb")]
async fn connect_duckdb(source: &FileDatabaseSource) -> Result<Provider> {
    use datafusion_table_providers::{
        duckdb::DuckDBTableFactory,
        sql::db_connection_pool::duckdbpool::DuckDbConnectionPoolBuilder,
    };
    use flow_like_storage::datafusion::common::TableReference;

    // Defaults to ReadWrite mode; true read-only needs the forked duckdb crate's AccessMode.
    let builder = if source.path == ":memory:" {
        DuckDbConnectionPoolBuilder::memory()
    } else {
        DuckDbConnectionPoolBuilder::file(&source.path)
    };
    let pool = builder
        .build()
        .map_err(|e| flow_like_types::anyhow!("DuckDB connection failed: {}", e))?;

    let factory = DuckDBTableFactory::new(Arc::new(pool));
    let table_ref = TableReference::bare(source.table.clone());
    if source.readonly {
        factory.table_provider(table_ref).await
    } else {
        factory.read_write_table_provider(table_ref).await
    }
    .map_err(|e| flow_like_types::anyhow!("Failed to create DuckDB table provider: {}", e))
}

#[cfg(not(feature = "duckdb"))]
async fn connect_duckdb(_source: &FileDatabaseSource) -> Result<Provider> {
    Err(not_enabled("DuckDB", "duckdb"))
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub struct CsvSource {
    pub delimiter: u8,
    pub has_header: bool,
}

#[cfg(feature = "execute")]
impl Default for CsvSource {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
        }
    }
}

/// An in-memory table over CSV bytes. Column types are inferred; without a header the columns
/// are named `column_1`, `column_2`, ….
#[cfg(feature = "execute")]
pub fn csv_provider(bytes: &[u8], source: &CsvSource) -> Result<Provider> {
    use crate::data::excel::CSVTable;

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(source.delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut records = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| flow_like_types::anyhow!("CSV parse failed: {e}"))?;
        records.push(record.iter().map(str::to_string).collect::<Vec<_>>());
    }
    let width = records.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return Err(flow_like_types::anyhow!("CSV has no columns"));
    }
    let headers = if source.has_header {
        let mut headers = records.remove(0);
        headers.resize(width, String::new());
        headers
            .into_iter()
            .enumerate()
            .map(|(index, name)| match name.trim() {
                "" => format!("column_{}", index + 1),
                name => name.to_string(),
            })
            .collect()
    } else {
        (1..=width).map(|index| format!("column_{index}")).collect()
    };
    let rows = records
        .into_iter()
        .map(|mut row| {
            row.resize(width, String::new());
            row.into_iter()
                .map(|cell| {
                    if cell.is_empty() {
                        flow_like_types::Value::Null
                    } else {
                        flow_like_types::Value::String(cell)
                    }
                })
                .collect()
        })
        .collect();
    let table: Provider = CSVTable::new(headers, rows, None).to_memtable()?;
    Ok(table)
}

/// In-memory tables over the tables detected in a workbook, as `(table name, provider)`, plus
/// the detector's warnings. Names follow `extract_workbook_tables`.
#[cfg(feature = "execute")]
pub fn workbook_providers(
    bytes: Vec<u8>,
    sheet_filter: Option<&str>,
    config: &crate::data::excel::table_detect::ExtractConfig,
    mode: crate::data::excel::table_detect::SheetTableMode,
    name_prefix: &str,
) -> Result<(Vec<(String, Provider)>, Vec<String>)> {
    let extracted = crate::data::excel::table_detect::extract_workbook_tables(
        bytes,
        sheet_filter,
        config,
        mode,
        name_prefix,
        None,
    )?;
    let mut providers = Vec::with_capacity(extracted.tables.len());
    for table in &extracted.tables {
        let name = table.name.clone().unwrap_or_default();
        let provider: Provider = table.to_memtable()?;
        providers.push((name, provider));
    }
    Ok((providers, extracted.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    fn postgres(host: &str) -> PostgresSource {
        PostgresSource {
            host: host.to_string(),
            port: 5432,
            database: "sales".to_string(),
            username: "reader".to_string(),
            password: "secret".to_string(),
            schema: "public".to_string(),
            table: "orders".to_string(),
            ssl_mode: "prefer".to_string(),
            readonly: true,
        }
    }

    async fn refusal(source: ConnectorSource, environment: ExecutionEnvironment) -> String {
        connect_table(&source, environment)
            .await
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn servers_refuse_hosts_on_the_executors_own_network_plane() {
        for host in [
            "127.0.0.1",
            "localhost",
            "169.254.169.254",
            "169.254.170.2",
            "0.0.0.0",
            "::1",
            "metadata.google.internal",
            "168.63.129.16",
        ] {
            let sources = [
                ConnectorSource::Postgres(postgres(host)),
                ConnectorSource::Mysql(MysqlSource {
                    host: host.to_string(),
                    port: 3306,
                    database: "sales".to_string(),
                    username: "reader".to_string(),
                    password: String::new(),
                    table: "orders".to_string(),
                    ssl_mode: "preferred".to_string(),
                    readonly: true,
                }),
                ConnectorSource::Clickhouse(ClickhouseSource {
                    host: host.to_string(),
                    port: 8123,
                    database: "default".to_string(),
                    username: "default".to_string(),
                    password: String::new(),
                    table: "events".to_string(),
                }),
                ConnectorSource::FlightSql(FlightSqlSource {
                    host: host.to_string(),
                    port: 31337,
                    username: String::new(),
                    password: String::new(),
                    query: "SELECT 1".to_string(),
                    use_tls: false,
                }),
                ConnectorSource::Oracle(oracle(host)),
                ConnectorSource::Odbc(odbc(&format!("Driver={{x}};Server={host}"))),
            ];
            for source in sources {
                let error = refusal(source, ExecutionEnvironment::Server).await;
                assert!(
                    error.contains("network plane"),
                    "{host} must be refused server-side, got: {error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn local_runs_are_not_filtered() {
        let error = refusal(
            ConnectorSource::Postgres(postgres("127.0.0.1")),
            ExecutionEnvironment::Local,
        )
        .await;
        assert!(!error.contains("network plane"), "{error}");
    }

    #[tokio::test]
    async fn public_addresses_pass_the_server_check() {
        ensure_connection_allowed(
            ExecutionEnvironment::Server,
            "PostgreSQL",
            "93.184.216.34",
            5432,
        )
        .await
        .expect("a public IP literal is allowed");
    }

    #[tokio::test]
    async fn public_only_refuses_private_ranges_for_every_driver() {
        let policy = ConnectPolicy::public_only(ExecutionEnvironment::Server);
        for host in [
            "10.0.0.5",
            "192.168.1.10",
            "172.16.0.1",
            "100.64.0.1",
            "fd00::1",
            "127.0.0.1",
        ] {
            let sources = [
                ConnectorSource::Postgres(postgres(host)),
                ConnectorSource::Mysql(MysqlSource {
                    host: host.to_string(),
                    port: 3306,
                    database: "sales".to_string(),
                    username: "reader".to_string(),
                    password: String::new(),
                    table: "orders".to_string(),
                    ssl_mode: "preferred".to_string(),
                    readonly: true,
                }),
                ConnectorSource::Clickhouse(ClickhouseSource {
                    host: host.to_string(),
                    port: 8123,
                    database: "default".to_string(),
                    username: "default".to_string(),
                    password: String::new(),
                    table: "events".to_string(),
                }),
                ConnectorSource::FlightSql(FlightSqlSource {
                    host: host.to_string(),
                    port: 31337,
                    username: String::new(),
                    password: String::new(),
                    query: "SELECT 1".to_string(),
                    use_tls: false,
                }),
                ConnectorSource::Oracle(oracle(host)),
                ConnectorSource::Odbc(odbc(&format!("Driver={{x}};Server={host}"))),
            ];
            for source in sources {
                let error = connect_table(&source, policy)
                    .await
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default();
                assert!(
                    error.contains("refused"),
                    "{host} must be refused in public-only mode, got: {error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn public_only_still_admits_public_addresses_and_ignores_local_runs() {
        ensure_connection_allowed(
            ConnectPolicy::public_only(ExecutionEnvironment::Server),
            "PostgreSQL",
            "93.184.216.34",
            5432,
        )
        .await
        .expect("a public IP literal is allowed");
        ensure_connection_allowed(
            ConnectPolicy::public_only(ExecutionEnvironment::Local),
            "PostgreSQL",
            "10.0.0.5",
            5432,
        )
        .await
        .expect("local runs are not filtered");
    }

    #[tokio::test]
    async fn plain_server_mode_keeps_private_ranges_reachable() {
        ensure_connection_allowed(ExecutionEnvironment::Server, "PostgreSQL", "10.0.0.5", 5432)
            .await
            .expect("private ranges stay a deployment decision");
    }

    #[test]
    fn oracle_connection_string_keeps_driver_braces_and_braces_secrets() {
        let source = oracle("db.example.com");
        assert_eq!(
            source.connection_string(),
            "Driver={Oracle 21 ODBC driver};DBQ=db.example.com:1521/ORCLPDB1;UID=flow;PWD={p;w}"
        );
        assert_eq!(source.effective_schema(), "FLOW");
    }

    #[tokio::test]
    async fn hosts_cannot_smuggle_connection_parameters() {
        for host in [
            "db.example.com hostaddr=169.254.169.254",
            "evil.test/x@169.254.169.254",
            "a=b",
            "",
            "db.example.com?sslmode=disable",
        ] {
            for environment in [ExecutionEnvironment::Local, ExecutionEnvironment::Server] {
                let error = refusal(ConnectorSource::Postgres(postgres(host)), environment).await;
                assert!(error.contains("must be a host name"), "{host:?}: {error}");
            }
        }
    }

    fn oracle(host: &str) -> OracleSource {
        OracleSource {
            host: host.to_string(),
            port: 1521,
            service_name: "ORCLPDB1".to_string(),
            username: "flow".to_string(),
            password: "p;w".to_string(),
            schema: String::new(),
            source_table: "ORDERS".to_string(),
            odbc_driver: "Oracle 21 ODBC driver".to_string(),
        }
    }

    fn odbc(connection_string: &str) -> OdbcSource {
        OdbcSource {
            connection_string: connection_string.to_string(),
            username: "flow".to_string(),
            password: "p;w".to_string(),
            catalog: "SALES".to_string(),
            schema: "PUBLIC".to_string(),
            source_table: "Orders".to_string(),
            table_name: "orders".to_string(),
            max_text_size: 65536,
        }
    }

    #[tokio::test]
    async fn server_odbc_needs_an_explicit_vetted_server_and_no_ambient_identity() {
        let run = |connection_string: &'static str| async move {
            refusal(
                ConnectorSource::Odbc(odbc(connection_string)),
                ExecutionEnvironment::Server,
            )
            .await
        };

        for (connection_string, expected) in [
            ("DSN=corporate", "borrows the executor's own identity"),
            (
                "FILEDSN=/etc/odbc.dsn",
                "borrows the executor's own identity",
            ),
            (
                "Driver={SQL Server};Server=db.example.com;Trusted_Connection=yes",
                "borrows the executor's own identity",
            ),
            (
                "Driver={SQL Server};Server=db.example.com;Integrated Security=SSPI",
                "borrows the executor's own identity",
            ),
            ("Driver={Snowflake};Database=SALES", "explicit Server="),
            ("Driver={SQL Server};Server=127.0.0.1,1433", "network plane"),
            (
                "Driver={SQL Server};Server=tcp:169.254.169.254,1433",
                "network plane",
            ),
            ("Driver={MySQL};Host=localhost;Port=3306", "network plane"),
            ("Driver={Postgres};Server=[::1]:5432", "must be a host name"),
        ] {
            let error = run(connection_string).await;
            assert!(error.contains(expected), "{connection_string}: {error}");
        }

        let allowed = refusal(
            ConnectorSource::Odbc(odbc("Driver={SQL Server};Server=93.184.216.34,1433")),
            ExecutionEnvironment::Server,
        )
        .await;
        assert!(!allowed.contains("network plane"), "{allowed}");
        assert!(!allowed.contains("borrows"), "{allowed}");
    }

    #[test]
    fn odbc_attributes_read_braced_values_and_the_target() {
        let attributes = odbc_attributes(
            "Driver={ODBC Driver 18};Server=tcp:db.example.com,1444;PWD={a;b}}c};UID=x",
        );
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("driver"), Some("ODBC Driver 18"));
        assert_eq!(get("pwd"), Some("a;b}c"));
        assert_eq!(get("uid"), Some("x"));
        assert_eq!(
            odbc_target(&attributes),
            Some(("db.example.com".to_string(), Some(1444)))
        );
        assert_eq!(
            odbc_target(&odbc_attributes("Host=db.example.com:5432")),
            Some(("db.example.com".to_string(), Some(5432)))
        );
        assert_eq!(odbc_target(&odbc_attributes("Driver={x}")), None);
    }

    #[test]
    fn odbc_attribute_braces_values_that_would_split_the_string() {
        assert_eq!(odbc_attribute("PWD", "plain"), "PWD=plain");
        assert_eq!(odbc_attribute("PWD", "a;b"), "PWD={a;b}");
        assert_eq!(odbc_attribute("PWD", "x}y"), "PWD={x}}y}");
        assert_eq!(odbc_attribute("UID", " padded "), "UID={ padded }");
    }

    #[test]
    fn odbc_source_appends_credentials_and_keeps_identifier_case() {
        let source = odbc("Driver={Snowflake};Server=acme.snowflakecomputing.com; ");
        assert_eq!(
            source.full_connection_string(),
            "Driver={Snowflake};Server=acme.snowflakecomputing.com;UID=flow;PWD={p;w}"
        );
        let table = source.table_reference();
        assert_eq!(
            (table.catalog(), table.schema(), table.table()),
            (Some("SALES"), Some("PUBLIC"), "Orders")
        );

        let bare = OdbcSource {
            username: String::new(),
            password: String::new(),
            catalog: String::new(),
            schema: String::new(),
            ..source.clone()
        };
        assert!(!bare.full_connection_string().contains("PWD"));
        assert_eq!(bare.table_reference().schema(), None);
        let partial = OdbcSource {
            schema: "PUBLIC".to_string(),
            ..bare
        };
        assert_eq!(partial.table_reference().schema(), Some("PUBLIC"));
    }

    #[tokio::test]
    async fn odbc_rejects_a_non_positive_text_size() {
        let source = OdbcSource {
            max_text_size: 0,
            ..odbc("Driver={x};Server=93.184.216.34")
        };
        let error = refusal(ConnectorSource::Odbc(source), ExecutionEnvironment::Local).await;
        assert!(error.contains("Max Text Size"), "{error}");
    }

    #[test]
    fn server_file_engines_open_only_staged_files() {
        let root =
            std::env::temp_dir().join(format!("flow-like-staged-{}", flow_like_types::create_id()));
        let outside = std::env::temp_dir().join(format!(
            "flow-like-outside-{}",
            flow_like_types::create_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let staged = root.join("data.db");
        let foreign = outside.join("data.db");
        std::fs::write(&staged, b"x").unwrap();
        std::fs::write(&foreign, b"x").unwrap();
        let source =
            |path: &std::path::Path, staged_root: Option<&std::path::Path>| FileDatabaseSource {
                path: path.to_string_lossy().into_owned(),
                table: "t".to_string(),
                readonly: true,
                staged_root: staged_root.map(Into::into),
            };

        assert!(
            ensure_file_allowed(
                ExecutionEnvironment::Server,
                "SQLite",
                &source(&staged, Some(&root))
            )
            .is_ok()
        );
        for refused in [
            source(&foreign, Some(&root)),
            source(&staged, None),
            source(&root.join("../data.db"), Some(&root)),
            source(&root.join("missing.db"), Some(&root)),
            source(std::path::Path::new("/etc/hosts"), Some(&root)),
        ] {
            let error = ensure_file_allowed(ExecutionEnvironment::Server, "SQLite", &refused)
                .expect_err(&refused.path)
                .to_string();
            assert!(error.contains("never host paths"), "{error}");
        }
        assert!(
            ensure_file_allowed(
                ExecutionEnvironment::Local,
                "SQLite",
                &source(&foreign, None)
            )
            .is_ok()
        );
        let memory = FileDatabaseSource {
            path: ":memory:".to_string(),
            ..source(&staged, None)
        };
        assert!(ensure_file_allowed(ExecutionEnvironment::Server, "DuckDB", &memory).is_ok());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn flight_sql_endpoints_follow_the_tls_flag() {
        let mut source = FlightSqlSource {
            host: "gizmo.example.com".to_string(),
            port: 31337,
            username: String::new(),
            password: String::new(),
            query: "SELECT 1".to_string(),
            use_tls: true,
        };
        assert_eq!(
            flight_sql_endpoint(&source),
            "https://gizmo.example.com:31337"
        );
        source.use_tls = false;
        assert_eq!(
            flight_sql_endpoint(&source),
            "http://gizmo.example.com:31337"
        );
    }

    #[cfg(feature = "execute")]
    async fn first_int(provider: Provider, sql: &str) -> i64 {
        use flow_like_storage::arrow_array::{Array, cast::AsArray, types::Int64Type};
        let ctx = flow_like_storage::datafusion::prelude::SessionContext::new();
        ctx.register_table("t", provider).unwrap();
        let batches = ctx.sql(sql).await.unwrap().collect().await.unwrap();
        let column = batches[0].column(0);
        assert_eq!(column.len(), 1);
        column.as_primitive::<Int64Type>().value(0)
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn csv_provider_infers_types_and_names_headerless_columns() {
        let csv = b"id,name,score\n1,ada,2.5\n2,bob,\n";
        let provider = csv_provider(csv, &CsvSource::default()).unwrap();
        assert_eq!(
            first_int(provider.clone(), "SELECT sum(id) FROM t").await,
            3
        );
        assert_eq!(first_int(provider, "SELECT count(score) FROM t").await, 1);

        let headerless = csv_provider(
            b"1;x\n2;y\n",
            &CsvSource {
                delimiter: b';',
                has_header: false,
            },
        )
        .unwrap();
        let names: Vec<_> = headerless
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        assert_eq!(names, ["column_1", "column_2"]);
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn workbook_providers_register_detected_tables() {
        use crate::data::excel::table_detect::{ExtractConfig, SheetTableMode};

        let mut book = umya_spreadsheet::new_file();
        let sheet = book.get_sheet_mut(&0).unwrap();
        sheet.set_name("Orders");
        for (column, header) in ["Order", "Total"].iter().enumerate() {
            sheet
                .get_cell_mut((column as u32 + 1, 1))
                .set_value_string(*header);
        }
        for (row, total) in [10.0, 32.0].iter().enumerate() {
            let row = row as u32 + 2;
            sheet
                .get_cell_mut((1, row))
                .set_value_string(format!("A-{row}"));
            sheet.get_cell_mut((2, row)).set_value_number(*total);
        }
        let mut bytes = Vec::new();
        umya_spreadsheet::writer::xlsx::write_writer(&book, &mut bytes).unwrap();

        let (providers, _) = workbook_providers(
            bytes,
            None,
            &ExtractConfig::default(),
            SheetTableMode::WholeSheet,
            "",
        )
        .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].0, "orders");
        let total = first_int(
            providers[0].1.clone(),
            "SELECT CAST(sum(\"Total\") AS BIGINT) FROM t",
        )
        .await;
        assert_eq!(total, 42);
    }

    #[cfg(feature = "execute")]
    #[test]
    fn csv_provider_rejects_empty_input() {
        assert!(csv_provider(b"", &CsvSource::default()).is_err());
    }
}
