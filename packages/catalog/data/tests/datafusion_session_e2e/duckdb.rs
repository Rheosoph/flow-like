use crate::fixture::{ansi_boolean, ansi_timestamp, values_sql};
use crate::harness::unique;
use crate::scenarios::Backend;
use datafusion_table_providers::sql::db_connection_pool::{
    DbConnectionPool, duckdbpool::DuckDbConnectionPoolBuilder,
};
use flow_like_catalog_data::data::datafusion::databases::RegisterDuckdbNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_storage::datafusion::arrow::array::as_primitive_array;
use flow_like_storage::datafusion::arrow::datatypes::Int64Type;
use flow_like_storage::datafusion::physical_plan::common::collect;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

pub struct Duckdb {
    directory: TempDir,
}

pub fn duckdb() -> Duckdb {
    Duckdb {
        directory: tempfile::tempdir().expect("temporary directory for DuckDB"),
    }
}

impl Duckdb {
    fn path(&self) -> PathBuf {
        self.directory.path().join("people.duckdb")
    }

    async fn execute(&self, sql: &str) {
        let path = self.path();
        let pool = DuckDbConnectionPoolBuilder::file(path.to_str().unwrap())
            .build()
            .unwrap_or_else(|error| panic!("opening DuckDB file {}: {error}", path.display()));
        let connection = pool
            .connect()
            .await
            .unwrap_or_else(|error| panic!("connecting to DuckDB: {error}"));
        connection
            .as_sync()
            .expect("DuckDB exposes synchronous connections")
            .execute(sql, &[])
            .unwrap_or_else(|error| panic!("DuckDB rejected `{sql}`: {error}"));
    }

    async fn scalar(&self, sql: &str) -> i64 {
        let path = self.path();
        let pool = DuckDbConnectionPoolBuilder::file(path.to_str().unwrap())
            .build()
            .unwrap_or_else(|error| panic!("opening DuckDB file {}: {error}", path.display()));
        let connection = pool
            .connect()
            .await
            .unwrap_or_else(|error| panic!("connecting to DuckDB: {error}"));
        let stream = connection
            .as_sync()
            .expect("DuckDB exposes synchronous connections")
            .query_arrow(sql, &[], None)
            .unwrap_or_else(|error| panic!("DuckDB rejected `{sql}`: {error}"));
        let batches = collect(stream)
            .await
            .unwrap_or_else(|error| panic!("reading `{sql}`: {error}"));
        as_primitive_array::<Int64Type>(batches[0].column(0)).value(0)
    }
}

#[async_trait]
impl Backend for Duckdb {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterDuckdbNode::new())
    }

    fn register_inputs(&self, source: &str, table_name: &str, readonly: bool) -> Value {
        json!({
            "file_path": self.path(),
            "source_table": source,
            "table_name": table_name,
            "readonly": readonly,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["file_path"] = json!(self.directory.path().join("missing.duckdb"));
        inputs
    }

    fn writable(&self) -> bool {
        true
    }

    async fn create_source(&self) -> String {
        let table = unique("people");
        self.execute(&format!(
            "CREATE TABLE {table} (
                id BIGINT PRIMARY KEY,
                name VARCHAR NOT NULL,
                score DOUBLE NOT NULL,
                active BOOLEAN NOT NULL,
                created_at TIMESTAMP NOT NULL
            );
            INSERT INTO {table} VALUES {};",
            values_sql(ansi_boolean, ansi_timestamp)
        ))
        .await;
        table
    }

    async fn count_rows(&self, source: &str) -> i64 {
        self.scalar(&format!("SELECT COUNT(*) FROM {source}")).await
    }

    async fn drop_source(&self, _source: &str) {}
}

mod duckdb_suite {
    use super::duckdb;
    crate::backend_suite!(duckdb());
}
