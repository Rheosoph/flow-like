use crate::postgres::{PostgresWire, cockroach, postgres};
use crate::scenarios::Backend;
use flow_like_catalog_data::data::datafusion::databases::RegisterOdbcNode;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_types::async_trait;
use flow_like_types::{Value, json::json};
use std::path::Path;
use std::sync::Arc;

/// `df_register_odbc` reaching a PostgreSQL-wire server through the PostgreSQL ODBC
/// driver, which stands in for vendor drivers (Snowflake, Oracle, Athena) that need
/// accounts. The fixture data is seeded and counted over the native protocol.
pub struct OdbcOverPostgres {
    server: PostgresWire,
    driver: String,
}

const DRIVER_CANDIDATES: [&str; 4] = [
    "/opt/homebrew/lib/psqlodbcw.so",
    "/usr/local/lib/psqlodbcw.so",
    "/usr/lib/x86_64-linux-gnu/odbc/psqlodbcw.so",
    "/usr/lib/aarch64-linux-gnu/odbc/psqlodbcw.so",
];

fn psqlodbc_driver() -> String {
    std::env::var("FLOW_LIKE_E2E_PSQLODBC")
        .ok()
        .or_else(|| {
            DRIVER_CANDIDATES
                .iter()
                .find(|path| Path::new(path).exists())
                .map(|path| path.to_string())
        })
        .unwrap_or_else(|| {
            panic!(
                "PostgreSQL ODBC driver not found in {DRIVER_CANDIDATES:?}; install it \
                 (`brew install psqlodbc` / `apt install odbc-postgresql`) or set \
                 FLOW_LIKE_E2E_PSQLODBC to its path"
            )
        })
}

fn over(server: PostgresWire) -> OdbcOverPostgres {
    OdbcOverPostgres {
        server,
        driver: psqlodbc_driver(),
    }
}

#[async_trait]
impl Backend for OdbcOverPostgres {
    fn register_node(&self) -> Arc<dyn NodeLogic> {
        Arc::new(RegisterOdbcNode::new())
    }

    fn register_inputs(&self, source: &str, table_name: &str, _readonly: bool) -> Value {
        json!({
            "connection_string": format!(
                "Driver={{{}}};Server=localhost;Port={};Database={};SSLmode=disable;BoolsAsChar=0",
                self.driver, self.server.port, self.server.database
            ),
            "username": self.server.user,
            "password": self.server.password,
            "schema": "public",
            "source_table": source,
            "table_name": table_name,
        })
    }

    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value {
        let mut inputs = self.register_inputs(source, table_name, true);
        inputs["username"] = json!(crate::harness::unique("nobody"));
        inputs["password"] = json!("wrong-password");
        inputs
    }

    async fn create_source(&self) -> String {
        self.server.create_source().await
    }

    async fn count_rows(&self, source: &str) -> i64 {
        self.server.count_rows(source).await
    }

    async fn drop_source(&self, source: &str) {
        self.server.drop_source(source).await;
    }
}

mod odbc_postgres_suite {
    use super::{over, postgres};
    crate::backend_suite!(
        over(postgres()),
        ignore = "needs unixODBC + psqlodbc and `docker compose -f tests/docker-compose.test.yml up -d --wait postgres`"
    );
}

mod odbc_cockroach_suite {
    use super::{cockroach, over};
    crate::backend_suite!(
        over(cockroach()),
        ignore = "needs unixODBC + psqlodbc and `docker compose -f tests/docker-compose.test.yml up -d --wait cockroachdb`"
    );
}
