//! Data Studio query workbench: a read-only, multi-table SQL surface over both
//! native tables and ontology overlays, with DataFusion-native parameters and
//! composable saved views. Execution reuses the same read-only validation,
//! concurrency semaphore, and adapter construction as the graph SQL surface.

pub mod saved_query;

use crate::arrow_utils::record_batch_to_value;
use crate::databases::graph::lancegraph::{
    CypherSafetyConfig, GraphOverlayDef, global_query_semaphore, open_table_adapter,
    validate_readonly_sql,
};
use datafusion::common::{ParamValues, ScalarValue};
use datafusion::prelude::SessionContext;
use flow_like_types::{Result, Value, anyhow};
use lancedb::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Fallback row cap when the caller does not supply a limit. Kept well below the
/// hard ceiling in [`CypherSafetyConfig`] so an unbounded workbench query cannot
/// materialize the whole table into JSON.
const DEFAULT_WORKBENCH_LIMIT: usize = 1_000;

/// Upper bound on saved views considered for a single query session. Additional
/// views are skipped so a pathological saved-view set cannot make every query
/// on the surface fail.
const MAX_WORKBENCH_VIEWS: usize = 64;

/// A single result column with its DataFusion-inferred type name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlColumn {
    pub name: String,
    pub type_name: String,
    pub position: usize,
}

/// A typed, bounded result set for the query workbench.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlQueryResult {
    pub columns: Vec<SqlColumn>,
    pub rows: Vec<Value>,
    pub row_count: usize,
    pub truncated: bool,
}

/// Which set of base tables a workbench query runs against.
pub enum WorkbenchSurface {
    /// All non-reserved native tables in the app database.
    Native,
    /// The node and edge tables of a single ontology overlay.
    Overlay(GraphOverlayDef),
    /// A frozen installed ontology contract. Physical tables are never
    /// registered directly; each name resolves to a projected view containing
    /// only contract-approved identity/display/endpoint/property columns.
    RemoteOverlay(GraphOverlayDef),
}

/// A saved view registered as a named virtual table before the query runs.
pub struct WorkbenchView {
    pub name: String,
    pub sql: String,
}

/// Internal tables (`__…__`) are never exposed to the workbench. Mirrors the
/// canonical `flow_like_catalog_core::is_reserved_table`; duplicated here because
/// `flow-like-storage` sits below `flow-like-catalog-core` in the dependency graph.
fn is_reserved_table(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__") && name.len() > 4
}

/// Validates that a string is a single read-only SQL statement. Exposed so
/// callers (e.g. the API) can reject writes before persisting a saved query.
pub fn validate_workbench_sql(sql: &str) -> Result<()> {
    validate_readonly_sql(sql)
}

/// Coerces a JSON parameter map into DataFusion named parameter values. Types are
/// inferred from the supplied values, so binding never interpolates text into the
/// SQL — placeholders are resolved by the planner via `$name`.
pub fn bind_params(params: &Value) -> Result<ParamValues> {
    let map = match params {
        Value::Object(map) => map,
        Value::Null => return Ok(ParamValues::Map(HashMap::new())),
        _ => return Err(anyhow!("Query parameters must be a JSON object")),
    };

    let mut values = HashMap::with_capacity(map.len());
    for (name, value) in map {
        let scalar =
            json_to_scalar(value).map_err(|error| anyhow!("Parameter '{}': {}", name, error))?;
        values.insert(name.clone(), scalar.into());
    }
    Ok(ParamValues::Map(values))
}

fn json_to_scalar(value: &Value) -> Result<ScalarValue> {
    Ok(match value {
        Value::Null => ScalarValue::Null,
        Value::Bool(value) => ScalarValue::Boolean(Some(*value)),
        Value::String(value) => ScalarValue::Utf8(Some(value.clone())),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                ScalarValue::Int64(Some(int))
            } else if let Some(uint) = number.as_u64() {
                ScalarValue::UInt64(Some(uint))
            } else if let Some(float) = number.as_f64() {
                ScalarValue::Float64(Some(float))
            } else {
                return Err(anyhow!("unsupported numeric parameter"));
            }
        }
        Value::Array(_) | Value::Object(_) => {
            return Err(anyhow!("array and object parameters are not supported"));
        }
    })
}

