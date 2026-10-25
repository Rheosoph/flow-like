mod shared;

use std::sync::Arc;

use datafusion::{
    error::Result,
    execution::{context::SessionContext, options::CsvReadOptions},
};
use datafusion_federation::sql::{SQLFederationProvider, SQLSchemaProvider};

const CSV_PATH: &str = "./examples/data/test.csv";
const TABLE_NAME: &str = "test";

use shared::{overwrite_default_schema, MockSqliteExecutor};

#[tokio::main]
async fn main() -> Result<()> {
    // Create a remote context for the mock sqlite DB
    let remote_ctx = Arc::new(SessionContext::new());

    // Registers a CSV file
    remote_ctx
        .register_csv(TABLE_NAME, CSV_PATH, CsvReadOptions::new())
        .await?;
    let known_tables: Vec<String> = [TABLE_NAME].iter().map(|&x| x.into()).collect();

    // Create the federation provider
    let executor = Arc::new(MockSqliteExecutor::new(remote_ctx));
    let provider = Arc::new(SQLFederationProvider::new(executor));

    // Get the schema
    let schema_provider =
        Arc::new(SQLSchemaProvider::new_with_tables(provider, known_tables).await?);

    // Main context
    let state = datafusion_federation::default_session_state();
    overwrite_default_schema(&state, schema_provider)?;
    let ctx = SessionContext::new_with_state(state);

    // Run a query
    let query = r#"SELECT * FROM test"#;
    let df = ctx.sql(query).await?;

    df.show().await
}
