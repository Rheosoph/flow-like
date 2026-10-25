//! End-to-end tests of DataFusion session handling against real databases.
//!
//! Every test drives the catalog nodes themselves — Create Session, the Register nodes,
//! SQL Query, List Tables — through one shared execution cache, exactly as a flow does.
//! Each provider is only compiled in with its feature, so enable the ones to test:
//!
//! ```sh
//! FEATURES=execute,postgres-federation,mysql-federation,sqlite-federation,duckdb-federation,clickhouse-federation,flight,odbc-federation
//! cargo test -p flow-like-catalog-data --features $FEATURES --test datafusion_session_e2e
//! ```
//!
//! SQLite, DuckDB and Lance run in-process. Server databases are `#[ignore]`d and need
//! the containers in `tests/docker-compose.test.yml`; `mise run test:datafusion:e2e`
//! starts them and runs everything with `--include-ignored`. The ODBC feature links the
//! unixODBC driver manager, and the ODBC suites use the PostgreSQL ODBC driver
//! (`brew install unixodbc psqlodbc`, or `apt install unixodbc-dev odbc-postgresql`).

mod fixture;
mod harness;
mod lance;
mod scenarios;

#[cfg(any(
    all(feature = "sqlite", feature = "duckdb"),
    all(feature = "postgres", feature = "mysql", feature = "clickhouse")
))]
mod mixed;

#[cfg(feature = "clickhouse")]
mod clickhouse;
#[cfg(feature = "duckdb")]
mod duckdb;
#[cfg(feature = "flight")]
mod flight;
#[cfg(feature = "mysql")]
mod mysql;
#[cfg(all(feature = "odbc", feature = "postgres"))]
mod odbc;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "sqlite")]
mod sqlite;