async fn build_context(
    connection: &Connection,
    surface: &WorkbenchSurface,
    views: &[WorkbenchView],
) -> Result<SessionContext> {
    let ctx = SessionContext::new();
    let mut base_tables: HashSet<String> = HashSet::new();

    match surface {
        WorkbenchSurface::Native => {
            let names = connection
                .table_names()
                .execute()
                .await
                .map_err(|e| anyhow!("Failed to list tables: {}", e))?;
            for name in names {
                if is_reserved_table(&name) {
                    continue;
                }
                let adapter = open_table_adapter(connection, &name).await?;
                ctx.register_table(&name, adapter)?;
                base_tables.insert(name);
            }
        }
        WorkbenchSurface::Overlay(overlay) => {
            let tables = overlay
                .nodes
                .iter()
                .map(|node| &node.table)
                .chain(overlay.edges.iter().map(|edge| &edge.table));
            for table in tables {
                if !base_tables.insert(table.clone()) {
                    continue;
                }
                let adapter = open_table_adapter(connection, table).await?;
                ctx.register_table(table, adapter)?;
            }
        }
        WorkbenchSurface::RemoteOverlay(overlay) => {
            let projections =
                crate::databases::graph::lancegraph::frozen_remote_table_projections(overlay)?;
            for (table, columns) in projections {
                let adapter = open_table_adapter(connection, &table).await?;
                let column_refs = columns.iter().map(String::as_str).collect::<Vec<_>>();
                let projected = ctx.read_table(adapter)?.select_columns(&column_refs)?;
                ctx.register_table(&table, projected.into_view())?;
                base_tables.insert(table);
            }
        }
    }

    register_views(&ctx, views, &base_tables).await?;
    Ok(ctx)
}

/// Registers saved views as virtual tables in dependency order. Views may
/// reference base tables and other views; ordering is resolved by a fixpoint
/// (repeatedly registering whatever plans successfully). Invalid, unresolved,
/// and cyclic views are skipped so they cannot prevent unrelated SQL from
/// executing.
async fn register_views(
    ctx: &SessionContext,
    views: &[WorkbenchView],
    base_tables: &HashSet<String>,
) -> Result<()> {
    if views.is_empty() {
        return Ok(());
    }
    if views.len() > MAX_WORKBENCH_VIEWS {
        tracing::warn!(
            supplied = views.len(),
            registered_cap = MAX_WORKBENCH_VIEWS,
            "Workbench view input exceeds the per-query registration cap; invalid entries are filtered before applying the cap"
        );
    }

    let normalized_base_tables = base_tables
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<HashSet<_>>();
    let mut eligible = Vec::with_capacity(views.len().min(MAX_WORKBENCH_VIEWS));
    let mut eligible_names = HashSet::new();
    for view in views {
        let normalized_name = view.name.to_lowercase();
        if normalized_base_tables.contains(&normalized_name) {
            tracing::warn!(
                view = %view.name,
                "Skipping workbench view because its name collides with a base table"
            );
            continue;
        }
        if view.sql.contains('$') {
            tracing::warn!(
                view = %view.name,
                "Skipping workbench view because saved views cannot declare parameters"
            );
            continue;
        }
        if let Err(error) = validate_readonly_sql(&view.sql) {
            tracing::warn!(
                view = %view.name,
                %error,
                "Skipping invalid workbench view"
            );
            continue;
        }
        if !eligible_names.insert(normalized_name) {
            tracing::warn!(
                view = %view.name,
                "Skipping duplicate workbench view name"
            );
            continue;
        }
        if eligible.len() == MAX_WORKBENCH_VIEWS {
            tracing::warn!(
                view = %view.name,
                registered_cap = MAX_WORKBENCH_VIEWS,
                "Skipping valid workbench view beyond the per-query registration cap"
            );
            continue;
        }
        eligible.push(view);
    }

    let mut pending = eligible;
    let mut last_errors: HashMap<String, String> = HashMap::new();
    while !pending.is_empty() {
        let mut progressed = false;
        let mut still_pending = Vec::new();
        for view in pending {
            match ctx.sql(&view.sql).await {
                Ok(df) => {
                    if let Err(error) = ctx.register_table(&view.name, df.into_view()) {
                        tracing::warn!(
                            view = %view.name,
                            %error,
                            "Skipping workbench view that could not be registered"
                        );
                    } else {
                        progressed = true;
                    }
                }
                Err(error) => {
                    last_errors.insert(view.name.clone(), error.to_string());
                    still_pending.push(view);
                }
            }
        }
        if !progressed {
            for view in still_pending {
                let error = last_errors
                    .get(&view.name)
                    .map(String::as_str)
                    .unwrap_or("unresolved reference");
                tracing::warn!(
                    view = %view.name,
                    error,
                    "Skipping unresolved or cyclic workbench view"
                );
            }
            break;
        }
        pending = still_pending;
    }
    Ok(())
}

