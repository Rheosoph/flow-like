//! The data tools of a headless server turn: table changes in the bot's own workspace app and
//! the read-only graph queries. Both call the route handlers as the bot, so its rights,
//! row and column restrictions and quotas apply.

use std::collections::HashMap;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_bot_memory::store::is_memory_table;
use flow_like_storage::contracts::database::DatabaseSelector;
use serde_json::{Map, Value, json};

use super::{AppTools, MAX_ROWS, ToolResult, bounded, failed, required, text};
use crate::routes::app::{
    db::{
        ScopeParams,
        add_column::{AddColumnPayload, add_column},
        alter_column::{AlterColumnPayload, alter_column},
        build_index::{BuildIndexPayload, IndexType, build_index},
        create_table::{CreateTablePayload, create_table},
        db_add::{AddToDBPayload, add_to_table},
        db_delete::{DeleteFromDBPayload, delete_from_table},
        db_update::{UpdatePayload, update_table},
        drop_columns::{DropColumnsPayload, drop_columns},
        drop_index::drop_index,
        drop_table::drop_table,
        optimize::{OptimizePayload, optimize_table},
        set_column_metadata::{ColumnMetadataPayload, set_column_metadata},
    },
    graph::{
        analytics::{AnalyticsParams, graph_analytics},
        cypher::{CypherPayload, run_cypher},
        neighbors::{NeighborsPayload, neighbors},
        paths::{PathsPayload, find_paths},
        sample::{SampleParams, sample_nodes},
        search::{SearchNodesPayload, search_nodes},
        sql::{SqlPayload, run_sql},
        subgraph::{SubgraphPayload, subgraph},
    },
};

const MAX_GRAPH_DEPTH: u64 = 5;
const MAX_SAMPLE_ROWS: u64 = 50;
const MAX_GRAPH_NODES: usize = 200;
const MAX_GRAPH_EDGES: usize = 400;
const COLUMN_METADATA_KEYS: [&str; 5] = [
    "display_name",
    "description",
    "classification",
    "pii",
    "read_attributes",
];

/// Argument keys each graph query operation forwards to its route payload.
const GRAPH_OPERATION_KEYS: [(&str, &[&str]); 8] = [
    ("cypher", &["query", "params"]),
    ("sql", &["query", "params"]),
    (
        "neighbors",
        &["label", "node_id", "depth", "direction", "edge_labels"],
    ),
    ("subgraph", &["seeds", "depth"]),
    (
        "paths",
        &["from_label", "from_id", "to_label", "to_id", "max_depth"],
    ),
    ("analytics", &[]),
    ("search_nodes", &["query"]),
    ("sample", &["label", "n"]),
];

enum TableChange {
    Create(CreateTablePayload),
    Insert(Vec<Value>),
    Delete(DeleteFromDBPayload),
    Update(UpdatePayload),
    BuildIndex(BuildIndexPayload),
    DropIndex(String),
    Optimize(OptimizePayload),
    AddColumn(AddColumnPayload),
    DropColumns(Vec<String>),
    AlterColumn(AlterColumnPayload),
    SetColumnMetadata(String, ColumnMetadataPayload),
    DropTable,
}

fn from_args<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("{what} is not valid: {error}"))
}

fn object<'a>(args: &'a Value, key: &str, operation: &str) -> Result<&'a Value, String> {
    args.get(key)
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("Operation '{operation}' requires {key} as an object."))
}

