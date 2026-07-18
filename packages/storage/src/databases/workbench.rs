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

/// Upper bound on saved views registered into a single query session, guarding
/// against pathological view sets before dependency resolution runs.
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
    }

    register_views(&ctx, views, &base_tables).await?;
    Ok(ctx)
}

/// Registers saved views as virtual tables in dependency order. Views may
/// reference base tables and other views; ordering is resolved by a fixpoint
/// (repeatedly registering whatever plans successfully) which naturally detects
/// cycles and unresolved references.
async fn register_views(
    ctx: &SessionContext,
    views: &[WorkbenchView],
    base_tables: &HashSet<String>,
) -> Result<()> {
    if views.is_empty() {
        return Ok(());
    }
    if views.len() > MAX_WORKBENCH_VIEWS {
        return Err(anyhow!(
            "Too many views to register ({} > {})",
            views.len(),
            MAX_WORKBENCH_VIEWS
        ));
    }

    for view in views {
        if base_tables.contains(&view.name) {
            return Err(anyhow!(
                "View '{}' collides with an existing table name",
                view.name
            ));
        }
        if view.sql.contains('$') {
            return Err(anyhow!("View '{}' must not declare parameters", view.name));
        }
        validate_readonly_sql(&view.sql).map_err(|e| anyhow!("View '{}': {}", view.name, e))?;
    }

    let mut pending: Vec<&WorkbenchView> = views.iter().collect();
    let mut last_errors: HashMap<String, String> = HashMap::new();
    while !pending.is_empty() {
        let mut progressed = false;
        let mut still_pending = Vec::new();
        for view in pending {
            match ctx.sql(&view.sql).await {
                Ok(df) => {
                    ctx.register_table(&view.name, df.into_view())
                        .map_err(|e| anyhow!("Failed to register view '{}': {}", view.name, e))?;
                    progressed = true;
                }
                Err(error) => {
                    last_errors.insert(view.name.clone(), error.to_string());
                    still_pending.push(view);
                }
            }
        }
        if !progressed {
            let details: Vec<String> = still_pending
                .iter()
                .map(|view| {
                    format!(
                        "{} ({})",
                        view.name,
                        last_errors
                            .get(&view.name)
                            .map(String::as_str)
                            .unwrap_or("unresolved reference")
                    )
                })
                .collect();
            return Err(anyhow!(
                "Unresolved or cyclic view dependencies: {}",
                details.join("; ")
            ));
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
        .limit(0, Some(limit))?;

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
        if rows.len() >= limit {
            rows.truncate(limit);
            break;
        }
    }

    let truncated = rows.len() >= limit;
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
        assert!(
            validate_workbench_sql("WITH a AS (SELECT 1 AS x) SELECT x FROM a").is_ok()
        );
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
}
