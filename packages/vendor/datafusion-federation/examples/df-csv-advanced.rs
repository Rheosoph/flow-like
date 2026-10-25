mod shared;

use std::sync::Arc;

use datafusion::{
    execution::{
        context::SessionContext, options::CsvReadOptions, session_state::SessionStateBuilder,
    },
    optimizer::Optimizer,
};

use datafusion_federation::{
    sql::{MultiSchemaProvider, SQLFederationProvider, SQLSchemaProvider},
    FederatedQueryPlanner, FederationOptimizerRule,
};

use shared::{overwrite_default_schema, MockPostgresExecutor, MockSqliteExecutor};

const CSV_PATH_SQLITE: &str = "./examples/data/test.csv";
const CSV_PATH_POSTGRES: &str = "./examples/data/test2.csv";
const TABLE_NAME_SQLITE: &str = "test_sqlite";
const TABLE_NAME_POSTGRES: &str = "test_pg";

#[tokio::main]
async fn main() {
    // This example demonstrates how DataFusion, with federation enabled, to
    // executes a query using two execution engines.
    //
    // The query used in this example is:
    //
    // ```sql
    // SELECT t.*
    // FROM test_pg AS t
    // JOIN test_sqlite AS a
    // ON t.foo = a.foo
    // ```
    //
    // In this query, `test_pg` is a table in a PostgreSQL database, and `test_sqlite` is a table
    // in an SQLite database. DataFusion Federation will identify the sub-plans that can be
    // executed by external databases. In this example, there will be only two sub-plans.
    //
    //           ┌────────────┐
    //           │    Join    │
    //           └────────────┘
    //                  ▲
    //          ┌───────┴──────────┐
    //   ┌──────────────┐   ┌────────────┐
    //   │ test_sqlite  │   │    Join    │
    //   └──────────────┘   └────────────┘
    //                            ▲
    //                            |
    //                      ┌────────────┐
    //                      │   test_pg  │
    //                      └────────────┘
    //
    // Note: For the purpose of this example, both the SQLite and PostgreSQL engines are dummy
    // engines that use DataFusion SessionContexts with registered CSV files. However, this setup
    // works fine for demonstration purposes. If you'd like to use actual SQLite and PostgreSQL
    // engines, you can check out the table-providers repository at
    // https://github.com/datafusion-contrib/datafusion-table-providers/.

    /////////////////////
    // Remote sqlite DB
    /////////////////////
    // Create a datafusion::SessionContext and register a csv file as a table in that context
    // This will be passed to the MockSqliteExecutor and acts as a dummy sqlite engine.
    let sqlite_remote_ctx = Arc::new(SessionContext::new());
    // Registers a CSV file
    sqlite_remote_ctx
        .register_csv(TABLE_NAME_SQLITE, CSV_PATH_SQLITE, CsvReadOptions::new())
        .await
        .expect("Register csv file");

    let sqlite_known_tables: Vec<String> = [TABLE_NAME_SQLITE].iter().map(|&x| x.into()).collect();

    // Create the federation provider
    let sqlite_executor = Arc::new(MockSqliteExecutor::new(sqlite_remote_ctx));
    let sqlite_federation_provider = Arc::new(SQLFederationProvider::new(sqlite_executor));
    // Create the schema provider
    let sqlite_schema_provider = Arc::new(
        SQLSchemaProvider::new_with_tables(sqlite_federation_provider, sqlite_known_tables)
            .await
            .expect("Create new schema provider with tables"),
    );

    /////////////////////
    // Remote postgres DB
    /////////////////////
    // Create a datafusion::SessionContext and register a csv file as a table in that context
    // This will be passed to the MockPostgresExecutor and acts as a dummy postgres engine.
    let postgres_remote_ctx = Arc::new(SessionContext::new());
    // Registers a CSV file
    postgres_remote_ctx
        .register_csv(
            TABLE_NAME_POSTGRES,
            CSV_PATH_POSTGRES,
            CsvReadOptions::new(),
        )
        .await
        .expect("Register csv file");

    let postgres_known_tables: Vec<String> =
        [TABLE_NAME_POSTGRES].iter().map(|&x| x.into()).collect();

    // Create the federation provider
    let postgres_executor = Arc::new(MockPostgresExecutor::new(postgres_remote_ctx));
    let postgres_federation_provider = Arc::new(SQLFederationProvider::new(postgres_executor));
    // Create the schema provider
    let postgres_schema_provider = Arc::new(
        SQLSchemaProvider::new_with_tables(postgres_federation_provider, postgres_known_tables)
            .await
            .expect("Create new schema provider with tables"),
    );

    /////////////////////
    //  Main(local)  DB
    /////////////////////
    // Get the default optimizer rules
    let mut rules = Optimizer::new().rules;

    // Create a new federation optimizer rule and add it to the default rules
    rules.push(Arc::new(FederationOptimizerRule::new()));

    // Create a new SessionState with the optimizer rule we created above
    let state = SessionStateBuilder::new()
        .with_optimizer_rules(rules)
        .with_query_planner(Arc::new(FederatedQueryPlanner::new()))
        .build();

    // Replace the default schema for the main context with the schema providers
    // from the remote DBs
    let schema_provider =
        MultiSchemaProvider::new(vec![sqlite_schema_provider, postgres_schema_provider]);
    overwrite_default_schema(&state, Arc::new(schema_provider))
        .expect("Overwrite the default schema form the main context");

    // Create the session context for the main db
    let ctx = SessionContext::new_with_state(state);

    // Run a query
    let query = r#"SELECT t.* FROM test_pg as t join test_sqlite as a ON t.foo = a.foo"#;
    let df = ctx
        .sql(query)
        .await
        .expect("Create a dataframe from sql query");

    df.show().await.expect("Execute the dataframe");
}