fn flag(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn index_type(args: &Value) -> Result<IndexType, String> {
    let name = text(args, "index_type").unwrap_or("Auto");
    serde_json::from_value(json!(name)).map_err(|_| format!("Unknown index_type '{name}'."))
}

fn column_metadata(args: &Value) -> Result<ColumnMetadataPayload, String> {
    let metadata = object(args, "column_metadata", "set_column_metadata")?;
    if let Some(key) = metadata
        .as_object()
        .into_iter()
        .flat_map(Map::keys)
        .find(|key| !COLUMN_METADATA_KEYS.contains(&key.as_str()))
    {
        return Err(format!("column_metadata has the unknown field '{key}'."));
    }
    from_args(metadata.clone(), "column_metadata")
}

fn column_definition(args: &Value) -> Result<AddColumnPayload, String> {
    let definition = object(args, "column_definition", "add_column")?;
    required(definition, "name")?;
    if text(definition, "sql_expression").is_some() == text(definition, "type").is_some() {
        return Err(
            "add_column requires column_definition.name and exactly one of sql_expression or type."
                .to_string(),
        );
    }
    from_args(definition.clone(), "column_definition")
}

fn rows(args: &Value, operation: &str) -> Result<Vec<Value>, String> {
    args.get("items")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .cloned()
        .ok_or_else(|| format!("Operation '{operation}' requires items as a non-empty array."))
}

fn dropped_columns(args: &Value) -> Result<Vec<String>, String> {
    let columns: Vec<String> = args
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if columns.is_empty() {
        Err("Operation 'drop_columns' requires columns as a non-empty array of names.".to_string())
    } else {
        Ok(columns)
    }
}

fn parse_change(operation: &str, args: &Value) -> Result<(String, TableChange), String> {
    let table = required(args, "table_name")?;
    if is_memory_table(table) {
        return Err(format!(
            "Table '{table}' holds the bot's memory; it is read and changed through memory, not database_tool."
        ));
    }
    let change = match operation {
        "create_table" => TableChange::Create(from_args(
            json!({
                "fields": args.get("fields").cloned().unwrap_or(Value::Null),
                "if_not_exists": flag(args, "if_not_exists").unwrap_or(true),
            }),
            "create_table fields",
        )?),
        "insert" | "add_items" => TableChange::Insert(rows(args, operation)?),
        "delete" | "remove_items" => TableChange::Delete(DeleteFromDBPayload {
            query: required(args, "filter")?.to_string(),
        }),
        "update" => {
            let updates = object(args, "updates", operation)?;
            TableChange::Update(UpdatePayload {
                filter: required(args, "filter")?.to_string(),
                updates: from_args(updates.clone(), "updates")?,
            })
        }
        "build_index" => TableChange::BuildIndex(BuildIndexPayload {
            column: required(args, "column")?.to_string(),
            index_type: index_type(args)?,
            optimize: flag(args, "optimize").unwrap_or(false),
        }),
        "drop_index" => TableChange::DropIndex(required(args, "index_name")?.to_string()),
        "optimize" => TableChange::Optimize(OptimizePayload {
            keep_versions: flag(args, "keep_versions").unwrap_or(true),
        }),
        "add_column" => TableChange::AddColumn(column_definition(args)?),
        "drop_columns" => TableChange::DropColumns(dropped_columns(args)?),
        "alter_column" => TableChange::AlterColumn(AlterColumnPayload {
            column: required(args, "column")?.to_string(),
            rename: None,
            nullable: Some(
                flag(args, "nullable").ok_or("Operation 'alter_column' requires nullable.")?,
            ),
        }),
        "set_column_metadata" => TableChange::SetColumnMetadata(
            required(args, "column")?.to_string(),
            column_metadata(args)?,
        ),
        "delete_table" => {
            if required(args, "confirm_table_name")? != table {
                return Err(format!(
                    "delete_table confirmation mismatch: confirm_table_name must equal '{table}'. Nothing was deleted."
                ));
            }
            TableChange::DropTable
        }
        "import_geojson" => {
            return Err(
                "import_geojson reads a file forwarded to the run, which a bot turn has none of; insert the features as rows instead."
                    .to_string(),
            );
        }
        other => return Err(format!("Unsupported database_tool operation '{other}'.")),
    };
    Ok((table.to_string(), change))
}

fn cap(value: &mut Value, key: &str, max: usize) -> bool {
    match value.get_mut(key).and_then(Value::as_array_mut) {
        Some(items) if items.len() > max => {
            items.truncate(max);
            true
        }
        _ => false,
    }
}

fn bound_graph_result(value: &mut Value) {
    let cut = [
        cap(value, "nodes", MAX_GRAPH_NODES),
        cap(value, "edges", MAX_GRAPH_EDGES),
        cap(value, "paths", MAX_ROWS as usize),
    ];
    if cut.contains(&true) {
        value["truncated"] = json!(true);
    }
}

fn graph_payload(operation: &str, args: &Value, limit: u64) -> Result<Value, String> {
    let keys = GRAPH_OPERATION_KEYS
        .iter()
        .find(|(name, _)| *name == operation)
        .map(|(_, keys)| *keys)
        .ok_or_else(|| format!("Unsupported graph_query_tool operation '{operation}'."))?;
    let mut payload: Map<String, Value> = keys
        .iter()
        .filter_map(|key| {
            let value = args.get(*key).filter(|value| !value.is_null())?;
            Some((key.to_string(), value.clone()))
        })
        .collect();
    for key in ["depth", "max_depth"] {
        if payload.contains_key(key) {
            payload.insert(
                key.to_string(),
                json!(bounded(args, key, 1, MAX_GRAPH_DEPTH)),
            );
        }
    }
    if payload.contains_key("n") {
        payload.insert(
            "n".to_string(),
            json!(bounded(args, "n", 10, MAX_SAMPLE_ROWS)),
        );
    }
    payload.insert("limit".to_string(), json!(limit));
    Ok(Value::Object(payload))
}

fn ok_with(key: &str, value: Value) -> Value {
    json!({ "status": "ok", key: value })
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

impl AppTools {
    pub(super) async fn change_workspace_table(
        &self,
        app_id: &str,
        operation: &str,
        args: &Value,
    ) -> ToolResult {
        let (table, change) = parse_change(operation, args)?;
        let (state, user) = (State(self.state.clone()), Extension(self.user()));
        let path = || Path((app_id.to_string(), table.clone()));
        let scope = || Query(ScopeParams::default());
        let selector = || Query(DatabaseSelector::default());
        let done = || json!({ "status": "ok", "table_name": table });
        match change {
            TableChange::Create(payload) => {
                let created = create_table(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?;
                let mut answer = to_json(&created.0)?;
                answer["status"] = json!("ok");
                Ok(answer)
            }
            TableChange::Insert(items) => {
                let inserted = items.len();
                add_to_table(
                    state,
                    user,
                    path(),
                    scope(),
                    selector(),
                    Query(HashMap::new()),
                    Json(AddToDBPayload { items }),
                )
                .await
                .map_err(failed)?
                .0;
                Ok(json!({ "status": "ok", "table_name": table, "inserted": inserted }))
            }
            TableChange::Delete(payload) => {
                let filter = payload.query.clone();
                delete_from_table(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(json!({ "status": "ok", "table_name": table, "filter": filter }))
            }
            TableChange::Update(payload) => {
                let filter = payload.filter.clone();
                update_table(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(json!({ "status": "ok", "table_name": table, "filter": filter }))
            }
            TableChange::BuildIndex(payload) => {
                let column = payload.column.clone();
                build_index(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(json!({ "status": "ok", "table_name": table, "column": column }))
            }
            TableChange::DropIndex(index_name) => {
                drop_index(
                    state,
                    user,
                    Path((app_id.to_string(), table.clone(), index_name.clone())),
                    scope(),
                    selector(),
                )
                .await
                .map_err(failed)?
                .0;
                Ok(json!({ "status": "ok", "table_name": table, "index_name": index_name }))
            }
            TableChange::Optimize(payload) => {
                optimize_table(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(done())
            }
            TableChange::AddColumn(payload) => {
                let column = payload.name.clone();
                add_column(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(json!({ "status": "ok", "table_name": table, "column": column }))
            }
            TableChange::DropColumns(columns) => {
                drop_columns(
                    state,
                    user,
                    path(),
                    scope(),
                    selector(),
                    Json(DropColumnsPayload {
                        columns: columns.clone(),
                    }),
                )
                .await
                .map_err(failed)?
                .0;
                Ok(json!({ "status": "ok", "table_name": table, "columns": columns }))
            }
            TableChange::AlterColumn(payload) => {
                let column = payload.column.clone();
                alter_column(state, user, path(), scope(), selector(), Json(payload))
                    .await
                    .map_err(failed)?
                    .0;
                Ok(json!({ "status": "ok", "table_name": table, "column": column }))
            }
            TableChange::SetColumnMetadata(column, payload) => {
                let saved = set_column_metadata(
                    state,
                    user,
                    Path((app_id.to_string(), table.clone(), column.clone())),
                    scope(),
                    selector(),
                    Json(payload),
                )
                .await
                .map_err(failed)?;
                Ok(json!({
                    "status": "ok",
                    "table_name": table,
                    "column": column,
                    "column_metadata": to_json(&saved.0)?,
                }))
            }
            TableChange::DropTable => {
                let dropped = drop_table(state, user, path(), scope(), selector())
                    .await
                    .map_err(failed)?;
                let dropped = dropped.0;
                Ok(json!({
                    "status": "ok",
                    "app_id": app_id,
                    "table_name": table,
                    "dropped": dropped.dropped,
                    "irreversible": true,
                    "ontologies_pruned": dropped.ontologies,
                    "saved_queries_referencing": dropped.saved_queries,
                    "warnings": dropped.warnings,
                }))
            }
        }
    }

    pub(super) async fn graph_query(&self, args: &Value) -> ToolResult {
        let app_id = self.app_id(args)?;
        if flag(args, "user_scoped") == Some(true) {
            return Err(
                "A bot queries an app's shared graph only; call again without user_scoped."
                    .to_string(),
            );
        }
        let operation = required(args, "operation")?;
        let overlay_id = required(args, "overlay_id")?;
        let limit = bounded(args, "limit", 100, MAX_ROWS);
        let payload = graph_payload(operation, args, limit)?;
        let (state, user) = (State(self.state.clone()), Extension(self.user()));
        let path = || Path((app_id.to_string(), overlay_id.to_string()));
        let scope = || Query(ScopeParams::default());
        let mut answer = match operation {
            "cypher" => {
                let body: CypherPayload = from_args(payload, "cypher arguments")?;
                let rows = run_cypher(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("rows", rows.0)
            }
            "sql" => {
                let body: SqlPayload = from_args(payload, "sql arguments")?;
                let rows = run_sql(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("rows", rows.0)
            }
            "neighbors" => {
                let body: NeighborsPayload = from_args(payload, "neighbors arguments")?;
                let found = neighbors(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("result", to_json(&found.0)?)
            }
            "subgraph" => {
                let body: SubgraphPayload = from_args(payload, "subgraph arguments")?;
                let found = subgraph(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("result", to_json(&found.0)?)
            }
            "paths" => {
                let body: PathsPayload = from_args(payload, "paths arguments")?;
                let found = find_paths(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("result", to_json(&found.0)?)
            }
            "analytics" => {
                let found = graph_analytics(
                    state,
                    user,
                    path(),
                    Query(AnalyticsParams {
                        scope: None,
                        limit: Some(limit as usize),
                    }),
                )
                .await
                .map_err(failed)?;
                ok_with("analytics", to_json(&found.0)?)
            }
            "search_nodes" => {
                let body: SearchNodesPayload = from_args(payload, "search arguments")?;
                let found = search_nodes(state, user, path(), scope(), Json(body))
                    .await
                    .map_err(failed)?;
                ok_with("nodes", to_json(&found.0)?)
            }
            "sample" => {
                let label = required(args, "label")?.to_string();
                let n = bounded(args, "n", 10, MAX_SAMPLE_ROWS) as usize;
                let found = sample_nodes(
                    state,
                    user,
                    path(),
                    Query(SampleParams {
                        scope: None,
                        label,
                        n,
                    }),
                )
                .await
                .map_err(failed)?;
                ok_with("rows", json!(found.0))
            }
            other => return Err(format!("Unsupported graph_query_tool operation '{other}'.")),
        };
        for key in ["rows", "nodes"] {
            if cap(&mut answer, key, limit as usize) {
                answer["truncated"] = json!(true);
            }
        }
        if let Some(result) = answer.get_mut("result") {
            bound_graph_result(result);
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(operation: &str, args: Value) -> String {
        parse_change(operation, &args)
            .err()
            .unwrap_or_else(|| panic!("{operation} was accepted"))
    }

    #[test]
    fn memory_tables_are_never_changed() {
        for table in ["facts", "Facts", "facts_expiry"] {
            for operation in ["create_table", "insert", "delete_table", "drop_columns"] {
                let args = json!({
                    "table_name": table,
                    "items": [{ "a": 1 }],
                    "columns": ["a"],
                    "fields": [],
                    "confirm_table_name": table,
                });
                assert!(
                    refusal(operation, args).contains("holds the bot's memory"),
                    "{operation} on {table}"
                );
            }
        }
        assert!(parse_change("optimize", &json!({ "table_name": "notes" })).is_ok());
    }

    #[test]
    fn unknown_and_unsupported_operations_are_refused_by_name() {
        let args = json!({ "table_name": "notes" });
        assert!(refusal("truncate", args.clone()).contains("'truncate'"));
        assert!(refusal("import_geojson", args).contains("import_geojson"));
    }

    #[test]
    fn arguments_are_validated_per_operation() {
        let table = |extra: Value| {
            let mut args = json!({ "table_name": "notes" });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            args
        };
        assert!(refusal("insert", table(json!({ "items": [] }))).contains("items"));
        assert!(refusal("update", table(json!({ "filter": "a = 1" }))).contains("updates"));
        assert!(refusal("delete", table(json!({}))).contains("filter"));
        assert!(refusal("alter_column", table(json!({ "column": "a" }))).contains("nullable"));
        assert!(refusal("drop_columns", table(json!({ "columns": [] }))).contains("columns"));
        assert!(
            refusal(
                "add_column",
                table(json!({ "column_definition": { "name": "a" } }))
            )
            .contains("exactly one")
        );
        assert!(
            refusal(
                "set_column_metadata",
                table(json!({ "column": "a", "column_metadata": { "colour": "red" } }))
            )
            .contains("'colour'")
        );
        assert!(
            refusal(
                "build_index",
                table(json!({ "column": "a", "index_type": "Nope" }))
            )
            .contains("Nope")
        );
        assert!(
            refusal(
                "delete_table",
                table(json!({ "confirm_table_name": "other" }))
            )
            .contains("Nothing was deleted")
        );
        assert!(matches!(
            parse_change(
                "delete_table",
                &table(json!({ "confirm_table_name": "notes" }))
            ),
            Ok((_, TableChange::DropTable))
        ));
        assert!(matches!(
            parse_change(
                "build_index",
                &table(json!({ "column": "a", "index_type": "btree" }))
            ),
            Ok((_, TableChange::BuildIndex(_)))
        ));
    }

    #[test]
    fn graph_payloads_keep_only_their_keys_and_bound_numbers() {
        let args = json!({
            "label": "Person",
            "node_id": 7,
            "depth": 99,
            "query": "ignored",
            "n": 5000,
            "limit": 9000,
        });
        let neighbors = graph_payload("neighbors", &args, 200).unwrap();
        assert_eq!(neighbors["depth"], 5);
        assert_eq!(neighbors["limit"], 200);
        assert!(neighbors.get("query").is_none());
        let sample = graph_payload("sample", &args, 200).unwrap();
        assert_eq!(sample["n"], MAX_SAMPLE_ROWS);
        assert!(
            graph_payload("drop", &args, 200)
                .unwrap_err()
                .contains("'drop'")
        );
    }

    #[test]
    fn graph_results_are_capped_and_marked_truncated() {
        let mut result = json!({
            "nodes": vec![json!({}); MAX_GRAPH_NODES + 3],
            "edges": vec![json!({}); 2],
        });
        bound_graph_result(&mut result);
        assert_eq!(result["nodes"].as_array().unwrap().len(), MAX_GRAPH_NODES);
        assert_eq!(result["truncated"], true);
        let mut small = json!({ "nodes": [json!({})] });
        bound_graph_result(&mut small);
        assert!(small.get("truncated").is_none());
    }
}
