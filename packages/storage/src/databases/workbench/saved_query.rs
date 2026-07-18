//! Persistence for Data Studio saved queries and views. Stored as one row per
//! artifact in the reserved `__saved_queries__` table, mirroring the graph
//! overlay / ontology import pattern: a `definition_json` blob plus a few
//! denormalized scalar columns for cheap filtering, upserted by `id`.

use crate::arrow_utils::record_batch_to_value;
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use lancedb::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const SAVED_QUERIES_TABLE: &str = "__saved_queries__";

/// A stored query or view. `kind` distinguishes a runnable (optionally
/// parametrized) query from a composable view usable as a named virtual table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQueryDef {
    pub id: String,
    pub app_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: SavedQueryKind,
    pub surface: SavedQuerySurface,
    #[serde(default)]
    pub overlay_id: Option<String>,
    pub sql: String,
    /// Opaque JSON-Schema-shaped parameter definition owned by the UI. The
    /// backend never interprets it; parameter values are bound from the execute
    /// payload at run time.
    #[serde(default)]
    pub param_schema: Option<Value>,
    /// Opaque chart/graph configuration owned by the UI.
    #[serde(default)]
    pub viz_config: Option<Value>,
    #[serde(default)]
    pub default_limit: Option<usize>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQueryKind {
    Query,
    View,
}

impl SavedQueryKind {
    fn as_str(self) -> &'static str {
        match self {
            SavedQueryKind::Query => "query",
            SavedQueryKind::View => "view",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedQuerySurface {
    Native,
    Overlay,
}

impl SavedQuerySurface {
    fn as_str(self) -> &'static str {
        match self {
            SavedQuerySurface::Native => "native",
            SavedQuerySurface::Overlay => "overlay",
        }
    }
}

pub async fn list_saved_queries(connection: &Connection) -> Result<Vec<SavedQueryDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        return Ok(Vec::new());
    }

    let table = connection
        .open_table(SAVED_QUERIES_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open saved queries table: {}", e))?;
    let result = table
        .query()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query saved queries: {}", e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect saved queries: {}", e))?;

    let mut queries: Vec<SavedQueryDef> = Vec::new();
    for batch in &batches {
        for row in record_batch_to_value(batch)? {
            if let Some(definition) = row.get("definition_json").and_then(|value| value.as_str()) {
                match serde_json::from_str(definition) {
                    Ok(query) => queries.push(query),
                    Err(error) => {
                        let query_id = row
                            .get("id")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<unknown>");
                        tracing::warn!(
                            %error,
                            query_id,
                            "Skipping saved query with unparseable definition"
                        );
                    }
                }
            }
        }
    }
    queries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(queries)
}

pub async fn find_saved_query(
    connection: &Connection,
    query_id: &str,
) -> Result<Option<SavedQueryDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        return Ok(None);
    }

    let table = connection
        .open_table(SAVED_QUERIES_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open saved queries table: {}", e))?;
    let filter = format!("id = '{}'", query_id.replace('\'', "''"));
    let result = table
        .query()
        .only_if(filter)
        .limit(1)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query saved query '{}': {}", query_id, e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect saved query '{}': {}", query_id, e))?;

    for batch in &batches {
        for row in record_batch_to_value(batch)? {
            if let Some(definition) = row.get("definition_json").and_then(|value| value.as_str()) {
                return serde_json::from_str(definition)
                    .map(Some)
                    .map_err(|e| anyhow!("Failed to parse saved query: {}", e));
            }
        }
    }
    Ok(None)
}

pub async fn load_saved_query(connection: &Connection, query_id: &str) -> Result<SavedQueryDef> {
    find_saved_query(connection, query_id)
        .await?
        .ok_or_else(|| anyhow!("Saved query '{}' not found", query_id))
}

fn saved_query_batch(
    query: &SavedQueryDef,
) -> Result<(Arc<arrow::datatypes::Schema>, arrow::array::RecordBatch)> {
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("app_id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("surface", DataType::Utf8, false),
        Field::new("definition_json", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ]));
    let definition_json = serde_json::to_string(query)
        .map_err(|e| anyhow!("Failed to serialize saved query: {}", e))?;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![query.id.as_str()])),
            Arc::new(StringArray::from(vec![query.app_id.as_str()])),
            Arc::new(StringArray::from(vec![query.name.as_str()])),
            Arc::new(StringArray::from(vec![query.kind.as_str()])),
            Arc::new(StringArray::from(vec![query.surface.as_str()])),
            Arc::new(StringArray::from(vec![definition_json.as_str()])),
            Arc::new(StringArray::from(vec![query.updated_at.as_str()])),
        ],
    )?;
    Ok((schema, batch))
}

pub async fn save_saved_query(connection: &Connection, query: &SavedQueryDef) -> Result<()> {
    let (schema, batch) = saved_query_batch(query)?;

    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;
    if table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        let table = connection
            .open_table(SAVED_QUERIES_TABLE)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open saved queries table: {}", e))?;
        let mut merger = table.merge_insert(&["id"]);
        merger
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
            arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
        );
        merger
            .execute(reader)
            .await
            .map_err(|e| anyhow!("Failed to upsert saved query: {}", e))?;
    } else {
        connection
            .create_table(SAVED_QUERIES_TABLE, vec![batch])
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to create saved queries table: {}", e))?;
    }
    Ok(())
}

/// Atomically updates an existing saved query only when its persisted revision
/// still matches the one the caller loaded. Returns `false` on a revision
/// mismatch (a concurrent write won the race).
pub async fn save_saved_query_if_unchanged(
    connection: &Connection,
    query: &SavedQueryDef,
    expected_updated_at: &str,
) -> Result<bool> {
    let (schema, batch) = saved_query_batch(query)?;
    let table = connection
        .open_table(SAVED_QUERIES_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open saved queries table: {}", e))?;
    let expected = expected_updated_at.replace('\'', "''");
    let mut merger = table.merge_insert(&["id"]);
    merger.when_matched_update_all(Some(format!("target.updated_at = '{expected}'")));
    let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
        arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
    );
    let result = merger
        .execute(reader)
        .await
        .map_err(|e| anyhow!("Failed to conditionally update saved query: {}", e))?;
    Ok(result.num_updated_rows == 1)
}

pub async fn delete_saved_query(connection: &Connection, query_id: &str) -> Result<()> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;
    if !table_names.iter().any(|name| name == SAVED_QUERIES_TABLE) {
        return Ok(());
    }

    let table = connection
        .open_table(SAVED_QUERIES_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open saved queries table: {}", e))?;
    table
        .delete(&format!("id = '{}'", query_id.replace('\'', "''")))
        .await
        .map_err(|e| anyhow!("Failed to delete saved query: {}", e))?;
    Ok(())
}
