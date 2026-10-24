use crate::data::datafusion::session::DataFusionSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[allow(unused_imports)]
use std::sync::Arc;

// ============================================================================
// REAL DATABASE PROVIDERS via datafusion-table-providers
// These implementations use actual TableProviders for full query federation.
// Each feature flag corresponds to a database backend.
// ============================================================================

#[cfg(feature = "postgres")]
use datafusion_table_providers::{
    postgres::PostgresTableFactory, sql::db_connection_pool::postgrespool::PostgresConnectionPool,
    util::secrets::to_secret_map,
};

#[cfg(feature = "mysql")]
use datafusion_table_providers::{
    mysql::MySQLTableFactory, sql::db_connection_pool::mysqlpool::MySQLConnectionPool,
    util::secrets::to_secret_map as mysql_to_secret_map,
};

#[cfg(feature = "sqlite")]
use datafusion_table_providers::{
    sql::db_connection_pool::{Mode as SqliteMode, sqlitepool::SqliteConnectionPoolFactory},
    sqlite::SqliteTableFactory,
};

#[cfg(feature = "duckdb")]
use datafusion_table_providers::{
    duckdb::DuckDBTableFactory, sql::db_connection_pool::duckdbpool::DuckDbConnectionPoolBuilder,
};

#[cfg(feature = "clickhouse")]
use datafusion_table_providers::{
    clickhouse::ClickHouseTableFactory,
    sql::db_connection_pool::clickhousepool::ClickHouseConnectionPool,
    util::secrets::to_secret_map as clickhouse_to_secret_map,
};

/// HTTP-based drivers (ClickHouse, Flight SQL) bring their own clients, so a
/// flow-supplied endpoint is vetted against the egress policy before connecting.
#[cfg(all(feature = "execute", any(feature = "clickhouse", feature = "flight")))]
async fn ensure_endpoint_allowed(
    context: &ExecutionContext,
    driver: &str,
    endpoint: &str,
) -> flow_like_types::Result<()> {
    let url = flow_like_types::reqwest::Url::parse(endpoint).map_err(|error| {
        flow_like_types::anyhow!("{driver} endpoint '{endpoint}' is not a valid URL: {error}")
    })?;
    flow_like::flow::execution::egress::ensure_url_resolves_allowed(
        context.execution_environment(),
        &url,
    )
    .await
}

/// Product binaries compile rustls with both ring and aws-lc-rs, so rustls cannot pick a
/// process-wide provider itself, and drivers that build their TLS config internally
/// panic until one is installed. Mirrors the catalog-web TLS nodes.
#[cfg(all(feature = "execute", any(feature = "postgres", feature = "mysql")))]
fn ensure_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

// ============================================================================
// PostgreSQL Node
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterPostgresNode {}

