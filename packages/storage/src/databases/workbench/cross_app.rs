//! Read-only SQL over the tables of several apps in one session. Every app is a schema
//! (`crm.customers`) with its own column restrictions, and only the tables the statement
//! names are opened, so a query over two tables of an object-store app opens two tables.

use super::{
    SqlQueryResult, bind_params, collect_bounded, is_reserved_table, plan_bounded, query_timeout,
    row_limit,
};
use crate::databases::graph::lancegraph::{
    CypherSafetyConfig, global_query_semaphore, open_table_adapter, validate_readonly_sql,
};
use datafusion::catalog::MemorySchemaProvider;
use datafusion::common::TableReference;
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::{SessionConfig, SessionContext};
use flow_like_storage_contracts::column_metadata::{ColumnReader, DataSensitivity};
use flow_like_types::{Result, Value, anyhow};
use lancedb::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

const RESERVED_SCHEMAS: [&str; 2] = ["public", "information_schema"];
const MAX_SCHEMA_NAME_CHARS: usize = 63;

/// One app's tables in a cross-app session, addressed as `<name>.<table>`.
#[derive(Clone)]
pub struct WorkbenchSchema {
    /// A lowercase SQL identifier: a letter or `_`, then letters, digits or `_`.
    pub name: String,
    pub connection: Connection,
    /// Columns this reader may not read are hidden, as on the single-app workbench.
    pub reader: ColumnReader,
}

/// Bounds of one cross-app query, on top of the shared timeout and concurrency permits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkbenchLimits {
    /// Rows returned: the workbench default when unset, never above its ceiling.
    pub rows: Option<usize>,
    /// Bytes the query's operators may hold at once; unbounded when unset.
    pub memory_bytes: Option<usize>,
}

/// A cross-app result with the tables it scanned and the label of the columns it read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossAppQueryResult {
    #[serde(flatten)]
    pub result: SqlQueryResult,
    /// `schema.table` of every scanned table, sorted.
    pub tables: Vec<String>,
    /// The strictest classification, and personal data, over every column the query read,
    /// including columns only filtered on.
    pub sensitivity: DataSensitivity,
}

/// Executes one read-only statement over the tables of `schemas`, with the validation,
/// parameter binding, concurrency permit, timeout and row cap of the single-app workbench.
/// Tables must be schema-qualified; table functions such as `generate_series` need no schema.
pub async fn execute_readonly_sql_across_apps(
    schemas: &[WorkbenchSchema],
    sql: &str,
    params: &Value,
    limits: WorkbenchLimits,
) -> Result<CrossAppQueryResult> {
    validate_schema_names(schemas)?;
    let safety = CypherSafetyConfig::default();
    let limit = row_limit(limits.rows, &safety);
    let semaphore = global_query_semaphore(safety.max_concurrent);
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;

    validate_readonly_sql(sql)?;
    let param_values = bind_params(params)?;

    let ctx = tokio::time::timeout(
        query_timeout(&safety),
        cross_app_context(schemas, sql, limits.memory_bytes),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "Opening the tables of the query timed out after {}ms",
            safety.timeout_ms
        )
    })??;
    let df = plan_bounded(&ctx, sql, param_values, limit, &safety).await?;
    let (tables, sensitivity) = scanned_sources(&df.clone().into_optimized_plan()?)?;
    let result = collect_bounded(df, limit, &safety).await?;
    Ok(CrossAppQueryResult {
        result,
        tables,
        sensitivity,
    })
}

fn validate_schema_names(schemas: &[WorkbenchSchema]) -> Result<()> {
    if schemas.is_empty() {
        return Err(anyhow!("A cross-app query needs at least one app schema"));
    }
    let mut seen = HashSet::new();
    for schema in schemas {
        let name = schema.name.as_str();
        let identifier = name.len() <= MAX_SCHEMA_NAME_CHARS
            && name
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !identifier {
            return Err(anyhow!(
                "Schema name '{name}' must be a lowercase SQL identifier of at most \
                 {MAX_SCHEMA_NAME_CHARS} characters: a-z, 0-9 and _, not starting with a digit"
            ));
        }
        if RESERVED_SCHEMAS.contains(&name) {
            return Err(anyhow!("Schema name '{name}' is reserved"));
        }
        if !seen.insert(name) {
            return Err(anyhow!(
                "Schema name '{name}' is given to more than one app"
            ));
        }
    }
    Ok(())
}

