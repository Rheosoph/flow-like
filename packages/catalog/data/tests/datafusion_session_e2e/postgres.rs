use crate::fixture::{ansi_boolean, ansi_timestamp, values_sql};
use crate::harness::unique;
use crate::scenarios::Backend;
use flow_like_catalog_data::data::datafusion::databases::RegisterPostgresNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use std::sync::Arc;
use tokio_postgres::NoTls;

/// A server speaking the PostgreSQL wire protocol, mounted through `df_register_postgres`.
pub struct PostgresWire {
    pub(crate) label: &'static str,
    pub(crate) port: u16,
    pub(crate) database: &'static str,
    pub(crate) user: &'static str,
    pub(crate) password: &'static str,
}

pub fn postgres() -> PostgresWire {
    PostgresWire {
        label: "PostgreSQL",
        port: 54320,
        database: "flowlike_test",
        user: "flowlike",
        password: "flowlike_test",
    }
}

/// CockroachDB has no DataFusion provider of its own; users reach it through the
/// PostgreSQL node, so that is what this exercises.
pub fn cockroach() -> PostgresWire {
    PostgresWire {
        label: "CockroachDB",
        port: 26270,
        database: "defaultdb",
        user: "root",
        password: "",
    }
}

impl PostgresWire {
    async fn client(&self) -> tokio_postgres::Client {
        let mut config = format!(
            "host=localhost port={} user={} dbname={}",
            self.port, self.user, self.database
        );
        if !self.password.is_empty() {
            config.push_str(&format!(" password={}", self.password));
        }
        let (client, connection) = tokio_postgres::connect(&config, NoTls)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{} on localhost:{} is unreachable ({error}); start it with \
                     `docker compose -f tests/docker-compose.test.yml up -d --wait`",
                    self.label, self.port
                )
            });
        tokio::spawn(connection);
        client
    }
}

#[async_trait]
impl Backend for PostgresWire {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterPostgresNode::new())
    }

    fn register_inputs(&self, source: &str, table_name: &str, readonly: bool) -> Value {
        json!({
            "host": "localhost",
            "port": self.port,
            "database": self.database,
            "username": self.user,
            "password": self.password,
            "schema": "public",
            "source_table": source,
            "table_name": table_name,
            "ssl_mode": "disable",
            "readonly": readonly,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["username"] = json!(unique("nobody"));
        inputs["password"] = json!("wrong-password");
        inputs
    }

    fn writable(&self) -> bool {
        true
    }

    async fn create_source(&self) -> String {
        let table = unique("people");
        self.client()
            .await
            .batch_execute(&format!(
                "CREATE TABLE {table} (
                    id BIGINT PRIMARY KEY,
                    name TEXT NOT NULL,
                    score DOUBLE PRECISION NOT NULL,
                    active BOOLEAN NOT NULL,
                    created_at TIMESTAMP NOT NULL
                );
                INSERT INTO {table} VALUES {};",
                values_sql(ansi_boolean, ansi_timestamp)
            ))
            .await
            .unwrap_or_else(|error| panic!("seeding {} table {table}: {error}", self.label));
        table
    }

    async fn count_rows(&self, source: &str) -> i64 {
        self.client()
            .await
            .query_one(&format!("SELECT COUNT(*) FROM {source}"), &[])
            .await
            .unwrap_or_else(|error| panic!("counting {} table {source}: {error}", self.label))
            .get(0)
    }

    async fn drop_source(&self, source: &str) {
        let _ = self
            .client()
            .await
            .batch_execute(&format!("DROP TABLE IF EXISTS {source}"))
            .await;
    }
}

mod postgres_suite {
    use super::postgres;
    crate::backend_suite!(
        postgres(),
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait postgres`"
    );
}

mod cockroach_suite {
    use super::cockroach;
    crate::backend_suite!(
        cockroach(),
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait cockroachdb`"
    );
}