/// Executes a single read-only SQL statement against the given surface, with the
/// supplied views registered and parameters bound. Enforces the shared read-only
/// validation, concurrency permit, timeout, and row cap.
pub async fn execute_readonly_sql(
    connection: &Connection,
    surface: WorkbenchSurface,
    views: Vec<WorkbenchView>,
    sql: &str,
    params: &Value,
    limit: Option<usize>,
) -> Result<SqlQueryResult> {
    let safety = CypherSafetyConfig::default();
    let limit = limit
        .unwrap_or(DEFAULT_WORKBENCH_LIMIT)
        .min(safety.max_limit);
    let semaphore = global_query_semaphore(safety.max_concurrent);
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;

    validate_readonly_sql(sql)?;
    let param_values = bind_params(params)?;
    let timeout = std::time::Duration::from_millis(safety.timeout_ms);

    let ctx = build_context(connection, &surface, &views).await?;

    let df = tokio::time::timeout(timeout, ctx.sql(sql))
        .await
        .map_err(|_| anyhow!("SQL planning timed out after {}ms", safety.timeout_ms))??
        .with_param_values(param_values)?
        // Fetch one sentinel row beyond the caller's cap so an exact-size
        // result is not falsely reported as truncated.
        .limit(0, Some(limit.saturating_add(1)))?;

    let columns: Vec<SqlColumn> = df
        .schema()
        .fields()
        .iter()
        .enumerate()
        .map(|(position, field)| SqlColumn {
            name: field.name().to_string(),
            type_name: field.data_type().to_string(),
            position,
        })
        .collect();

    let batches = tokio::time::timeout(timeout, df.collect())
        .await
        .map_err(|_| anyhow!("SQL query timed out after {}ms", safety.timeout_ms))??;

    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(record_batch_to_value(batch)?);
        if rows.len() > limit {
            break;
        }
    }

    let truncated = rows.len() > limit;
    rows.truncate(limit);
    let row_count = rows.len();
    Ok(SqlQueryResult {
        columns,
        rows,
        row_count,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_validation_allows_select_and_cte() {
        assert!(validate_workbench_sql("SELECT 1").is_ok());
        assert!(validate_workbench_sql("WITH a AS (SELECT 1 AS x) SELECT x FROM a").is_ok());
    }

    #[test]
    fn readonly_validation_rejects_writes_and_multi_statements() {
        assert!(validate_workbench_sql("INSERT INTO t VALUES (1)").is_err());
        assert!(validate_workbench_sql("DROP TABLE t").is_err());
        assert!(validate_workbench_sql("SELECT 1; SELECT 2").is_err());
    }

    #[test]
    fn bind_params_accepts_objects_and_null() {
        assert!(bind_params(&Value::Null).is_ok());
        let params = serde_json::json!({ "a": "x", "b": 3, "c": true, "d": 1.5 });
        assert!(bind_params(&params).is_ok());
        assert!(bind_params(&serde_json::json!("nope")).is_err());
        assert!(bind_params(&serde_json::json!({ "a": [1, 2] })).is_err());
    }

    #[test]
    fn json_to_scalar_infers_types() {
        assert!(matches!(
            json_to_scalar(&Value::Bool(true)).unwrap(),
            ScalarValue::Boolean(Some(true))
        ));
        assert!(matches!(
            json_to_scalar(&serde_json::json!(7)).unwrap(),
            ScalarValue::Int64(Some(7))
        ));
        assert!(matches!(
            json_to_scalar(&serde_json::json!(2.5)).unwrap(),
            ScalarValue::Float64(Some(_))
        ));
        assert!(matches!(
            json_to_scalar(&serde_json::json!(u64::MAX)).unwrap(),
            ScalarValue::UInt64(Some(u64::MAX))
        ));
        assert!(matches!(
            json_to_scalar(&Value::String("hi".into())).unwrap(),
            ScalarValue::Utf8(Some(_))
        ));
    }

    #[test]
    fn reserved_tables_are_hidden() {
        assert!(is_reserved_table("__graph_overlays__"));
        assert!(is_reserved_table("__saved_queries__"));
        assert!(!is_reserved_table("users"));
        assert!(!is_reserved_table("__"));
    }

    #[tokio::test]
    async fn register_views_keeps_resolvable_views_when_others_are_bad() -> Result<()> {
        let ctx = SessionContext::new();
        let base_tables = HashSet::from(["users".to_owned()]);
        let views = vec![
            // Put the dependent view first to exercise the fixpoint ordering.
            WorkbenchView {
                name: "dependent_view".to_owned(),
                sql: "SELECT value + 1 AS value FROM healthy_view".to_owned(),
            },
            WorkbenchView {
                name: "write_view".to_owned(),
                sql: "DELETE FROM users".to_owned(),
            },
            WorkbenchView {
                name: "parameter_view".to_owned(),
                sql: "SELECT $value AS value".to_owned(),
            },
            WorkbenchView {
                name: "users".to_owned(),
                sql: "SELECT 1 AS value".to_owned(),
            },
            WorkbenchView {
                name: "missing_view".to_owned(),
                sql: "SELECT * FROM missing_table".to_owned(),
            },
            WorkbenchView {
                name: "cycle_a".to_owned(),
                sql: "SELECT * FROM cycle_b".to_owned(),
            },
            WorkbenchView {
                name: "cycle_b".to_owned(),
                sql: "SELECT * FROM cycle_a".to_owned(),
            },
            WorkbenchView {
                name: "healthy_view".to_owned(),
                sql: "SELECT 1 AS value".to_owned(),
            },
        ];

        register_views(&ctx, &views, &base_tables).await?;

        let batches = ctx
            .sql("SELECT value FROM dependent_view")
            .await?
            .collect()
            .await?;
        assert_eq!(
            batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
            1
        );
        assert!(ctx.table("healthy_view").await.is_ok());
        assert!(ctx.table("write_view").await.is_err());
        assert!(ctx.table("parameter_view").await.is_err());
        assert!(ctx.table("missing_view").await.is_err());
        assert!(ctx.table("cycle_a").await.is_err());
        assert!(ctx.table("cycle_b").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn register_views_skips_entries_beyond_the_cap() -> Result<()> {
        let ctx = SessionContext::new();
        let views: Vec<WorkbenchView> = (0..MAX_WORKBENCH_VIEWS + 2)
            .map(|index| WorkbenchView {
                name: format!("view_{index}"),
                sql: format!("SELECT {index} AS value"),
            })
            .collect();

        register_views(&ctx, &views, &HashSet::new()).await?;

        assert!(ctx.table("view_0").await.is_ok());
        assert!(
            ctx.table(&format!("view_{}", MAX_WORKBENCH_VIEWS - 1))
                .await
                .is_ok()
        );
        assert!(
            ctx.table(&format!("view_{MAX_WORKBENCH_VIEWS}"))
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_legacy_views_do_not_consume_the_registration_cap() -> Result<()> {
        let ctx = SessionContext::new();
        let mut views: Vec<WorkbenchView> = (0..MAX_WORKBENCH_VIEWS)
            .map(|index| WorkbenchView {
                name: format!("invalid_{index}"),
                sql: "DELETE FROM users".to_string(),
            })
            .collect();
        views.push(WorkbenchView {
            name: "healthy_after_invalid".to_string(),
            sql: "SELECT 1 AS value".to_string(),
        });

        register_views(&ctx, &views, &HashSet::new()).await?;

        assert!(ctx.table("healthy_after_invalid").await.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn register_views_rejects_case_insensitive_legacy_name_collisions() -> Result<()> {
        let ctx = SessionContext::new();
        let views = vec![
            WorkbenchView {
                name: "USERS".to_string(),
                sql: "SELECT 1 AS value".to_string(),
            },
            WorkbenchView {
                name: "Revenue".to_string(),
                sql: "SELECT 1 AS value".to_string(),
            },
            WorkbenchView {
                name: "revenue".to_string(),
                sql: "SELECT 2 AS value".to_string(),
            },
        ];

        register_views(&ctx, &views, &HashSet::from(["users".to_string()])).await?;

        assert!(ctx.table("USERS").await.is_err());
        let batches = ctx
            .sql("SELECT value FROM Revenue")
            .await?
            .collect()
            .await?;
        let rows = record_batch_to_value(&batches[0])?;
        assert_eq!(rows[0].get("value").and_then(Value::as_i64), Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn remote_overlay_sql_exposes_only_frozen_contract_columns() -> Result<()> {
        use crate::databases::graph::lancegraph::{
            GraphOverlayDef, NodeMappingDef, PropertyColumnDef, PropertyProjectionMode,
        };
        use arrow::array::{RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = lancedb::connect(&test_path).execute().await?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("public", DataType::Utf8, false),
            Field::new("secret", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["1"])),
                Arc::new(StringArray::from(vec!["Ada"])),
                Arc::new(StringArray::from(vec!["approved"])),
                Arc::new(StringArray::from(vec!["internal-only"])),
            ],
        )?;
        connection
            .create_table("people", vec![batch])
            .execute()
            .await?;

        let overlay = GraphOverlayDef {
            id: "remote".to_string(),
            name: "Remote".to_string(),
            description: None,
            nodes: vec![NodeMappingDef {
                id: Some("person".to_string()),
                api_name: Some("person".to_string()),
                label: "Person".to_string(),
                table: "people".to_string(),
                id_column: "id".to_string(),
                display_column: Some("name".to_string()),
                property_columns: vec![PropertyColumnDef {
                    name: "public".to_string(),
                    data_type: "Utf8".to_string(),
                    nullable: false,
                }],
                style: Value::Null,
            }],
            edges: Vec::new(),
            object_views: Vec::new(),
            actions: Vec::new(),
            exposed: true,
            bindings_enabled: true,
            property_projection_mode: PropertyProjectionMode::Frozen,
            default_limit: 100,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        };

        let allowed = execute_readonly_sql(
            &connection,
            WorkbenchSurface::RemoteOverlay(overlay.clone()),
            Vec::new(),
            "SELECT id, name, public FROM people",
            &Value::Null,
            Some(10),
        )
        .await?;
        assert_eq!(allowed.row_count, 1);
        assert!(allowed.rows[0].get("secret").is_none());

        let denied = execute_readonly_sql(
            &connection,
            WorkbenchSurface::RemoteOverlay(overlay.clone()),
            Vec::new(),
            "SELECT secret FROM people",
            &Value::Null,
            Some(10),
        )
        .await;
        assert!(denied.is_err());

        let local = execute_readonly_sql(
            &connection,
            WorkbenchSurface::Overlay(overlay),
            Vec::new(),
            "SELECT secret FROM people",
            &Value::Null,
            Some(10),
        )
        .await?;
        assert_eq!(
            local.rows[0].get("secret").and_then(Value::as_str),
            Some("internal-only")
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn truncated_requires_a_row_beyond_the_limit() -> Result<()> {
        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = lancedb::connect(&test_path).execute().await?;
        let sql = "SELECT * FROM (VALUES (1), (2)) AS rows(value)";

        let exact = execute_readonly_sql(
            &connection,
            WorkbenchSurface::Native,
            Vec::new(),
            sql,
            &Value::Null,
            Some(2),
        )
        .await?;
        assert_eq!(exact.row_count, 2);
        assert!(!exact.truncated);

        let capped = execute_readonly_sql(
            &connection,
            WorkbenchSurface::Native,
            Vec::new(),
            sql,
            &Value::Null,
            Some(1),
        )
        .await?;
        assert_eq!(capped.row_count, 1);
        assert!(capped.truncated);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }
}