fn session_context(memory_bytes: Option<usize>) -> Result<SessionContext> {
    let mut runtime = RuntimeEnvBuilder::new();
    if let Some(bytes) = memory_bytes {
        runtime = runtime.with_memory_limit(bytes, 1.0);
    }
    let state = SessionStateBuilder::new()
        .with_config(SessionConfig::new())
        .with_runtime_env(runtime.build_arc()?)
        .with_default_features()
        .build();
    let ctx = SessionContext::new_with_state(
        crate::databases::df_provider::with_lance_order_pushdown(state),
    );
    crate::databases::register_sql_functions(&ctx);
    Ok(ctx)
}

/// A session holding exactly the tables `sql` names, each under its app's schema.
async fn cross_app_context(
    schemas: &[WorkbenchSchema],
    sql: &str,
    memory_bytes: Option<usize>,
) -> Result<SessionContext> {
    let ctx = session_context(memory_bytes)?;
    let state = ctx.state();
    let options = state.config().options();
    let default_catalog = options.catalog.default_catalog.clone();
    let statement = state.sql_to_statement(sql, &options.sql_parser.dialect)?;
    let names = schemas
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let mut wanted: BTreeMap<&str, (&WorkbenchSchema, BTreeSet<String>)> = BTreeMap::new();
    for reference in state.resolve_table_references(&statement)? {
        let (schema_name, table) = match &reference {
            TableReference::Partial { schema, table } => (schema.as_ref(), table.as_ref()),
            TableReference::Full {
                catalog,
                schema,
                table,
            } if catalog.as_ref() == default_catalog => (schema.as_ref(), table.as_ref()),
            TableReference::Full { .. } => {
                return Err(anyhow!(
                    "Table '{reference}' names a catalog; address tables as <schema>.<table> \
                     with one of the schemas {names}"
                ));
            }
            TableReference::Bare { table } => {
                if state.table_functions().contains_key(table.as_ref()) {
                    continue;
                }
                return Err(anyhow!(
                    "Table '{table}' needs its app schema, as in <schema>.{table}; \
                     schemas: {names}"
                ));
            }
        };
        let schema = schemas
            .iter()
            .find(|schema| schema.name == schema_name)
            .ok_or_else(|| {
                anyhow!("Unknown schema '{schema_name}' in '{reference}'; schemas: {names}")
            })?;
        if is_reserved_table(table) {
            return Err(anyhow!(
                "Table '{reference}' is internal and cannot be queried"
            ));
        }
        wanted
            .entry(schema.name.as_str())
            .or_insert_with(|| (schema, BTreeSet::new()))
            .1
            .insert(table.to_string());
    }

    let catalog = ctx
        .catalog(&default_catalog)
        .ok_or_else(|| anyhow!("The session has no default catalog '{default_catalog}'"))?;
    for (schema_name, (schema, tables)) in wanted {
        catalog.register_schema(schema_name, Arc::new(MemorySchemaProvider::new()))?;
        for table in tables {
            let adapter = open_table_adapter(&schema.connection, &table, &schema.reader)
                .await
                .map_err(|error| anyhow!("Failed to open '{schema_name}.{table}': {error}"))?;
            ctx.register_table(TableReference::partial(schema_name, table), adapter)?;
        }
    }
    Ok(ctx)
}

