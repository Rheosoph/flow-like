use crate::fixture::{PEOPLE, ansi_boolean, ansi_timestamp, values_sql};
use crate::harness::{Flow, unique};
use crate::scenarios::{Backend, register};
use flow_like_catalog_data::data::datafusion::databases::RegisterFlightSqlNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use std::sync::Arc;

/// GizmoSQL (DuckDB behind Arrow Flight SQL). The node registers a query, not a
/// table, so a "source" here is the SQL that produces the fixture rows.
pub struct FlightSql;

#[async_trait]
impl Backend for FlightSql {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterFlightSqlNode::new())
    }

    fn register_inputs(&self, source: &str, table_name: &str, _readonly: bool) -> Value {
        json!({
            "host": "localhost",
            "port": 31337,
            "username": "flowlike",
            "password": "flowlike_test",
            "query": source,
            "table_name": table_name,
            "use_tls": false,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["password"] = json!("wrong-password");
        inputs
    }

    /// The remote SQL is the registered query itself; later filters run in DataFusion.
    fn pushes_down(&self) -> bool {
        false
    }

    fn missing_source(&self) -> String {
        format!("SELECT * FROM {}", unique("missing"))
    }

    async fn create_source(&self) -> String {
        format!(
            "SELECT * FROM (VALUES {}) AS people(id, name, score, active, created_at)",
            values_sql(ansi_boolean, ansi_timestamp)
        )
    }

    async fn count_rows(&self, _source: &str) -> i64 {
        PEOPLE.len() as i64
    }

    async fn drop_source(&self, _source: &str) {}
}

mod flight_suite {
    use super::FlightSql;
    crate::backend_suite!(
        FlightSql,
        ignore = "needs `docker compose -f tests/docker-compose.test.yml up -d --wait gizmosql`"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn server_execution_refuses_loopback_flight_sql() {
    let flow = Flow::server();
    let session = flow.create_session(&unique("e2e")).await;
    for host in ["localhost", "127.0.0.1"] {
        let mut inputs = FlightSql.register_inputs("SELECT 1", "probe", true);
        inputs["host"] = json!(host);
        let error = register(&flow, &FlightSql, &session, inputs)
            .await
            .expect_failed("server-side Flight SQL registration against loopback")
            .await;
        assert!(error.contains("refused"), "unexpected error: {error}");
    }
}
