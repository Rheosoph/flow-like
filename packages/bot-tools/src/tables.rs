use flow_like_bot_memory::store::is_memory_table;
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::databases::vector::VectorStore;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Result;
use serde_json::Value;

use crate::error::{ToolError, ToolResult};

pub const SOURCES_TABLE: &str = "__bot_sources__";
const MAX_TABLE_NAME: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    Replace,
    Append,
}

/// Tables that belong to the tools themselves: knowledge indexes and the source registry.
pub fn is_internal(name: &str) -> bool {
    name.starts_with("kb_") || name == "kbmeta" || name.starts_with("__")
}

pub fn validate_table_name(name: &str) -> ToolResult<String> {
    let name = name.trim().to_lowercase();
    if is_internal(&name) || is_memory_table(&name) {
        return Err(ToolError::refused(format!(
            "'{name}' is reserved for the bot's own indexes and registries; pick another table name"
        )));
    }
    let valid = !name.is_empty()
        && name.len() <= MAX_TABLE_NAME
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(ToolError::invalid(format!(
            "'{name}' is not a valid table name: use 1 to {MAX_TABLE_NAME} characters of a-z, 0-9 and '_', starting with a letter"
        )));
    }
    Ok(name)
}

pub fn ensure_not_memory_table(name: &str) -> ToolResult<()> {
    if is_memory_table(name) {
        return Err(ToolError::refused(
            "This table belongs to the bot's memory. Use its memory tools, which respect this conversation's sharing settings.",
        ));
    }
    Ok(())
}

pub async fn workspace_table_names(connection: &Connection) -> Result<Vec<String>> {
    Ok(table_names(connection)
        .await?
        .into_iter()
        .filter(|name| !is_memory_table(name))
        .collect())
}

async fn store(connection: &Connection, table: &str) -> LanceDBVectorStore {
    LanceDBVectorStore::from_connection(connection.clone(), table.to_string()).await
}

pub async fn table_names(connection: &Connection) -> Result<Vec<String>> {
    let mut names: Vec<String> = connection
        .table_names()
        .execute()
        .await?
        .into_iter()
        .filter(|name| !is_internal(name))
        .collect();
    names.sort();
    Ok(names)
}

pub async fn exists(connection: &Connection, table: &str) -> Result<bool> {
    store(connection, table).await.table_exists().await
}

pub async fn drop_table(connection: &Connection, table: &str) -> Result<()> {
    let mut table = store(connection, table).await;
    if table.table_exists().await? {
        table.drop_table().await?;
    }
    Ok(())
}

pub async fn save_rows(
    connection: &Connection,
    table: &str,
    rows: Vec<Value>,
    mode: SaveMode,
) -> Result<usize> {
    if rows.is_empty() {
        return Err(flow_like_types::anyhow!("There are no rows to save"));
    }
    if mode == SaveMode::Replace {
        drop_table(connection, table).await?;
    }
    let count = rows.len();
    store(connection, table).await.insert(rows).await?;
    Ok(count)
}

/// Replaces `table` with the batches (which share one schema).
pub async fn replace_with_batches(
    connection: &Connection,
    table: &str,
    schema: &Schema,
    batches: &[RecordBatch],
) -> Result<usize> {
    let batch = flow_like_storage::arrow::compute::concat_batches(&schema.clone().into(), batches)?;
    let rows = batch.num_rows();
    drop_table(connection, table).await?;
    store(connection, table)
        .await
        .insert_record_batch(batch)
        .await?;
    Ok(rows)
}

pub async fn upsert_json(connection: &Connection, table: &str, row: Value) -> Result<()> {
    store(connection, table)
        .await
        .upsert(vec![row], "id".to_string())
        .await
}

pub async fn all_json(connection: &Connection, table: &str) -> Result<Vec<Value>> {
    let table = store(connection, table).await;
    if !table.table_exists().await? {
        return Ok(Vec::new());
    }
    table.filter("1 = 1", None, 10_000, 0).await
}

pub async fn delete_where(connection: &Connection, table: &str, filter: &str) -> Result<()> {
    let table = store(connection, table).await;
    if table.table_exists().await? {
        table.delete(filter).await?;
    }
    Ok(())
}
