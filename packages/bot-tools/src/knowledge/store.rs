use std::collections::BTreeMap;

use flow_like_storage::arrow_schema::{DataType, Field, Schema};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ToolError;

pub const META_TABLE: &str = "kbmeta";
pub const TABLE_PREFIX: &str = "kb_";
pub const SOURCES_TABLE: &str = "__kb_sources";
pub const FOLDERS_TABLE: &str = "__kb_folders";
const RECORD_SCAN: usize = 100_000;
const MAX_NAME: usize = 40;
const SOURCE_SCAN: usize = 20_000;

pub fn table_name(index: &str) -> String {
    format!("{TABLE_PREFIX}{index}")
}

pub fn validate_name(name: &str) -> Result<String, ToolError> {
    let name = name.trim().to_lowercase();
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(name)
    } else {
        Err(ToolError::invalid(format!(
            "'{name}' is not a valid index name: use 1 to {MAX_NAME} characters of a-z, 0-9 and '_', starting with a letter"
        )))
    }
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexMeta {
    pub id: String,
    pub description: String,
    pub embedding_model: String,
    pub dimension: i64,
    pub created_at: i64,
}

/// What was indexed of one file read through a folder: the listing's size and modified time
/// (`-1` when the host gives none) and the content hash that decides a re-index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub kb_index: String,
    pub source: String,
    pub bytes: i64,
    pub modified_ms: i64,
    pub sha256: String,
    pub chunks: i64,
    pub indexed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderRecord {
    pub id: String,
    pub kb_index: String,
    pub app_id: String,
    pub prefix: String,
    pub last_sync_ms: i64,
    pub files: i64,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
}

pub fn source_record_id(index: &str, source: &str) -> String {
    format!("{index}\0{source}")
}

