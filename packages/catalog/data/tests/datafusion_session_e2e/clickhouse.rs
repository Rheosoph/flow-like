use crate::fixture::{ansi_boolean, quoted_timestamp, values_sql};
use crate::harness::{Flow, unique};
use crate::scenarios::{Backend, register};
use flow_like_catalog_data::data::datafusion::databases::RegisterClickhouseNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use std::sync::Arc;

const PORT: u16 = 8123;
const USER: &str = "flowlike";
const PASSWORD: &str = "flowlike_test";

pub struct Clickhouse;

impl Clickhouse {
    async fn execute(&self, sql: &str) -> String {
        let response = reqwest::Client::new()
            .post(format!("http://localhost:{PORT}/"))
            .header("X-ClickHouse-User", USER)
            .header("X-ClickHouse-Key", PASSWORD)
            .body(sql.to_string())
            .send()
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "ClickHouse on localhost:{PORT} is unreachable ({error}); start it with \
                     `docker compose -f tests/docker-compose.test.yml up -d --wait`"
                )
            });
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        assert!(status.is_success(), "ClickHouse rejected `{sql}`: {body}");
        body
    }
}

#[async_trait]
impl Backend for Clickhouse {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterClickhouseNode::new())
    }

    fn register_inputs(&self, source: &str, table_name: &str, readonly: bool) -> Value {
        json!({
            "host": "localhost",
            "port": PORT,
            "database": "default",
            "username": USER,
            "password": PASSWORD,
            "source_table": source,
            "table_name": table_name,
            "readonly": readonly,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["password"] = json!("wrong-password");
        inputs
    }

    async fn create_source(&self) -> String {
        let table = unique("people");
        self.execute(&format!(
            "CREATE TABLE default.{table} (
                id Int64,
                name String,
                score Float64,
                active Bool,
                created_at DateTime64(6)
            ) ENGINE = MergeTree ORDER BY id"
        ))
        .await;
        self.execute(&format!(
            "INSERT INTO default.{table} VALUES {}",
            values_sql(ansi_boolean, quoted_timestamp)
        ))
        .await;
        table
    }

    async fn count_rows(&self, source: &str) -> i64 {
        self.execute(&format!(
            "SELECT count() FROM default.{source} FORMAT TabSeparated"
        ))
        .await
        .trim()
        .parse()
        .expect("ClickHouse count() is an integer")
    }

    async fn drop_source(&self, source: &str) {
        self.execute(&format!("DROP TABLE IF EXISTS default.{source}"))
            .await;
    }
}

mod clickhouse_suite {
    use super::Clickhouse;
    crate::backend_suite!(
        Clickhouse,
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait clickhouse`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn server_execution_refuses_loopback_clickhouse() {
    let flow = Flow::server();
    let session = flow.create_session(&unique("e2e")).await;
    for host in ["localhost", "127.0.0.1"] {
        let mut inputs = Clickhouse.register_inputs("people", "people", true);
        inputs["host"] = json!(host);
        let error = register(&flow, &Clickhouse, &session, inputs)
            .await
            .expect_failed("server-side ClickHouse registration against loopback")
            .await;
        assert!(error.contains("refused"), "unexpected error: {error}");
    }
}
