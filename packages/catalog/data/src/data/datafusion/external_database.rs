//! One connection pool per external database, shared by every table a query reads there, and
//! the tables it holds. The egress checks of `connect_table` run once, before the pool opens.
//! BigQuery and Athena keep one authenticated client per source instead of a pool.

#[cfg(feature = "athena")]
use super::athena_table::AthenaDatabase;
#[cfg(feature = "bigquery")]
use super::bigquery_table::BigqueryDatabase;
use super::connectors::{ConnectPolicy, ConnectorSource, Provider, ensure_connection_allowed};
#[cfg(feature = "clickhouse")]
use datafusion_table_providers::sql::db_connection_pool::clickhousepool::ClickHouseConnectionPool;
#[cfg(feature = "mysql")]
use datafusion_table_providers::sql::db_connection_pool::mysqlpool::MySQLConnectionPool;
#[cfg(feature = "postgres")]
use datafusion_table_providers::sql::db_connection_pool::postgrespool::PostgresConnectionPool;
use flow_like_types::{Result, anyhow};
#[cfg(any(
    feature = "postgres",
    feature = "mysql",
    feature = "clickhouse",
    feature = "bigquery",
    feature = "athena"
))]
use std::sync::Arc;

/// A pool on one external database; the providers it builds share its connections.
pub struct ExternalDatabase {
    pool: Pool,
}

enum Pool {
    #[cfg(feature = "postgres")]
    Postgres {
        pool: Arc<PostgresConnectionPool>,
        database: String,
        schema: String,
    },
    #[cfg(feature = "mysql")]
    Mysql {
        pool: Arc<MySQLConnectionPool>,
        database: String,
    },
    #[cfg(feature = "clickhouse")]
    Clickhouse {
        pool: Arc<ClickHouseConnectionPool>,
        database: String,
    },
    #[cfg(feature = "bigquery")]
    Bigquery(Arc<BigqueryDatabase>),
    #[cfg(feature = "athena")]
    Athena(Arc<AthenaDatabase>),
}

/// Opens a pool on the database `source` names once the egress checks of `policy` pass. The
/// source's table is not used: [`ExternalDatabase::table`] names one per provider.
pub async fn connect_database(
    source: &ConnectorSource,
    policy: impl Into<ConnectPolicy>,
) -> Result<ExternalDatabase> {
    let policy = policy.into();
    let pool = match source {
        ConnectorSource::Postgres(source) => {
            ensure_connection_allowed(policy, "PostgreSQL", &source.host, source.port).await?;
            open_postgres(source).await?
        }
        ConnectorSource::Mysql(source) => {
            ensure_connection_allowed(policy, "MySQL", &source.host, source.port).await?;
            open_mysql(source).await?
        }
        ConnectorSource::Clickhouse(source) => {
            ensure_connection_allowed(policy, "ClickHouse", &source.host, source.port).await?;
            open_clickhouse(source).await?
        }
        #[cfg(feature = "bigquery")]
        ConnectorSource::Bigquery(source) => {
            Pool::Bigquery(BigqueryDatabase::open(source, policy).await?)
        }
        #[cfg(not(feature = "bigquery"))]
        ConnectorSource::Bigquery(_) => {
            return Err(super::connectors::not_enabled("BigQuery", "bigquery"));
        }
        #[cfg(feature = "athena")]
        ConnectorSource::Athena(source) => {
            Pool::Athena(AthenaDatabase::open(source, policy).await?)
        }
        #[cfg(not(feature = "athena"))]
        ConnectorSource::Athena(_) => {
            return Err(super::connectors::not_enabled("Athena", "athena"));
        }
        _ => {
            return Err(anyhow!(
                "Only PostgreSQL, MySQL, ClickHouse, BigQuery and Athena databases open a shared connection"
            ));
        }
    };
    Ok(ExternalDatabase { pool })
}

#[cfg(feature = "postgres")]
async fn open_postgres(source: &super::connectors::PostgresSource) -> Result<Pool> {
    Ok(Pool::Postgres {
        pool: super::connectors::postgres_pool(source).await?,
        database: source.database.clone(),
        schema: source.schema.clone(),
    })
}

#[cfg(not(feature = "postgres"))]
async fn open_postgres(_source: &super::connectors::PostgresSource) -> Result<Pool> {
    Err(super::connectors::not_enabled("PostgreSQL", "postgres"))
}

#[cfg(feature = "mysql")]
async fn open_mysql(source: &super::connectors::MysqlSource) -> Result<Pool> {
    Ok(Pool::Mysql {
        pool: super::connectors::mysql_pool(source).await?,
        database: source.database.clone(),
    })
}

#[cfg(not(feature = "mysql"))]
async fn open_mysql(_source: &super::connectors::MysqlSource) -> Result<Pool> {
    Err(super::connectors::not_enabled("MySQL", "mysql"))
}

#[cfg(feature = "clickhouse")]
async fn open_clickhouse(source: &super::connectors::ClickhouseSource) -> Result<Pool> {
    Ok(Pool::Clickhouse {
        pool: super::connectors::clickhouse_pool(source).await?,
        database: source.database.clone(),
    })
}

#[cfg(not(feature = "clickhouse"))]
async fn open_clickhouse(_source: &super::connectors::ClickhouseSource) -> Result<Pool> {
    Err(super::connectors::not_enabled("ClickHouse", "clickhouse"))
}