/// The scanned tables and the combined label of the columns their scans return or filter on.
fn scanned_sources(plan: &LogicalPlan) -> Result<(Vec<String>, DataSensitivity)> {
    let mut tables = BTreeSet::new();
    let mut sensitivity = DataSensitivity::default();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            if let Some(schema) = scan.table_name.schema() {
                tables.insert(format!("{schema}.{}", scan.table_name.table()));
            }
            let schema = scan.source.schema();
            let mut read: BTreeSet<usize> = scan
                .projection
                .as_ref()
                .map(|indices| indices.iter().copied().collect())
                .unwrap_or_else(|| (0..schema.fields().len()).collect());
            for expression in &scan.filters {
                for column in expression.column_refs() {
                    read.insert(schema.index_of(&column.name)?);
                }
            }
            for field in read
                .into_iter()
                .filter_map(|index| schema.fields().get(index))
            {
                sensitivity = sensitivity.combine(DataSensitivity::from_metadata(field.metadata()));
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok((tables.into_iter().collect(), sensitivity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use flow_like_storage_contracts::column_metadata::{
        CLASSIFICATION_METADATA_KEY, DataClassification, PII_METADATA_KEY, READ_ATTRIBUTES_KEY,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;

    struct TempApp {
        path: PathBuf,
        connection: Connection,
    }

    impl TempApp {
        async fn with_tables(tables: Vec<(&str, RecordBatch)>) -> Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "flow-like-cross-app-{}",
                flow_like_types::create_id()
            ));
            std::fs::create_dir_all(&path)?;
            let connection = lancedb::connect(&path.to_string_lossy()).execute().await?;
            for (name, batch) in tables {
                connection.create_table(name, vec![batch]).execute().await?;
            }
            Ok(Self { path, connection })
        }

        fn schema(&self, name: &str, reader: ColumnReader) -> WorkbenchSchema {
            WorkbenchSchema {
                name: name.to_string(),
                connection: self.connection.clone(),
                reader,
            }
        }
    }

    impl Drop for TempApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn labelled(name: &str, data_type: DataType, labels: &[(&str, &str)]) -> Field {
        Field::new(name, data_type, true).with_metadata(
            labels
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<HashMap<_, _>>(),
        )
    }

    /// `customers(id, name, region, email)`; `email` is confidential personal data that only
    /// readers holding `support` may see.
    fn customers() -> Result<RecordBatch> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            labelled(
                "name",
                DataType::Utf8,
                &[(CLASSIFICATION_METADATA_KEY, "internal")],
            ),
            Field::new("region", DataType::Utf8, true),
            labelled(
                "email",
                DataType::Utf8,
                &[
                    (CLASSIFICATION_METADATA_KEY, "confidential"),
                    (PII_METADATA_KEY, "true"),
                    (READ_ATTRIBUTES_KEY, "support"),
                ],
            ),
        ]));
        Ok(RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["Acme", "Globex", "Initech"])),
                Arc::new(StringArray::from(vec!["West", "East", "West"])),
                Arc::new(StringArray::from(vec![
                    "a@acme.test",
                    "g@globex.test",
                    "i@initech.test",
                ])),
            ],
        )?)
    }

    fn orders() -> Result<RecordBatch> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("customer_id", DataType::Int64, false),
            Field::new("total", DataType::Int64, false),
        ]));
        Ok(RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![10, 11, 12, 13])),
                Arc::new(Int64Array::from(vec![1, 1, 2, 3])),
                Arc::new(Int64Array::from(vec![100, 50, 70, 30])),
            ],
        )?)
    }

    async fn crm_and_shop() -> Result<(TempApp, TempApp)> {
        Ok((
            TempApp::with_tables(vec![("customers", customers()?)]).await?,
            TempApp::with_tables(vec![("orders", orders()?)]).await?,
        ))
    }

    fn limits(rows: usize) -> WorkbenchLimits {
        WorkbenchLimits {
            rows: Some(rows),
            memory_bytes: None,
        }
    }

    #[tokio::test]
    async fn joins_tables_of_two_apps_with_bound_parameters() -> Result<()> {
        let (crm, shop) = crm_and_shop().await?;
        let schemas = [
            crm.schema("crm", ColumnReader::Unrestricted),
            shop.schema("shop", ColumnReader::Unrestricted),
        ];

        let result = execute_readonly_sql_across_apps(
            &schemas,
            "SELECT c.name, SUM(o.total) AS total FROM crm.customers c \
             JOIN shop.orders o ON o.customer_id = c.id \
             WHERE c.region = $region GROUP BY c.name ORDER BY c.name",
            &json!({ "region": "West" }),
            limits(10),
        )
        .await?;

        assert_eq!(
            result.result.rows,
            vec![
                json!({ "name": "Acme", "total": 150 }),
                json!({ "name": "Initech", "total": 30 }),
            ]
        );
        assert_eq!(result.tables, vec!["crm.customers", "shop.orders"]);
        assert_eq!(
            result.sensitivity,
            DataSensitivity {
                classification: Some(DataClassification::Internal),
                pii: false,
            }
        );
        assert!(
            execute_readonly_sql_across_apps(
                &schemas,
                "SELECT name FROM crm.customers WHERE region = $region",
                &Value::Null,
                limits(10),
            )
            .await
            .is_err(),
            "an unbound placeholder must not read as NULL"
        );
        Ok(())
    }

    #[tokio::test]
    async fn every_app_keeps_its_own_column_restrictions() -> Result<()> {
        let crm = TempApp::with_tables(vec![("customers", customers()?)]).await?;
        let sql = "SELECT email FROM crm.customers ORDER BY id";

        let restricted = [crm.schema("crm", ColumnReader::with_attributes(["sales"]))];
        assert!(
            execute_readonly_sql_across_apps(&restricted, sql, &Value::Null, limits(10))
                .await
                .is_err()
        );
        let visible = execute_readonly_sql_across_apps(
            &restricted,
            "SELECT * FROM crm.customers ORDER BY id",
            &Value::Null,
            limits(10),
        )
        .await?;
        let columns: Vec<_> = visible
            .result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        assert_eq!(columns, vec!["id", "name", "region"]);

        let support = [crm.schema("crm", ColumnReader::with_attributes(["support"]))];
        let emails =
            execute_readonly_sql_across_apps(&support, sql, &Value::Null, limits(10)).await?;
        assert_eq!(emails.result.row_count, 3);
        assert_eq!(
            emails.sensitivity,
            DataSensitivity {
                classification: Some(DataClassification::Confidential),
                pii: true,
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn filter_only_columns_count_towards_the_label() -> Result<()> {
        let crm = TempApp::with_tables(vec![("customers", customers()?)]).await?;
        let schemas = [crm.schema("crm", ColumnReader::Unrestricted)];

        let result = execute_readonly_sql_across_apps(
            &schemas,
            "SELECT id FROM crm.customers WHERE email = 'a@acme.test'",
            &Value::Null,
            limits(10),
        )
        .await?;

        assert_eq!(result.result.rows, vec![json!({ "id": 1 })]);
        assert_eq!(
            result.sensitivity.classification,
            Some(DataClassification::Confidential)
        );
        assert!(result.sensitivity.pii);
        Ok(())
    }

    #[tokio::test]
    async fn opens_only_the_tables_the_statement_names() -> Result<()> {
        let crm = TempApp::with_tables(vec![("customers", customers()?)]).await?;
        let broken = crm.path.join("broken.lance");
        std::fs::create_dir_all(&broken)?;
        std::fs::write(broken.join("_latest.manifest"), b"not a manifest")?;
        let schemas = [crm.schema("crm", ColumnReader::Unrestricted)];

        let result = execute_readonly_sql_across_apps(
            &schemas,
            "WITH west AS (SELECT * FROM crm.customers WHERE region = 'West') \
             SELECT COUNT(*) AS n FROM west",
            &Value::Null,
            limits(10),
        )
        .await?;
        assert_eq!(result.result.rows, vec![json!({ "n": 2 })]);

        let error = execute_readonly_sql_across_apps(
            &schemas,
            "SELECT * FROM crm.broken",
            &Value::Null,
            limits(10),
        )
        .await
        .expect_err("a broken table fails only when it is named");
        assert!(
            error.to_string().contains("Failed to open 'crm.broken'"),
            "{error}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn refuses_writes_and_tables_outside_the_schemas() -> Result<()> {
        let (crm, shop) = crm_and_shop().await?;
        let schemas = [
            crm.schema("crm", ColumnReader::Unrestricted),
            shop.schema("shop", ColumnReader::Unrestricted),
        ];
        let run = |sql: &'static str| {
            let schemas = schemas.clone();
            async move {
                execute_readonly_sql_across_apps(&schemas, sql, &Value::Null, limits(10))
                    .await
                    .expect_err(sql)
                    .to_string()
            }
        };

        assert!(run("DELETE FROM crm.customers").await.contains("read-only"));
        assert!(
            run("SELECT 1; SELECT 2")
                .await
                .contains("Exactly one SQL statement")
        );
        assert!(
            run("SELECT * FROM customers")
                .await
                .contains("needs its app schema")
        );
        assert!(
            run("SELECT * FROM hr.people")
                .await
                .contains("Unknown schema 'hr'")
        );
        assert!(
            run("SELECT * FROM crm.__saved_queries__")
                .await
                .contains("internal")
        );
        assert!(
            run("SELECT * FROM other.crm.customers")
                .await
                .contains("names a catalog")
        );
        Ok(())
    }

    #[tokio::test]
    async fn schema_names_are_unique_lowercase_identifiers() -> Result<()> {
        let crm = TempApp::with_tables(vec![("customers", customers()?)]).await?;
        for (names, expected) in [
            (vec!["CRM"], "lowercase SQL identifier"),
            (vec!["1crm"], "lowercase SQL identifier"),
            (vec!["crm-eu"], "lowercase SQL identifier"),
            (vec![""], "lowercase SQL identifier"),
            (vec!["public"], "reserved"),
            (vec!["crm", "crm"], "more than one app"),
            (vec![], "at least one app schema"),
        ] {
            let schemas: Vec<_> = names
                .iter()
                .map(|name| crm.schema(name, ColumnReader::Unrestricted))
                .collect();
            let error = execute_readonly_sql_across_apps(
                &schemas,
                "SELECT 1 AS one",
                &Value::Null,
                limits(10),
            )
            .await
            .expect_err(&format!("{names:?}"))
            .to_string();
            assert!(error.contains(expected), "{names:?}: {error}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn memory_limit_bounds_what_operators_may_hold() -> Result<()> {
        let crm = TempApp::with_tables(vec![("customers", customers()?)]).await?;
        let schemas = [crm.schema("crm", ColumnReader::Unrestricted)];
        let sql = "SELECT COUNT(*) AS n FROM generate_series(1, 1000000) a \
                   JOIN generate_series(1, 1000000) b ON a.value = b.value";

        let unbounded =
            execute_readonly_sql_across_apps(&schemas, sql, &Value::Null, limits(10)).await?;
        assert_eq!(unbounded.result.rows, vec![json!({ "n": 1_000_000 })]);
        assert!(unbounded.tables.is_empty());

        let error = execute_readonly_sql_across_apps(
            &schemas,
            sql,
            &Value::Null,
            WorkbenchLimits {
                rows: Some(10),
                memory_bytes: Some(1024 * 1024),
            },
        )
        .await
        .expect_err("a 1 MiB pool cannot hold a million-row hash table");
        assert!(error.to_string().contains("Resources exhausted"), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn row_cap_reports_truncation() -> Result<()> {
        let shop = TempApp::with_tables(vec![("orders", orders()?)]).await?;
        let schemas = [shop.schema("shop", ColumnReader::Unrestricted)];
        let result = execute_readonly_sql_across_apps(
            &schemas,
            "SELECT id FROM shop.orders ORDER BY id",
            &Value::Null,
            limits(3),
        )
        .await?;
        assert_eq!(result.result.row_count, 3);
        assert!(result.result.truncated);
        Ok(())
    }
}
