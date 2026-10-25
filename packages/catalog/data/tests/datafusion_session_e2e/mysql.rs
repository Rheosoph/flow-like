use crate::fixture::{ansi_boolean, ansi_timestamp, values_sql};
use crate::harness::unique;
use crate::scenarios::{Backend, Surface};
use flow_like_catalog_data::data::datafusion::databases::RegisterMysqlNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use sqlx::{Executor, MySqlPool};
use std::sync::Arc;

/// A server speaking the MySQL protocol, mounted through `df_register_mysql`.
pub struct MysqlWire {
    label: &'static str,
    port: u16,
}

pub fn mysql() -> MysqlWire {
    MysqlWire {
        label: "MySQL",
        port: 33060,
    }
}

pub fn mariadb() -> MysqlWire {
    MysqlWire {
        label: "MariaDB",
        port: 33070,
    }
}

const DATABASE: &str = "flowlike_test";
const USER: &str = "flowlike";
const PASSWORD: &str = "flowlike_test";

impl MysqlWire {
    async fn pool(&self) -> MySqlPool {
        MySqlPool::connect(&format!(
            "mysql://{USER}:{PASSWORD}@localhost:{}/{DATABASE}",
            self.port
        ))
        .await
        .unwrap_or_else(|error| {
            panic!(
                "{} on localhost:{} is unreachable ({error}); start it with \
                 `docker compose -f tests/docker-compose.test.yml up -d --wait`",
                self.label, self.port
            )
        })
    }
}

#[async_trait]
impl Backend for MysqlWire {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterMysqlNode::new())
    }

    /// MySQL and MariaDB define BOOLEAN as TINYINT(1).
    fn surface(&self) -> Surface {
        Surface {
            boolean_as_integer: true,
            timestamp_as_text: false,
        }
    }

    fn register_inputs(&self, source: &str, table_name: &str, readonly: bool) -> Value {
        json!({
            "host": "localhost",
            "port": self.port,
            "database": DATABASE,
            "username": USER,
            "password": PASSWORD,
            "source_table": source,
            "table_name": table_name,
            "ssl_mode": "disabled",
            "readonly": readonly,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["password"] = json!("wrong-password");
        inputs
    }

    fn writable(&self) -> bool {
        true
    }

    async fn create_source(&self) -> String {
        let table = unique("people");
        let pool = self.pool().await;
        pool.execute(
            format!(
                "CREATE TABLE {table} (
                    id BIGINT PRIMARY KEY,
                    name VARCHAR(64) NOT NULL,
                    score DOUBLE NOT NULL,
                    active BOOLEAN NOT NULL,
                    created_at DATETIME NOT NULL
                ) CHARACTER SET utf8mb4"
            )
            .as_str(),
        )
        .await
        .unwrap_or_else(|error| panic!("creating {} table {table}: {error}", self.label));
        pool.execute(
            format!(
                "INSERT INTO {table} VALUES {}",
                values_sql(ansi_boolean, ansi_timestamp)
            )
            .as_str(),
        )
        .await
        .unwrap_or_else(|error| panic!("seeding {} table {table}: {error}", self.label));
        pool.close().await;
        table
    }

    async fn count_rows(&self, source: &str) -> i64 {
        let pool = self.pool().await;
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {source}"))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| panic!("counting {} table {source}: {error}", self.label));
        pool.close().await;
        count
    }

    async fn drop_source(&self, source: &str) {
        let pool = self.pool().await;
        let _ = pool
            .execute(format!("DROP TABLE IF EXISTS {source}").as_str())
            .await;
        pool.close().await;
    }
}

mod mysql_suite {
    use super::mysql;
    crate::backend_suite!(
        mysql(),
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait mysql`"
    );
}

mod mariadb_suite {
    use super::mariadb;
    crate::backend_suite!(
        mariadb(),
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait mariadb`"
    );
}