#[cfg(any(
    feature = "postgres",
    feature = "mysql",
    feature = "clickhouse",
    feature = "bigquery",
    feature = "athena"
))]
impl ExternalDatabase {
    /// A read-only provider for `table`, on the shared pool.
    pub async fn table(&self, table: &str) -> Result<Provider> {
        match &self.pool {
            #[cfg(feature = "postgres")]
            Pool::Postgres {
                pool,
                database,
                schema,
            } => {
                super::connectors::postgres_table(pool.clone(), database, schema, table, true).await
            }
            #[cfg(feature = "mysql")]
            Pool::Mysql { pool, .. } => {
                super::connectors::mysql_table(pool.clone(), table, true).await
            }
            #[cfg(feature = "clickhouse")]
            Pool::Clickhouse { pool, database } => {
                super::connectors::clickhouse_table(pool.clone(), database, table).await
            }
            #[cfg(feature = "bigquery")]
            Pool::Bigquery(database) => database.table(table).await,
            #[cfg(feature = "athena")]
            Pool::Athena(database) => database.table(table).await,
        }
    }

    /// The tables and views of the PostgreSQL schema, of the MySQL, ClickHouse or Athena
    /// database, or of the BigQuery dataset the source names, sorted.
    pub async fn tables(&self) -> Result<Vec<String>> {
        let mut tables = match &self.pool {
            #[cfg(feature = "postgres")]
            Pool::Postgres { pool, schema, .. } => postgres_tables(pool, schema).await?,
            #[cfg(feature = "mysql")]
            Pool::Mysql { pool, database } => listed_tables(&**pool, "MySQL", database).await?,
            #[cfg(feature = "clickhouse")]
            Pool::Clickhouse { pool, database } => listed_tables(&**pool, "ClickHouse", database)
                .await?
                .into_iter()
                .filter(|name| !name.starts_with(".inner"))
                .collect(),
            #[cfg(feature = "bigquery")]
            Pool::Bigquery(database) => database.tables().await?,
            #[cfg(feature = "athena")]
            Pool::Athena(database) => database.tables().await?,
        };
        tables.sort();
        tables.dedup();
        Ok(tables)
    }
}

#[cfg(not(any(
    feature = "postgres",
    feature = "mysql",
    feature = "clickhouse",
    feature = "bigquery",
    feature = "athena"
)))]
impl ExternalDatabase {
    pub async fn table(&self, _table: &str) -> Result<Provider> {
        match self.pool {}
    }

    pub async fn tables(&self) -> Result<Vec<String>> {
        match self.pool {}
    }
}

#[cfg(any(feature = "mysql", feature = "clickhouse"))]
async fn listed_tables<Connections, T, P>(
    pool: &Connections,
    driver: &str,
    database: &str,
) -> Result<Vec<String>>
where
    Connections:
        datafusion_table_providers::sql::db_connection_pool::DbConnectionPool<T, P> + Sync + ?Sized,
    T: 'static,
    P: 'static,
{
    use datafusion_table_providers::sql::db_connection_pool::dbconnection::get_tables;

    let connection = pool
        .connect()
        .await
        .map_err(|error| anyhow!("{driver} connection failed: {error}"))?;
    get_tables(connection, database).await.map_err(|error| {
        anyhow!("Listing the tables of {driver} database '{database}' failed: {error}")
    })
}

/// The driver's own listing reads `pg_tables`, which leaves out views; queries address views
/// like tables, so this reads `information_schema.tables`, which also lists only what the
/// connecting role may see.
#[cfg(feature = "postgres")]
async fn postgres_tables(pool: &PostgresConnectionPool, schema: &str) -> Result<Vec<String>> {
    use datafusion_table_providers::sql::db_connection_pool::DbConnectionPool;
    use flow_like_storage::arrow::compute::cast;
    use flow_like_storage::arrow::datatypes::DataType;
    use flow_like_storage::arrow_array::cast::AsArray;

    // The driver binds parameters only as 'static references, so the schema is a literal.
    if schema.contains(['\'', '\\']) || schema.chars().any(char::is_control) {
        return Err(anyhow!(
            "PostgreSQL schema '{schema}' cannot be listed: its name holds a quote, a backslash or a control character"
        ));
    }
    let connection = pool
        .connect()
        .await
        .map_err(|error| anyhow!("PostgreSQL connection failed: {error}"))?;
    let connection = connection
        .as_async()
        .ok_or_else(|| anyhow!("PostgreSQL connections cannot list tables"))?;
    let sql = format!(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = '{schema}'"
    );
    let stream = connection
        .query_arrow(&sql, &[], None)
        .await
        .map_err(|error| {
            anyhow!("Listing the tables of PostgreSQL schema '{schema}' failed: {error}")
        })?;
    let batches = flow_like_storage::datafusion::physical_plan::common::collect(stream).await?;
    let mut tables = Vec::new();
    for batch in batches {
        if batch.num_columns() == 0 {
            continue;
        }
        let names = cast(batch.column(0), &DataType::Utf8)?;
        tables.extend(
            names
                .as_string::<i32>()
                .iter()
                .flatten()
                .map(str::to_string),
        );
    }
    Ok(tables)
}