pub fn folder_record_id(index: &str, app_id: &str, prefix: &str) -> String {
    format!("{index}\0{app_id}/{prefix}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRow {
    pub id: String,
    pub text: String,
    pub source: String,
    pub handle: String,
    pub page: Option<i64>,
    pub start_offset: i64,
    pub end_offset: i64,
    pub created_at: i64,
    pub classification: String,
    pub embedding_model: String,
    pub vector: Vec<f32>,
}

pub const HIT_COLUMNS: [&str; 9] = [
    "id",
    "text",
    "source",
    "handle",
    "page",
    "start_offset",
    "end_offset",
    "created_at",
    "classification",
];

fn text_field(name: &str) -> Field {
    Field::new(name, DataType::Utf8, false)
}

fn chunk_schema() -> Schema {
    Schema::new(vec![
        text_field("id"),
        text_field("text"),
        text_field("source"),
        text_field("handle"),
        Field::new("page", DataType::Int64, true),
        Field::new("start_offset", DataType::Int64, false),
        Field::new("end_offset", DataType::Int64, false),
        Field::new("created_at", DataType::Int64, false),
        text_field("classification"),
        text_field("embedding_model"),
    ])
}

/// The workspace's knowledge indexes: one LanceDB table `kb_<name>` per index and a small
/// `kbmeta` table naming each index's embedding model.
pub struct Knowledge {
    connection: Connection,
}

impl Knowledge {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    async fn store(&self, table: &str) -> LanceDBVectorStore {
        LanceDBVectorStore::from_connection(self.connection.clone(), table.to_string()).await
    }

    pub async fn meta(&self, name: &str) -> Result<Option<IndexMeta>> {
        let store = self.store(META_TABLE).await;
        if !store.table_exists().await? {
            return Ok(None);
        }
        let rows = store
            .filter(&format!("id = {}", quote(name)), None, 1, 0)
            .await?;
        rows.into_iter()
            .next()
            .map(|row| serde_json::from_value(row).map_err(Into::into))
            .transpose()
    }

    pub async fn all_meta(&self) -> Result<Vec<IndexMeta>> {
        let store = self.store(META_TABLE).await;
        if !store.table_exists().await? {
            return Ok(Vec::new());
        }
        let rows = store.filter("1 = 1", None, 1000, 0).await?;
        let mut metas: Vec<IndexMeta> = rows
            .into_iter()
            .map(|row| serde_json::from_value(row).map_err(Into::into))
            .collect::<Result<_>>()?;
        metas.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(metas)
    }

    pub async fn create(&self, meta: &IndexMeta) -> Result<()> {
        let mut table = self.store(&table_name(&meta.id)).await;
        table.create_empty_table(chunk_schema(), true).await?;
        let size = u32::try_from(meta.dimension)?;
        let has_vector = table.schema().await?.field_with_name("vector").is_ok();
        if !has_vector {
            table
                .add_typed_column("vector", "vector", Some(size))
                .await?;
        }
        let mut meta_store = self.store(META_TABLE).await;
        meta_store
            .upsert(vec![serde_json::to_value(meta)?], "id".to_string())
            .await
    }

    pub async fn drop(&self, name: &str) -> Result<()> {
        let mut table = self.store(&table_name(name)).await;
        if table.table_exists().await? {
            table.drop_table().await?;
        }
        let meta_store = self.store(META_TABLE).await;
        if meta_store.table_exists().await? {
            meta_store.delete(&format!("id = {}", quote(name))).await?;
        }
        for bookkeeping in [SOURCES_TABLE, FOLDERS_TABLE] {
            let records = self.store(bookkeeping).await;
            if records.table_exists().await? {
                records
                    .delete(&format!("kb_index = {}", quote(name)))
                    .await?;
            }
        }
        Ok(())
    }

    async fn records<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
        index: &str,
    ) -> Result<Vec<T>> {
        let store = self.store(table).await;
        if !store.table_exists().await? {
            return Ok(Vec::new());
        }
        store
            .filter(
                &format!("kb_index = {}", quote(index)),
                None,
                RECORD_SCAN,
                0,
            )
            .await?
            .into_iter()
            .map(|row| serde_json::from_value(row).map_err(Into::into))
            .collect()
    }

    async fn put_record<T: Serialize>(&self, table: &str, record: &T) -> Result<()> {
        let mut store = self.store(table).await;
        store
            .upsert(vec![serde_json::to_value(record)?], "id".to_string())
            .await
    }

    pub async fn source_records(&self, index: &str) -> Result<BTreeMap<String, SourceRecord>> {
        let records: Vec<SourceRecord> = self.records(SOURCES_TABLE, index).await?;
        Ok(records
            .into_iter()
            .map(|record| (record.source.clone(), record))
            .collect())
    }

    pub async fn put_source(&self, record: &SourceRecord) -> Result<()> {
        self.put_record(SOURCES_TABLE, record).await
    }

    pub async fn folders(&self, index: &str) -> Result<Vec<FolderRecord>> {
        let mut folders: Vec<FolderRecord> = self.records(FOLDERS_TABLE, index).await?;
        folders.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(folders)
    }

    pub async fn put_folder(&self, record: &FolderRecord) -> Result<()> {
        self.put_record(FOLDERS_TABLE, record).await
    }

    /// Removes a file's chunks and its change-detection record; returns the chunks removed.
    pub async fn remove_source(&self, index: &str, source: &str) -> Result<usize> {
        let removed = self.replace_source(index, source, &[]).await?;
        let records = self.store(SOURCES_TABLE).await;
        if records.table_exists().await? {
            records
                .delete(&format!("id = {}", quote(&source_record_id(index, source))))
                .await?;
        }
        Ok(removed)
    }

    pub async fn replace_source(
        &self,
        name: &str,
        source: &str,
        rows: &[ChunkRow],
    ) -> Result<usize> {
        let mut table = self.store(&table_name(name)).await;
        let previous = table
            .count(Some(format!("source = {}", quote(source))))
            .await?;
        if previous > 0 {
            table.delete(&format!("source = {}", quote(source))).await?;
        }
        let values = rows
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        if !values.is_empty() {
            table.upsert(values, "id".to_string()).await?;
            self.ensure_text_index(&table).await?;
        }
        Ok(previous)
    }

    async fn ensure_text_index(&self, table: &LanceDBVectorStore) -> Result<()> {
        let indexed = table
            .list_indices()
            .await?
            .iter()
            .any(|index| index.columns.iter().any(|column| column == "text"));
        if !indexed {
            table.index("text", Some("FTS")).await?;
        }
        Ok(())
    }

    pub async fn chunk_count(&self, name: &str) -> Result<usize> {
        let table = self.store(&table_name(name)).await;
        if !table.table_exists().await? {
            return Ok(0);
        }
        table.count(None).await
    }

    /// Chunks per source, capped at a scan of [`SOURCE_SCAN`] rows.
    pub async fn sources(&self, name: &str) -> Result<BTreeMap<String, usize>> {
        let table = self.store(&table_name(name)).await;
        let mut counts = BTreeMap::new();
        if !table.table_exists().await? {
            return Ok(counts);
        }
        let rows = table
            .filter("1 = 1", Some(vec!["source".into()]), SOURCE_SCAN, 0)
            .await?;
        for row in rows {
            if let Some(source) = row.get("source").and_then(Value::as_str) {
                *counts.entry(source.to_string()).or_default() += 1;
            }
        }
        Ok(counts)
    }

    /// The vector and BM25 rankings for one index, best first. A failing text search (no
    /// index yet, a query the tokenizer rejects) leaves the vector ranking to serve.
    pub async fn rankings(
        &self,
        name: &str,
        query: &str,
        vector: &[f32],
        pool: usize,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let table = self.store(&table_name(name)).await;
        if !table.table_exists().await? {
            return Ok((Vec::new(), Vec::new()));
        }
        let select: Vec<String> = HIT_COLUMNS.iter().map(ToString::to_string).collect();
        let nearest = table
            .vector_search(
                vector.iter().map(|v| f64::from(*v)).collect(),
                None,
                Some(select.clone()),
                pool,
                0,
            )
            .await?;
        let text = if query.trim().is_empty() {
            Vec::new()
        } else {
            table
                .fts_search(
                    query,
                    None,
                    Some(select),
                    Some(vec!["text".into()]),
                    pool,
                    0,
                )
                .await
                .unwrap_or_default()
        };
        Ok((nearest, text))
    }
}

pub fn meta_row(
    name: &str,
    description: &str,
    model: &str,
    dimension: usize,
    now: i64,
) -> IndexMeta {
    IndexMeta {
        id: name.to_string(),
        description: description.to_string(),
        embedding_model: model.to_string(),
        dimension: dimension as i64,
        created_at: now,
    }
}