impl RegisterPostgresNode {
    pub fn new() -> Self {
        RegisterPostgresNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterPostgresNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_postgres",
            "Register PostgreSQL",
            "Register a PostgreSQL table for federated queries. Uses real database connection for full SQL push-down.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerPostgres");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "host",
            "Host",
            "PostgreSQL server host",
            VariableType::String,
        )
        .set_default_value(Some(json!("localhost")));
        node.add_input_pin(
            "port",
            "Port",
            "PostgreSQL server port",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5432)));
        node.add_input_pin(
            "database",
            "Database",
            "Database name",
            VariableType::String,
        );
        node.add_input_pin(
            "username",
            "Username",
            "Database username",
            VariableType::String,
        );
        node.add_input_pin(
            "password",
            "Password",
            "Database password",
            VariableType::String,
        );
        node.add_input_pin(
            "schema",
            "Schema",
            "PostgreSQL schema",
            VariableType::String,
        )
        .set_default_value(Some(json!("public")));
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in PostgreSQL",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "ssl_mode",
            "SSL Mode",
            "SSL mode: disable, prefer, require, verify-ca, verify-full",
            VariableType::String,
        )
        .set_default_value(Some(json!("prefer")));
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Open connection in read-only mode",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "connection_url",
            "Connection URL",
            "Generated connection URL (without password)",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 7,
            governance: 7,
            reliability: 8,
            cost: 7,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let database: String = context.evaluate_pin("database").await?;
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;
        let schema: String = context
            .evaluate_pin("schema")
            .await
            .unwrap_or_else(|_| "public".to_string());
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let ssl_mode: String = context
            .evaluate_pin("ssl_mode")
            .await
            .unwrap_or_else(|_| "prefer".to_string());
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let cached_session = session.load_lazy(context).await?;
        let conn_url = format!(
            "postgresql://{}:****@{}:{}/{}?sslmode={}",
            username, host, port, database, ssl_mode
        );

        #[cfg(feature = "postgres")]
        {
            use flow_like_storage::datafusion::common::TableReference;
            use std::collections::HashMap;

            let mut params = HashMap::new();
            params.insert("host".to_string(), host.clone());
            params.insert("port".to_string(), port.to_string());
            params.insert("user".to_string(), username.clone());
            params.insert("pass".to_string(), password);
            params.insert("db".to_string(), database.clone());
            params.insert("sslmode".to_string(), ssl_mode.clone());

            ensure_crypto_provider();
            let pool = PostgresConnectionPool::new(to_secret_map(params))
                .await
                .map_err(|e| flow_like_types::anyhow!("PostgreSQL connection failed: {}", e))?;

            let factory = PostgresTableFactory::new(Arc::new(pool));
            let table_ref =
                TableReference::full(database.clone(), schema.clone(), source_table.clone());

            let table_provider = if readonly {
                factory.table_provider(table_ref).await
            } else {
                factory.read_write_table_provider(table_ref).await
            }
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to create PostgreSQL table provider: {}", e)
            })?;

            // The provider reads columns from the catalog, so a missing table comes back
            // as a table without columns instead of an error.
            if table_provider.schema().fields().is_empty() {
                return Err(flow_like_types::anyhow!(
                    "PostgreSQL table '{schema}.{source_table}' was not found in database \
                     '{database}', or none of its columns has a supported type"
                ));
            }

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "PostgreSQL table '{}' registered as '{}' (readonly: {})",
                source_table,
                table_name,
                readonly
            );
        }

        #[cfg(not(feature = "postgres"))]
        {
            let _ = (
                host,
                port,
                database,
                username,
                password,
                schema,
                source_table,
                table_name,
                ssl_mode,
                readonly,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "PostgreSQL support is not enabled. Rebuild with the 'postgres' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("connection_url", json!(conn_url))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// MySQL Node
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMysqlNode {}

impl RegisterMysqlNode {
    pub fn new() -> Self {
        RegisterMysqlNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterMysqlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_mysql",
            "Register MySQL",
            "Register a MySQL table for federated queries. Uses real database connection for full SQL push-down.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerMysql");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin("host", "Host", "MySQL server host", VariableType::String)
            .set_default_value(Some(json!("localhost")));
        node.add_input_pin("port", "Port", "MySQL server port", VariableType::Integer)
            .set_default_value(Some(json!(3306)));
        node.add_input_pin(
            "database",
            "Database",
            "Database name",
            VariableType::String,
        );
        node.add_input_pin(
            "username",
            "Username",
            "Database username",
            VariableType::String,
        );
        node.add_input_pin(
            "password",
            "Password",
            "Database password",
            VariableType::String,
        );
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in MySQL",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "ssl_mode",
            "SSL Mode",
            "SSL mode: disabled, preferred, required",
            VariableType::String,
        )
        .set_default_value(Some(json!("preferred")));
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Open connection in read-only mode",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "connection_url",
            "Connection URL",
            "Generated connection URL",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 7,
            governance: 7,
            reliability: 8,
            cost: 7,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let database: String = context.evaluate_pin("database").await?;
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let ssl_mode: String = context
            .evaluate_pin("ssl_mode")
            .await
            .unwrap_or_else(|_| "preferred".to_string());
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let cached_session = session.load_lazy(context).await?;
        let conn_url = format!(
            "mysql://{}:****@{}:{}/{}?ssl-mode={}",
            username, host, port, database, ssl_mode
        );

        #[cfg(feature = "mysql")]
        {
            use flow_like_storage::datafusion::common::TableReference;
            use std::collections::HashMap;

            let connection_string = format!(
                "mysql://{}:{}@{}:{}/{}",
                username, password, host, port, database
            );

            let mut params = HashMap::new();
            params.insert("connection_string".to_string(), connection_string);
            params.insert("sslmode".to_string(), ssl_mode.clone());

            ensure_crypto_provider();
            let pool = MySQLConnectionPool::new(mysql_to_secret_map(params))
                .await
                .map_err(|e| flow_like_types::anyhow!("MySQL connection failed: {}", e))?;

            // The pool is already bound to `database`, and the provider's writer resolves
            // the table by its full display name, so only a bare name reads and writes.
            let factory = MySQLTableFactory::new(Arc::new(pool));
            let table_ref = TableReference::bare(source_table.clone());

            let table_provider = if readonly {
                factory.table_provider(table_ref).await
            } else {
                factory.read_write_table_provider(table_ref).await
            }
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to create MySQL table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "MySQL table '{}' registered as '{}' (readonly: {})",
                source_table,
                table_name,
                readonly
            );
        }

        #[cfg(not(feature = "mysql"))]
        {
            let _ = (
                host,
                port,
                database,
                username,
                password,
                source_table,
                table_name,
                ssl_mode,
                readonly,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "MySQL support is not enabled. Rebuild with the 'mysql' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("connection_url", json!(conn_url))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// SQLite Node
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterSqliteNode {}

impl RegisterSqliteNode {
    pub fn new() -> Self {
        RegisterSqliteNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterSqliteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_sqlite",
            "Register SQLite",
            "Register a SQLite database table for federated queries. Uses real database connection.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerSqlite");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to SQLite database file",
            VariableType::String,
        );
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in SQLite",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Open database in read-only mode",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 8,
            reliability: 9,
            cost: 10,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let cached_session = session.load_lazy(context).await?;

        if !std::path::Path::new(&file_path).exists() {
            return Err(flow_like_types::anyhow!(
                "SQLite file not found: {}",
                file_path
            ));
        }

        #[cfg(feature = "sqlite")]
        {
            use flow_like_storage::datafusion::common::TableReference;
            use std::time::Duration;

            let pool_factory = SqliteConnectionPoolFactory::new(
                &file_path,
                SqliteMode::File,
                Duration::from_secs(30),
            );
            let pool = pool_factory
                .build()
                .await
                .map_err(|e| flow_like_types::anyhow!("SQLite connection failed: {}", e))?;

            let factory = SqliteTableFactory::new(Arc::new(pool));
            let table_ref = TableReference::bare(source_table.clone());

            if !readonly {
                tracing::warn!(
                    "SQLite read-write mode not supported in this version, using read-only"
                );
            }
            let table_provider = factory.table_provider(table_ref).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to create SQLite table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "SQLite table '{}:{}' registered as '{}' (readonly: {})",
                file_path,
                source_table,
                table_name,
                readonly
            );
        }

        #[cfg(not(feature = "sqlite"))]
        {
            let _ = (
                file_path,
                source_table,
                table_name,
                readonly,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "SQLite support is not enabled. Rebuild with the 'sqlite' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// DuckDB Node
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterDuckdbNode {}

impl RegisterDuckdbNode {
    pub fn new() -> Self {
        RegisterDuckdbNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterDuckdbNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_duckdb",
            "Register DuckDB",
            "Register a DuckDB database table for federated queries. Uses real database connection.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerDuckdb");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to DuckDB database file (or :memory:)",
            VariableType::String,
        )
        .set_default_value(Some(json!(":memory:")));
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in DuckDB",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Open database in read-only mode",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 10,
            governance: 8,
            reliability: 9,
            cost: 10,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let cached_session = session.load_lazy(context).await?;

        if file_path != ":memory:" && !std::path::Path::new(&file_path).exists() {
            return Err(flow_like_types::anyhow!(
                "DuckDB file not found: {}",
                file_path
            ));
        }

        #[cfg(feature = "duckdb")]
        {
            use flow_like_storage::datafusion::common::TableReference;

            // Use builder pattern - defaults to ReadWrite mode
            // Note: For true read-only mode, would need the forked duckdb crate's AccessMode
            let pool_builder = if file_path == ":memory:" {
                DuckDbConnectionPoolBuilder::memory()
            } else {
                DuckDbConnectionPoolBuilder::file(&file_path)
            };

            let pool = pool_builder
                .build()
                .map_err(|e| flow_like_types::anyhow!("DuckDB connection failed: {}", e))?;

            let factory = DuckDBTableFactory::new(Arc::new(pool));
            let table_ref = TableReference::bare(source_table.clone());

            let table_provider = if readonly {
                factory.table_provider(table_ref).await
            } else {
                factory.read_write_table_provider(table_ref).await
            }
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to create DuckDB table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "DuckDB table '{}:{}' registered as '{}' (readonly: {})",
                file_path,
                source_table,
                table_name,
                readonly
            );
        }

        #[cfg(not(feature = "duckdb"))]
        {
            let _ = (
                file_path,
                source_table,
                table_name,
                readonly,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "DuckDB support is not enabled. Rebuild with the 'duckdb' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// ClickHouse Node
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterClickhouseNode {}

impl RegisterClickhouseNode {
    pub fn new() -> Self {
        RegisterClickhouseNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterClickhouseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_clickhouse",
            "Register ClickHouse",
            "Register a ClickHouse table for federated queries. Uses real database connection for full SQL push-down.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerClickhouse");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "host",
            "Host",
            "ClickHouse server host",
            VariableType::String,
        )
        .set_default_value(Some(json!("localhost")));
        node.add_input_pin(
            "port",
            "Port",
            "ClickHouse HTTP port",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8123)));
        node.add_input_pin(
            "database",
            "Database",
            "Database name",
            VariableType::String,
        )
        .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "username",
            "Username",
            "Database username",
            VariableType::String,
        )
        .set_default_value(Some(json!("default")));
        node.add_input_pin(
            "password",
            "Password",
            "Database password",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in ClickHouse",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Use read-only queries",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "connection_url",
            "Connection URL",
            "Generated connection URL",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 10,
            governance: 7,
            reliability: 8,
            cost: 7,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let database: String = context
            .evaluate_pin("database")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let username: String = context
            .evaluate_pin("username")
            .await
            .unwrap_or_else(|_| "default".to_string());
        let password: String = context.evaluate_pin("password").await.unwrap_or_default();
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        let cached_session = session.load_lazy(context).await?;
        let conn_url = format!("http://{}:****@{}:{}/{}", username, host, port, database);

        #[cfg(feature = "clickhouse")]
        {
            use std::collections::HashMap;

            let url = format!("http://{}:{}", host, port);
            ensure_endpoint_allowed(context, "ClickHouse", &url).await?;

            let mut params = HashMap::new();
            params.insert("url".to_string(), url);
            params.insert("database".to_string(), database.clone());
            params.insert("user".to_string(), username.clone());
            if !password.is_empty() {
                params.insert("password".to_string(), password);
            }
            // ClickHouse otherwise ships String columns as Arrow Binary, so text surfaces
            // in rows as byte arrays.
            params.insert(
                "option_output_format_arrow_string_as_string".to_string(),
                "1".to_string(),
            );

            let pool = ClickHouseConnectionPool::new(clickhouse_to_secret_map(params))
                .await
                .map_err(|e| flow_like_types::anyhow!("ClickHouse connection failed: {}", e))?;

            let factory = ClickHouseTableFactory::new(Arc::new(pool));
            let table_ref = flow_like_storage::datafusion::common::TableReference::partial(
                database.clone(),
                source_table.clone(),
            );

            // ClickHouse only supports read-only access (no read_write_table_provider)
            if !readonly {
                tracing::warn!(
                    "ClickHouse tables are read-only. Write operations will fail at runtime."
                );
            }

            let table_provider = factory.table_provider(table_ref, None).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to create ClickHouse table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "ClickHouse table '{}' registered as '{}' (readonly: {})",
                source_table,
                table_name,
                readonly
            );
        }

        #[cfg(not(feature = "clickhouse"))]
        {
            let _ = (
                host,
                port,
                database,
                username,
                password,
                source_table,
                table_name,
                readonly,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "ClickHouse support is not enabled. Rebuild with the 'clickhouse' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("connection_url", json!(conn_url))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// Oracle Node (via ODBC)
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterOracleNode {}

impl RegisterOracleNode {
    pub fn new() -> Self {
        RegisterOracleNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterOracleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_oracle",
            "Register Oracle",
            "Register an Oracle database table for federated queries via ODBC. Requires Oracle Instant Client with ODBC driver installed.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerOracle");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin("host", "Host", "Oracle server host", VariableType::String)
            .set_default_value(Some(json!("localhost")));
        node.add_input_pin(
            "port",
            "Port",
            "Oracle listener port",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1521)));
        node.add_input_pin(
            "service_name",
            "Service Name",
            "Oracle service name or SID",
            VariableType::String,
        )
        .set_default_value(Some(json!("FREEPDB1")));
        node.add_input_pin(
            "username",
            "Username",
            "Database username",
            VariableType::String,
        );
        node.add_input_pin(
            "password",
            "Password",
            "Database password",
            VariableType::String,
        );
        node.add_input_pin(
            "schema",
            "Schema",
            "Oracle schema (defaults to username)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in Oracle",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "odbc_driver",
            "ODBC Driver",
            "ODBC driver name (e.g., 'Oracle 21 ODBC driver')",
            VariableType::String,
        )
        .set_default_value(Some(json!("Oracle 21 ODBC driver")));
        node.add_input_pin(
            "readonly",
            "Read Only",
            "Open connection in read-only mode",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "connection_url",
            "Connection URL",
            "Generated connection URL (without password)",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 8,
            governance: 8,
            reliability: 9,
            cost: 4,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let service_name: String = context.evaluate_pin("service_name").await?;
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;
        let schema: String = context
            .evaluate_pin("schema")
            .await
            .unwrap_or_else(|_| String::new());
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let odbc_driver: String = context
            .evaluate_pin("odbc_driver")
            .await
            .unwrap_or_else(|_| "Oracle 21 ODBC driver".to_string());
        let readonly: bool = context.evaluate_pin("readonly").await.unwrap_or(true);

        #[cfg(not(feature = "odbc"))]
        {
            let _ = (
                host,
                port,
                service_name,
                username,
                password,
                schema,
                source_table,
                table_name,
                odbc_driver,
                readonly,
                session,
            );
            return Err(flow_like_types::anyhow!(
                "Oracle/ODBC support is not enabled. Rebuild with the 'odbc' feature flag. \
                 Note: Oracle requires the Oracle Instant Client with ODBC driver installed on the system."
            ));
        }

        #[cfg(feature = "odbc")]
        {
            use datafusion_table_providers::odbc::ODBCTableFactory;
            use datafusion_table_providers::sql::db_connection_pool::odbcpool::ODBCPool;
            use datafusion_table_providers::util::secrets::to_secret_map;
            use flow_like_storage::datafusion::common::TableReference;
            use std::collections::HashMap;

            let cached_session = session.load_lazy(context).await?;
            let effective_schema = if schema.is_empty() {
                username.to_uppercase()
            } else {
                schema.to_uppercase()
            };
            let conn_url = format!(
                "oracle://{}:****@{}:{}/{}",
                username, host, port, service_name
            );

            let odbc_conn_string = format!(
                "Driver={{{}}};DBQ={}:{}/{};UID={};PWD={}",
                odbc_driver, host, port, service_name, username, password
            );

            let mut params = HashMap::new();
            params.insert("connection_string".to_string(), odbc_conn_string);

            let pool = ODBCPool::new(to_secret_map(params))
                .map_err(|e| flow_like_types::anyhow!("Oracle ODBC connection failed: {}", e))?;

            let factory = ODBCTableFactory::new(Arc::new(pool));
            let table_ref = TableReference::full(
                service_name.clone(),
                effective_schema.clone(),
                source_table.clone(),
            );

            let table_provider = factory.table_provider(table_ref, None).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to create Oracle table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "Oracle table '{}.{}' registered as '{}' via ODBC (readonly: {})",
                effective_schema,
                source_table,
                table_name,
                readonly
            );

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("connection_url", json!(conn_url))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
    }
}

// ============================================================================
// Generic ODBC Node
// ============================================================================

/// ODBC connection-string values containing `;`, braces or edge whitespace must be
/// wrapped in braces, with closing braces doubled.
#[cfg(feature = "odbc")]
fn odbc_attribute(key: &str, value: &str) -> String {
    if value.contains([';', '{', '}']) || value.trim() != value {
        format!("{key}={{{}}}", value.replace('}', "}}"))
    } else {
        format!("{key}={value}")
    }
}

/// The resolved inputs of Register ODBC Table.
#[cfg(feature = "execute")]
#[cfg_attr(not(feature = "odbc"), allow(dead_code))]
struct OdbcSource {
    connection_string: String,
    username: String,
    password: String,
    catalog: String,
    schema: String,
    source_table: String,
    table_name: String,
    max_text_size: i64,
}

#[cfg(feature = "execute")]
impl OdbcSource {
    async fn from_pins(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        Ok(Self {
            connection_string: context.evaluate_pin("connection_string").await?,
            username: context.evaluate_pin("username").await.unwrap_or_default(),
            password: context.evaluate_pin("password").await.unwrap_or_default(),
            catalog: context.evaluate_pin("catalog").await.unwrap_or_default(),
            schema: context.evaluate_pin("schema").await.unwrap_or_default(),
            source_table: context.evaluate_pin("source_table").await?,
            table_name: context.evaluate_pin("table_name").await?,
            max_text_size: context.evaluate_pin("max_text_size").await.unwrap_or(65536),
        })
    }
}

#[cfg(all(feature = "execute", feature = "odbc"))]
impl OdbcSource {
    fn connection_string(&self) -> String {
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
    fn table_reference(&self) -> flow_like_storage::datafusion::common::TableReference {
        use flow_like_storage::datafusion::common::TableReference;
        let table = self.source_table.clone();
        match (self.catalog.is_empty(), self.schema.is_empty()) {
            (true, true) => TableReference::bare(table),
            (true, false) => TableReference::partial(self.schema.clone(), table),
            (false, _) => TableReference::full(self.catalog.clone(), self.schema.clone(), table),
        }
    }

    async fn register(
        &self,
        session: &crate::data::datafusion::session::CachedDataFusionSession,
    ) -> flow_like_types::Result<()> {
        use datafusion_table_providers::odbc::ODBCTableFactory;
        use datafusion_table_providers::sql::db_connection_pool::odbcpool::ODBCPool;
        use datafusion_table_providers::util::secrets::to_secret_map;
        use std::collections::HashMap;

        if self.max_text_size <= 0 {
            return Err(flow_like_types::anyhow!(
                "Max Text Size must be a positive number of bytes, got {}",
                self.max_text_size
            ));
        }
        let params = HashMap::from([
            ("connection_string".to_string(), self.connection_string()),
            ("max_text_size".to_string(), self.max_text_size.to_string()),
        ]);
        let pool = ODBCPool::new(to_secret_map(params))
            .map_err(|e| flow_like_types::anyhow!("ODBC connection failed: {}", e))?;

        let table_ref = self.table_reference();
        let table_provider = ODBCTableFactory::new(Arc::new(pool))
            .table_provider(table_ref.clone(), None)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to create ODBC table provider for '{table_ref}': {e}"
                )
            })?;
        session
            .replace_table(&self.table_name, table_provider)
            .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

        tracing::info!(
            "ODBC table '{}' registered as '{}'",
            table_ref,
            self.table_name
        );
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterOdbcNode {}

impl RegisterOdbcNode {
    pub fn new() -> Self {
        RegisterOdbcNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterOdbcNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_odbc",
            "Register ODBC Table",
            "Register a table from any database with an installed ODBC driver (Snowflake, SQL Server, Teradata, …) for federated queries. The driver and an ODBC driver manager must be installed where the flow runs.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerOdbc");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "connection_string",
            "Connection String",
            "ODBC connection string without credentials, e.g. Driver={SnowflakeDSIIDriver};Server=<account>.snowflakecomputing.com;Database=SALES;Warehouse=COMPUTE_WH",
            VariableType::String,
        );
        node.add_input_pin(
            "username",
            "Username",
            "Database username, sent as UID (leave empty if the connection string authenticates)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "password",
            "Password",
            "Database password, sent as PWD",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "catalog",
            "Catalog",
            "Catalog (database) of the source table; empty uses the connection default",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "schema",
            "Schema",
            "Schema of the source table; empty uses the connection default",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Table name exactly as the source spells it (case is preserved)",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "max_text_size",
            "Max Text Size",
            "Largest text value fetched per cell, in bytes. Drivers report columns like Snowflake VARCHAR or PostgreSQL TEXT as unbounded, so the fetch buffers need a cap; longer values fail the query.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(65536)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 7,
            governance: 7,
            reliability: 7,
            cost: 6,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let source = OdbcSource::from_pins(context).await?;
        let cached_session = session.load_lazy(context).await?;

        #[cfg(feature = "odbc")]
        source.register(&cached_session).await?;

        #[cfg(not(feature = "odbc"))]
        {
            let _ = (source, cached_session);
            return Err(flow_like_types::anyhow!(
                "ODBC support is not enabled. Rebuild with the 'odbc' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// FlightSQL Node
// ============================================================================
//
// ## What is FlightSQL?
//
// Apache Arrow Flight SQL is a protocol for interacting with SQL databases
// using the Arrow Flight RPC framework. It provides:
//
// 1. **High Performance**: Uses gRPC + Arrow IPC format for efficient columnar
//    data transfer. 10-100x faster than JDBC/ODBC for analytics workloads.
//
// 2. **Language Agnostic**: Works with any language that supports gRPC and Arrow.
//
// 3. **Universal SQL Interface**: Any database that implements Flight SQL can
//    be queried with the same client code.
//
// **Supported Databases**:
// - Apache Drill, Apache Spark, Ballista
// - ClickHouse (via Flight SQL interface)
// - Dremio
// - DuckDB (embedded Flight SQL server)
// - InfluxDB IOx
// - SQLite (via go-sqlite3-flight)
// - And many more...
//
// **Use Cases**:
// - Connecting to remote analytical databases
// - High-throughput data pipelines
// - Cross-database federation with native Arrow support
// - Replacing slow JDBC/ODBC connections for analytics
//
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterFlightSqlNode {}

impl RegisterFlightSqlNode {
    pub fn new() -> Self {
        RegisterFlightSqlNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterFlightSqlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_flightsql",
            "Register FlightSQL",
            "Register a table via Arrow Flight SQL protocol. High-performance columnar data transfer (10-100x faster than JDBC/ODBC). Supports Dremio, InfluxDB, DuckDB Flight, ClickHouse Flight, and more.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerFlightsql");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_input_pin(
            "host",
            "Host",
            "Flight SQL server host",
            VariableType::String,
        )
        .set_default_value(Some(json!("localhost")));
        node.add_input_pin(
            "port",
            "Port",
            "Flight SQL server port (typically 443 for TLS, or service-specific)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(443)));
        node.add_input_pin(
            "username",
            "Username",
            "Username for authentication (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "password",
            "Password/Token",
            "Password or bearer token for authentication (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute (e.g., SELECT * FROM my_table)",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register the query result in DataFusion",
            VariableType::String,
        );
        node.add_input_pin(
            "use_tls",
            "Use TLS",
            "Enable TLS/SSL encryption for the connection",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "skip_verify",
            "Skip TLS Verify",
            "Skip TLS certificate verification (for self-signed certs)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "endpoint",
            "Endpoint",
            "Flight SQL endpoint URL",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 6,
            security: 7,
            performance: 10,
            governance: 7,
            reliability: 8,
            cost: 8,
        });
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let host: String = context.evaluate_pin("host").await?;
        let port: i64 = context.evaluate_pin("port").await?;
        let username: String = context.evaluate_pin("username").await.unwrap_or_default();
        let password: String = context.evaluate_pin("password").await.unwrap_or_default();
        let query: String = context.evaluate_pin("query").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let use_tls: bool = context.evaluate_pin("use_tls").await.unwrap_or(true);
        let _skip_verify: bool = context.evaluate_pin("skip_verify").await.unwrap_or(false);

        let cached_session = session.load_lazy(context).await?;
        let scheme = if use_tls { "https" } else { "http" };
        let endpoint = format!("{}://{}:{}", scheme, host, port);

        #[cfg(feature = "flight")]
        {
            use datafusion_table_providers::flight::FlightTableFactory;
            use datafusion_table_providers::flight::sql::{
                FlightSqlDriver, PASSWORD, QUERY, USERNAME,
            };
            use std::collections::HashMap;

            ensure_endpoint_allowed(context, "Flight SQL", &endpoint).await?;
            let driver = FlightSqlDriver::new();
            let factory = FlightTableFactory::new(Arc::new(driver));

            let mut options: HashMap<String, String> = HashMap::new();
            options.insert(QUERY.to_string(), query);
            if !username.is_empty() {
                options.insert(USERNAME.to_string(), username);
            }
            if !password.is_empty() {
                options.insert(PASSWORD.to_string(), password);
            }

            let table = factory
                .open_table(&endpoint, options)
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to create FlightSQL table: {}", e))?;

            cached_session
                .replace_table(&table_name, Arc::new(table))
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "FlightSQL query registered as '{}' (endpoint: {})",
                table_name,
                endpoint
            );
        }

        #[cfg(not(feature = "flight"))]
        {
            let _ = (
                host,
                port,
                username,
                password,
                query,
                table_name,
                use_tls,
                cached_session,
            );
            return Err(flow_like_types::anyhow!(
                "FlightSQL support is not enabled. Rebuild with the 'flight' feature flag."
            ));
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("endpoint", json!(endpoint)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

// ============================================================================
// AWS Athena Node (via ODBC)
// ============================================================================
//
// ## What is AWS Athena?
//
// Amazon Athena is a serverless interactive query service that allows you to
// analyze data in Amazon S3 using standard SQL. It integrates with AWS Glue
// Data Catalog for schema management.
//
// ## Connection Method
//
// This implementation uses the Simba Athena ODBC driver which provides:
// - Standard SQL support via ODBC
// - AWS IAM authentication
// - Workgroup and data catalog support
// - Query result location management
//
// ## Prerequisites
//
// 1. Install Simba Athena ODBC Driver from AWS:
//    https://docs.aws.amazon.com/athena/latest/ug/connect-with-odbc.html
//
// 2. Configure AWS credentials (IAM user or role with Athena permissions)
//
// 3. Create an S3 bucket for query results
//
// ============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RegisterAthenaNode {}

impl RegisterAthenaNode {
    pub fn new() -> Self {
        RegisterAthenaNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterAthenaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_athena",
            "Register AWS Athena",
            "Register an AWS Athena table for federated queries via ODBC. Query data in S3 using serverless SQL.",
            "Data/DataFusion/Databases",
        );
        node.set_flowscript_name("df", "registerAthena");
        node.set_receiver("session");
        node.add_icon("/flow/icons/aws.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "region",
            "AWS Region",
            "AWS region (e.g., us-east-1)",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));
        node.add_input_pin(
            "access_key_id",
            "Access Key ID",
            "AWS Access Key ID",
            VariableType::String,
        );
        node.add_input_pin(
            "secret_access_key",
            "Secret Access Key",
            "AWS Secret Access Key",
            VariableType::String,
        );
        node.add_input_pin(
            "s3_output_location",
            "S3 Output",
            "S3 path for query results (e.g., s3://my-bucket/athena-results/)",
            VariableType::String,
        );

        node.add_input_pin(
            "catalog",
            "Catalog",
            "Athena data catalog name",
            VariableType::String,
        )
        .set_default_value(Some(json!("AwsDataCatalog")));
        node.add_input_pin(
            "database",
            "Database",
            "Athena/Glue database name",
            VariableType::String,
        );
        node.add_input_pin(
            "source_table",
            "Source Table",
            "Name of the table in Athena",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register in DataFusion",
            VariableType::String,
        );

        node.add_input_pin(
            "workgroup",
            "Workgroup",
            "Athena workgroup (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin(
            "odbc_driver",
            "ODBC Driver",
            "Simba Athena ODBC driver name/path",
            VariableType::String,
        )
        .set_default_value(Some(json!("Simba Athena ODBC Driver")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "DataFusion session",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();
        node.add_output_pin(
            "connection_info",
            "Connection Info",
            "Connection details (without secrets)",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 5,
            security: 6,
            performance: 7,
            governance: 8,
            reliability: 7,
            cost: 5,
        });
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let region: String = context.evaluate_pin("region").await?;
        let access_key_id: String = context.evaluate_pin("access_key_id").await?;
        let secret_access_key: String = context.evaluate_pin("secret_access_key").await?;
        let s3_output_location: String = context.evaluate_pin("s3_output_location").await?;
        let catalog: String = context
            .evaluate_pin("catalog")
            .await
            .unwrap_or_else(|_| "AwsDataCatalog".to_string());
        let database: String = context.evaluate_pin("database").await?;
        let source_table: String = context.evaluate_pin("source_table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let workgroup: String = context
            .evaluate_pin("workgroup")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let odbc_driver: String = context
            .evaluate_pin("odbc_driver")
            .await
            .unwrap_or_else(|_| "Simba Athena ODBC Driver".to_string());

        #[cfg(feature = "odbc")]
        {
            use datafusion_table_providers::odbc::ODBCTableFactory;
            use datafusion_table_providers::sql::db_connection_pool::odbcpool::ODBCPool;
            use datafusion_table_providers::util::secrets::to_secret_map;
            use flow_like_storage::datafusion::common::TableReference;
            use std::collections::HashMap;

            let cached_session = session.load_lazy(context).await?;
            let conn_info = format!(
                "athena://{}@{}.{} (region: {}, workgroup: {})",
                catalog, database, source_table, region, workgroup
            );

            let odbc_conn_string = format!(
                "Driver={{{}}};AwsRegion={};S3OutputLocation={};AuthenticationType=IAM Credentials;UID={};PWD={};Catalog={};Workgroup={}",
                odbc_driver,
                region,
                s3_output_location,
                access_key_id,
                secret_access_key,
                catalog,
                workgroup
            );

            let mut params = HashMap::new();
            params.insert("connection_string".to_string(), odbc_conn_string);

            let pool = ODBCPool::new(to_secret_map(params))
                .map_err(|e| flow_like_types::anyhow!("Athena ODBC connection failed: {}", e))?;

            let factory = ODBCTableFactory::new(Arc::new(pool));
            let table_ref =
                TableReference::full(catalog.clone(), database.clone(), source_table.clone());

            let table_provider = factory.table_provider(table_ref, None).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to create Athena table provider: {}", e)
            })?;

            cached_session
                .replace_table(&table_name, table_provider)
                .map_err(|e| flow_like_types::anyhow!("Failed to register table: {}", e))?;

            tracing::info!(
                "AWS Athena table '{}.{}' registered as '{}' via ODBC",
                database,
                source_table,
                table_name
            );

            context.set_pin_value("session_out", json!(session)).await?;
            context
                .set_pin_value("connection_info", json!(conn_info))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "odbc"))]
        {
            let _ = (
                session,
                region,
                access_key_id,
                secret_access_key,
                s3_output_location,
                catalog,
                database,
                source_table,
                table_name,
                workgroup,
                odbc_driver,
            );
            Err(flow_like_types::anyhow!(
                "ODBC support is not enabled. Rebuild with the 'odbc' feature flag to use AWS Athena."
            ))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;
    use flow_like::flow::variable::VariableType;

    #[test]
    fn test_register_postgres_node_structure() {
        let node_logic = RegisterPostgresNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_postgres");
        assert_eq!(node.friendly_name, "Register PostgreSQL");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_postgres_node_input_pins() {
        let node_logic = RegisterPostgresNode::new();
        let node = node_logic.get_node();
        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        let required_pins = [
            ("exec_in", VariableType::Execution),
            ("session", VariableType::Struct),
            ("host", VariableType::String),
            ("port", VariableType::Integer),
            ("database", VariableType::String),
            ("username", VariableType::String),
            ("password", VariableType::String),
            ("schema", VariableType::String),
            ("source_table", VariableType::String),
            ("table_name", VariableType::String),
            ("ssl_mode", VariableType::String),
            ("readonly", VariableType::Boolean),
        ];

        for (pin_name, expected_type) in required_pins {
            let pin = input_pins.iter().find(|p| p.name == pin_name);
            assert!(pin.is_some(), "Missing input pin: {}", pin_name);
            assert_eq!(
                pin.unwrap().data_type,
                expected_type,
                "Wrong type for pin: {}",
                pin_name
            );
        }
    }

    #[test]
    fn test_register_postgres_node_output_pins() {
        let node_logic = RegisterPostgresNode::new();
        let node = node_logic.get_node();
        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        assert!(
            output_pins
                .iter()
                .any(|p| p.name == "exec_out" && p.data_type == VariableType::Execution)
        );
        assert!(
            output_pins
                .iter()
                .any(|p| p.name == "session_out" && p.data_type == VariableType::Struct)
        );
        assert!(
            output_pins
                .iter()
                .any(|p| p.name == "connection_url" && p.data_type == VariableType::String)
        );
    }

    #[test]
    fn test_register_mysql_node_structure() {
        let node_logic = RegisterMysqlNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_mysql");
        assert_eq!(node.friendly_name, "Register MySQL");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_sqlite_node_structure() {
        let node_logic = RegisterSqliteNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_sqlite");
        assert_eq!(node.friendly_name, "Register SQLite");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_duckdb_node_structure() {
        let node_logic = RegisterDuckdbNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_duckdb");
        assert_eq!(node.friendly_name, "Register DuckDB");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_clickhouse_node_structure() {
        let node_logic = RegisterClickhouseNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_clickhouse");
        assert_eq!(node.friendly_name, "Register ClickHouse");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_oracle_node_structure() {
        let node_logic = RegisterOracleNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_oracle");
        assert_eq!(node.friendly_name, "Register Oracle");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_register_flightsql_node_structure() {
        let node_logic = RegisterFlightSqlNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_flightsql");
        assert_eq!(node.friendly_name, "Register FlightSQL");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_all_database_nodes_have_session_pins() {
        let nodes: Vec<Box<dyn NodeLogic>> = vec![
            Box::new(RegisterPostgresNode::new()),
            Box::new(RegisterMysqlNode::new()),
            Box::new(RegisterSqliteNode::new()),
            Box::new(RegisterDuckdbNode::new()),
            Box::new(RegisterClickhouseNode::new()),
            Box::new(RegisterOracleNode::new()),
            Box::new(RegisterFlightSqlNode::new()),
            Box::new(RegisterOdbcNode::new()),
        ];

        for node_logic in nodes {
            let node = node_logic.get_node();
            let input_pins: Vec<_> = node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Input)
                .collect();
            let output_pins: Vec<_> = node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Output)
                .collect();

            assert!(
                input_pins
                    .iter()
                    .any(|p| p.name == "session" && p.data_type == VariableType::Struct),
                "Node {} missing session input pin",
                node.name
            );
            assert!(
                output_pins
                    .iter()
                    .any(|p| p.name == "session_out" && p.data_type == VariableType::Struct),
                "Node {} missing session_out output pin",
                node.name
            );
        }
    }

    #[test]
    fn test_all_database_nodes_have_execution_pins() {
        let nodes: Vec<Box<dyn NodeLogic>> = vec![
            Box::new(RegisterPostgresNode::new()),
            Box::new(RegisterMysqlNode::new()),
            Box::new(RegisterSqliteNode::new()),
            Box::new(RegisterDuckdbNode::new()),
            Box::new(RegisterClickhouseNode::new()),
            Box::new(RegisterOracleNode::new()),
            Box::new(RegisterFlightSqlNode::new()),
            Box::new(RegisterOdbcNode::new()),
        ];

        for node_logic in nodes {
            let node = node_logic.get_node();
            let input_pins: Vec<_> = node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Input)
                .collect();
            let output_pins: Vec<_> = node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Output)
                .collect();

            assert!(
                input_pins
                    .iter()
                    .any(|p| p.name == "exec_in" && p.data_type == VariableType::Execution),
                "Node {} missing exec_in pin",
                node.name
            );
            assert!(
                output_pins
                    .iter()
                    .any(|p| p.name == "exec_out" && p.data_type == VariableType::Execution),
                "Node {} missing exec_out pin",
                node.name
            );
        }
    }

    #[test]
    fn test_all_database_nodes_have_scores() {
        let nodes: Vec<Box<dyn NodeLogic>> = vec![
            Box::new(RegisterPostgresNode::new()),
            Box::new(RegisterMysqlNode::new()),
            Box::new(RegisterSqliteNode::new()),
            Box::new(RegisterDuckdbNode::new()),
            Box::new(RegisterClickhouseNode::new()),
            Box::new(RegisterOracleNode::new()),
            Box::new(RegisterFlightSqlNode::new()),
            Box::new(RegisterOdbcNode::new()),
        ];

        for node_logic in nodes {
            let node = node_logic.get_node();
            assert!(node.scores.is_some(), "Node {} missing scores", node.name);
        }
    }

    #[test]
    fn test_all_database_nodes_in_correct_category() {
        let nodes: Vec<Box<dyn NodeLogic>> = vec![
            Box::new(RegisterPostgresNode::new()),
            Box::new(RegisterMysqlNode::new()),
            Box::new(RegisterSqliteNode::new()),
            Box::new(RegisterDuckdbNode::new()),
            Box::new(RegisterClickhouseNode::new()),
            Box::new(RegisterOracleNode::new()),
            Box::new(RegisterFlightSqlNode::new()),
            Box::new(RegisterOdbcNode::new()),
        ];

        for node_logic in nodes {
            let node = node_logic.get_node();
            assert_eq!(
                node.category, "Data/DataFusion/Databases",
                "Node {} has wrong category",
                node.name
            );
        }
    }

    #[test]
    fn test_flightsql_has_query_and_tls_pins() {
        let node_logic = RegisterFlightSqlNode::new();
        let node = node_logic.get_node();
        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(
            input_pins.iter().any(|p| p.name == "query"),
            "FlightSQL should have query pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "use_tls"),
            "FlightSQL should have use_tls pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "skip_verify"),
            "FlightSQL should have skip_verify pin"
        );
    }

    #[test]
    fn test_oracle_has_odbc_driver_pin() {
        let node_logic = RegisterOracleNode::new();
        let node = node_logic.get_node();
        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(
            input_pins.iter().any(|p| p.name == "odbc_driver"),
            "Oracle should have odbc_driver pin for specifying the ODBC driver"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "service_name"),
            "Oracle should have service_name pin instead of database"
        );
    }

    #[test]
    fn test_register_odbc_node_structure() {
        let node = RegisterOdbcNode::new().get_node();
        assert_eq!(node.name, "df_register_odbc");
        let inputs: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .map(|p| p.name.as_str())
            .collect();
        for pin in [
            "connection_string",
            "username",
            "password",
            "catalog",
            "schema",
            "source_table",
            "table_name",
            "max_text_size",
        ] {
            assert!(inputs.contains(&pin), "Register ODBC Table lacks '{pin}'");
        }
    }

    #[cfg(feature = "odbc")]
    #[test]
    fn odbc_attribute_braces_values_that_would_split_the_string() {
        assert_eq!(odbc_attribute("PWD", "plain"), "PWD=plain");
        assert_eq!(odbc_attribute("PWD", "a;b"), "PWD={a;b}");
        assert_eq!(odbc_attribute("PWD", "x}y"), "PWD={x}}y}");
        assert_eq!(odbc_attribute("UID", " padded "), "UID={ padded }");
    }

    #[cfg(all(feature = "execute", feature = "odbc"))]
    fn odbc_source(catalog: &str, schema: &str, password: &str) -> OdbcSource {
        OdbcSource {
            connection_string: "Driver={Snowflake};Server=acme.snowflakecomputing.com; ".into(),
            username: "flow".into(),
            password: password.into(),
            catalog: catalog.into(),
            schema: schema.into(),
            source_table: "Orders".into(),
            table_name: "orders".into(),
            max_text_size: 65536,
        }
    }

    #[cfg(all(feature = "execute", feature = "odbc"))]
    #[test]
    fn odbc_source_appends_credentials_and_keeps_identifier_case() {
        let source = odbc_source("SALES", "PUBLIC", "p;w");
        assert_eq!(
            source.connection_string(),
            "Driver={Snowflake};Server=acme.snowflakecomputing.com;UID=flow;PWD={p;w}"
        );
        let table = source.table_reference();
        assert_eq!(
            (table.catalog(), table.schema(), table.table()),
            (Some("SALES"), Some("PUBLIC"), "Orders")
        );

        let bare = odbc_source("", "", "");
        assert!(!bare.connection_string().contains("PWD"));
        assert_eq!(bare.table_reference().schema(), None);
        assert_eq!(
            odbc_source("", "PUBLIC", "").table_reference().schema(),
            Some("PUBLIC")
        );
    }

    #[test]
    fn test_register_athena_node_structure() {
        let node_logic = RegisterAthenaNode::new();
        let node = node_logic.get_node();
        assert_eq!(node.name, "df_register_athena");
        assert_eq!(node.friendly_name, "Register AWS Athena");
        assert_eq!(node.category, "Data/DataFusion/Databases");
    }

    #[test]
    fn test_athena_has_aws_pins() {
        let node_logic = RegisterAthenaNode::new();
        let node = node_logic.get_node();
        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();

        assert!(
            input_pins.iter().any(|p| p.name == "region"),
            "Athena should have AWS region pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "access_key_id"),
            "Athena should have access_key_id pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "secret_access_key"),
            "Athena should have secret_access_key pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "s3_output_location"),
            "Athena should have s3_output_location pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "catalog"),
            "Athena should have catalog pin"
        );
        assert!(
            input_pins.iter().any(|p| p.name == "workgroup"),
            "Athena should have workgroup pin"
        );
    }
}
