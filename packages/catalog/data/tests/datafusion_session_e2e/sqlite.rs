use crate::fixture::{numeric_boolean, quoted_timestamp, values_sql};
use crate::harness::unique;
use crate::scenarios::{Backend, Surface};
use flow_like_catalog_data::data::datafusion::databases::RegisterSqliteNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use sqlx::{Executor, SqlitePool};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

pub struct Sqlite {
    directory: TempDir,
}

pub fn sqlite() -> Sqlite {
    Sqlite {
        directory: tempfile::tempdir().expect("temporary directory for SQLite"),
    }
}

impl Sqlite {
    pub fn path(&self) -> PathBuf {
        self.directory.path().join("people.db")
    }

    async fn pool(&self) -> SqlitePool {
        SqlitePool::connect(&format!("sqlite://{}?mode=rwc", self.path().display()))
            .await
            .expect("opening the SQLite fixture file")
    }
}

#[async_trait]
impl Backend for Sqlite {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterSqliteNode::new())
    }

    fn surface(&self) -> Surface {
        Surface {
            boolean_as_integer: true,
            timestamp_as_text: true,
        }
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
        inputs["file_path"] = json!(self.directory.path().join("missing.db"));
        inputs
    }

    async fn create_source(&self) -> String {
        let table = unique("people");
        let pool = self.pool().await;
        pool.execute(
            format!(
                "CREATE TABLE {table} (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    score REAL NOT NULL,
                    active BOOLEAN NOT NULL,
                    created_at TIMESTAMP NOT NULL
                );
                INSERT INTO {table} VALUES {};",
                values_sql(numeric_boolean, quoted_timestamp)
            )
            .as_str(),
        )
        .await
        .unwrap_or_else(|error| panic!("seeding SQLite table {table}: {error}"));
        pool.close().await;
        table
    }

    async fn count_rows(&self, source: &str) -> i64 {
        let pool = self.pool().await;
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {source}"))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| panic!("counting SQLite table {source}: {error}"));
        pool.close().await;
        count
    }

    async fn drop_source(&self, _source: &str) {}
}

mod sqlite_suite {
    use super::sqlite;
    crate::backend_suite!(sqlite());
}
