//! One LanceDB table behind a lock: created on first write from an explicit schema, read as
//! JSON rows, with a full-text (BM25) index on one column. Tables with vectors keep them in a
//! nullable `vector` column next to `embedding_model`, the model that made the row's vector
//! (empty while the row has none), so rows never wait for an embedder.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use flow_like_storage::arrow_array::RecordBatch;
use flow_like_storage::arrow_schema::{DataType, Field, Schema};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::lance::dataset::WriteMode;
use flow_like_storage::lancedb;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::tokio::sync::{Mutex, MutexGuard, RwLock};
use flow_like_types::{Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::llm::{Embedder, embed_checked};

/// Texts embedded per call while re-embedding.
const REEMBED_BATCH: usize = 64;

/// A SQL string literal.
pub(crate) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `column IN ('a', 'b')`.
pub(crate) fn in_list<'a>(column: &str, values: impl IntoIterator<Item = &'a str>) -> String {
    let quoted: Vec<String> = values.into_iter().map(quote).collect();
    format!("{column} IN ({})", quoted.join(", "))
}

pub(crate) fn millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

pub(crate) fn from_millis(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value).unwrap_or_default()
}

pub(crate) fn text_field(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Utf8, nullable)
}

pub(crate) fn int_field(name: &str, nullable: bool) -> Field {
    Field::new(name, DataType::Int64, nullable)
}

fn vector_len(row: &Value) -> Option<usize> {
    row.get("vector").and_then(Value::as_array).map(Vec::len)
}

/// The row without its vector, marked for re-embedding.
fn without_vector(row: &mut Value, has_column: bool) {
    if let Some(fields) = row.as_object_mut() {
        if has_column {
            fields.insert("vector".to_string(), Value::Null);
        } else {
            fields.remove("vector");
        }
        fields.insert("embedding_model".to_string(), json!(""));
    }
}

pub(crate) struct Table {
    connection: Connection,
    name: &'static str,
    write_options: Option<WriteOptions>,
    store: RwLock<LanceDBVectorStore>,
    fts_column: Option<&'static str>,
    fts_ready: AtomicBool,
    writes: Mutex<()>,
}

impl Table {
    async fn connect(
        connection: &Connection,
        name: &'static str,
        write_options: Option<&WriteOptions>,
    ) -> LanceDBVectorStore {
        let mut store =
            LanceDBVectorStore::from_connection(connection.clone(), name.to_string()).await;
        if let Some(options) = write_options {
            store.set_write_options(options.clone());
        }
        store
    }

    pub(crate) async fn open(
        connection: Connection,
        name: &'static str,
        fts_column: Option<&'static str>,
        write_options: Option<WriteOptions>,
    ) -> Self {
        let store = Self::connect(&connection, name, write_options.as_ref()).await;
        Self {
            connection,
            name,
            write_options,
            store: RwLock::new(store),
            fts_column,
            fts_ready: AtomicBool::new(false),
            writes: Mutex::new(()),
        }
    }

    /// Held by the store's writes, so a re-embed never rewrites a row from a stale read.
    pub(crate) async fn write_lock(&self) -> MutexGuard<'_, ()> {
        self.writes.lock().await
    }

    /// Whether the table exists, picking up a table another handle created meanwhile.
    pub(crate) async fn exists(&self) -> Result<bool> {
        if self.store.read().await.table_exists().await? {
            return Ok(true);
        }
        let reopened =
            Self::connect(&self.connection, self.name, self.write_options.as_ref()).await;
        if !reopened.table_exists().await? {
            return Ok(false);
        }
        *self.store.write().await = reopened;
        Ok(true)
    }

    /// Picks up versions other handles wrote; connections read lazily.
    pub(crate) async fn refresh(&self) -> Result<()> {
        if !self.exists().await? {
            return Ok(());
        }
        let table = self.store.read().await.raw().await?;
        table.checkout_latest().await?;
        Ok(())
    }

    pub(crate) async fn create(&self, schema: Schema) -> Result<()> {
        if self.exists().await? {
            return Ok(());
        }
        self.store
            .write()
            .await
            .create_empty_table(schema, true)
            .await?;
        Ok(())
    }

    /// Creates the table with its first rows in one commit; another creator winning is fine.
    pub(crate) async fn create_with(&self, rows: RecordBatch) -> Result<()> {
        if self.exists().await? {
            return Ok(());
        }
        let mut options = self.write_options.clone().unwrap_or_default();
        options
            .lance_write_params
            .get_or_insert_with(Default::default)
            .mode = WriteMode::Create;
        match self
            .connection
            .create_table(self.name, rows)
            .write_options(options)
            .execute()
            .await
        {
            Ok(_) | Err(lancedb::Error::TableAlreadyExists { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        self.exists().await?;
        Ok(())
    }

    pub(crate) async fn drop_table(&self) -> Result<()> {
        if self.exists().await? {
            self.store.write().await.drop_table().await?;
        }
        self.fts_ready.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn upsert_values(&self, values: Vec<Value>) -> Result<()> {
        if values.is_empty() {
            return Ok(());
        }
        self.store
            .write()
            .await
            .upsert(values, "id".to_string())
            .await?;
        self.ensure_fts().await
    }

    /// Inserts or replaces rows by `id`, then makes sure the full-text index exists.
    pub(crate) async fn upsert<T: Serialize>(&self, rows: &[T]) -> Result<()> {
        let values = rows
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        self.upsert_values(values).await
    }

    /// Inserts or replaces rows by `id`, creating the table from `schema` (without a vector
    /// column) first. A row keeps its `vector` only when it fits the vector column, which the
    /// first vector creates; any other row is stored without one and with an empty
    /// `embedding_model`, for [`Table::reembed`].
    pub(crate) async fn upsert_vectored<T: Serialize>(
        &self,
        schema: Schema,
        rows: &[T],
    ) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut values = rows
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        self.create(schema).await?;
        let mut column = self.vector_dimension().await?;
        if column.is_none()
            && let Some(size) = values.iter().find_map(vector_len)
        {
            self.add_vector_column(size).await?;
            column = Some(size);
        }
        for value in &mut values {
            if column.is_none() || vector_len(value) != column {
                without_vector(value, column.is_some());
            }
        }
        self.upsert_values(values).await
    }

    async fn ensure_fts(&self) -> Result<()> {
        let Some(column) = self.fts_column else {
            return Ok(());
        };
        if self.fts_ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        let store = self.store.read().await;
        let indexed = store
            .list_indices()
            .await?
            .iter()
            .any(|index| index.columns.iter().any(|indexed| indexed == column));
        if !indexed {
            store.index(column, Some("FTS")).await?;
        }
        self.fts_ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) async fn update(&self, filter: &str, values: HashMap<String, Value>) -> Result<()> {
        self.store.read().await.update(filter, values).await
    }

    pub(crate) async fn delete(&self, filter: &str) -> Result<()> {
        if !self.exists().await? {
            return Ok(());
        }
        self.store.read().await.delete(filter).await
    }

    /// The vector column's size; `None` while the table has none.
    pub(crate) async fn vector_dimension(&self) -> Result<Option<usize>> {
        if !self.exists().await? {
            return Ok(None);
        }
        let schema = self.store.read().await.schema().await?;
        Ok(
            match schema.field_with_name("vector").map(Field::data_type) {
                Ok(DataType::FixedSizeList(_, size)) => usize::try_from(*size).ok(),
                _ => None,
            },
        )
    }

    async fn add_vector_column(&self, size: usize) -> Result<()> {
        self.store
            .read()
            .await
            .add_typed_column("vector", "vector", Some(u32::try_from(size)?))
            .await
    }

    /// Gives the vector column `size` values. Rows stay; vectors of another size leave with the
    /// old column, after their rows were marked for re-embedding, so a crash in between loses
    /// vectors only.
    async fn ensure_vector_column(&self, size: usize) -> Result<()> {
        match self.vector_dimension().await? {
            Some(existing) if existing == size => Ok(()),
            Some(_) => {
                self.update(
                    "embedding_model != ''",
                    HashMap::from([("embedding_model".to_string(), json!(""))]),
                )
                .await?;
                self.store.read().await.drop_columns(&["vector"]).await?;
                self.add_vector_column(size).await
            }
            None => self.add_vector_column(size).await,
        }
    }

    /// Rows whose vector comes from another model than `model`.
    pub(crate) async fn stale_vectors(&self, model: &str) -> Result<usize> {
        self.count(Some(format!(
            "embedding_model != '' AND embedding_model != {}",
            quote(model)
        )))
        .await
    }

    /// Rows without a vector from `model`.
    pub(crate) async fn unembedded(&self, model: &str) -> Result<usize> {
        self.count(Some(format!("embedding_model != {}", quote(model))))
            .await
    }

    /// Whether a vector search for a query of `size` values can run: no row holds another
    /// model's vector and the column has that size. Rows still without one are left to BM25.
    pub(crate) async fn vectors_ready(&self, model: &str, size: usize) -> Result<bool> {
        Ok(self.stale_vectors(model).await? == 0 && self.vector_dimension().await? == Some(size))
    }

    /// Embeds the `text` of every row without a vector from the embedder's model, in batches;
    /// each batch is written from a fresh read under the write lock, and a row whose text
    /// changed meanwhile waits for the next run. The rows themselves are never removed.
    pub(crate) async fn reembed(&self, embedder: &dyn Embedder, columns: &[&str]) -> Result<usize> {
        let model = embedder.model_id();
        let waiting: Vec<Value> = self
            .all_rows(&format!("embedding_model != {}", quote(model)), columns)
            .await?;
        let mut size = None;
        let mut written = 0;
        for batch in waiting.chunks(REEMBED_BATCH) {
            let text = |row: &Value| row["text"].as_str().unwrap_or_default().to_string();
            let texts: Vec<String> = batch.iter().map(text).collect();
            let vectors = embed_checked(embedder, &texts).await?;
            let dimension = vectors.first().map_or(0, Vec::len);
            if let Some(first) = size
                && first != dimension
            {
                bail!("Embedder '{model}' returned vectors of {first} values, then of {dimension}");
            }
            size = Some(dimension);
            let _writes = self.write_lock().await;
            self.ensure_vector_column(dimension).await?;
            let ids = batch.iter().filter_map(|row| row["id"].as_str());
            let fresh: Vec<Value> = self.all_rows(&in_list("id", ids), columns).await?;
            let mut rows = Vec::new();
            for (row, vector) in batch.iter().zip(vectors) {
                let Some(mut current) = fresh
                    .iter()
                    .find(|current| current["id"] == row["id"])
                    .cloned()
                else {
                    continue;
                };
                if text(&current) != text(row) {
                    continue;
                }
                current["vector"] = json!(vector);
                current["embedding_model"] = json!(model);
                rows.push(current);
            }
            written += rows.len();
            self.upsert_values(rows).await?;
        }
        Ok(written)
    }

    pub(crate) async fn count(&self, filter: Option<String>) -> Result<usize> {
        if !self.exists().await? {
            return Ok(0);
        }
        self.store.read().await.count(filter).await
    }

    pub(crate) async fn rows<T: DeserializeOwned>(
        &self,
        filter: &str,
        columns: &[&str],
        limit: usize,
        offset: usize,
    ) -> Result<Vec<T>> {
        if !self.exists().await? {
            return Ok(Vec::new());
        }
        let rows = self
            .store
            .read()
            .await
            .filter(filter, Some(owned(columns)), limit, offset)
            .await?;
        decode(rows)
    }

    /// Every row matching `filter`, read in pages.
    pub(crate) async fn all_rows<T: DeserializeOwned>(
        &self,
        filter: &str,
        columns: &[&str],
    ) -> Result<Vec<T>> {
        const PAGE: usize = 500;
        let mut rows = Vec::new();
        loop {
            let page: Vec<T> = self.rows(filter, columns, PAGE, rows.len()).await?;
            let done = page.len() < PAGE;
            rows.extend(page);
            if done {
                return Ok(rows);
            }
        }
    }

    /// Nearest rows by cosine distance, with `_distance`.
    pub(crate) async fn nearest(
        &self,
        vector: &[f32],
        filter: Option<&str>,
        columns: &[&str],
        limit: usize,
    ) -> Result<Vec<Value>> {
        if !self.exists().await? {
            return Ok(Vec::new());
        }
        let vector: Vec<f64> = vector.iter().map(|value| f64::from(*value)).collect();
        self.store
            .read()
            .await
            .vector_search(vector, filter, Some(owned(columns)), limit, 0)
            .await
    }

    /// BM25 matches of the full-text column, best first. A missing index or a query the
    /// tokenizer rejects yields no matches rather than an error, so vector search still serves.
    pub(crate) async fn full_text(
        &self,
        text: &str,
        filter: Option<&str>,
        columns: &[&str],
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let Some(column) = self.fts_column else {
            return Ok(Vec::new());
        };
        if text.trim().is_empty() || !self.exists().await? {
            return Ok(Vec::new());
        }
        if let Err(error) = self.ensure_fts().await {
            tracing::debug!("No full-text index on {}.{column}: {error:#}", self.name);
            return Ok(Vec::new());
        }
        let result = self
            .store
            .read()
            .await
            .fts_search(
                text,
                filter,
                Some(owned(columns)),
                Some(vec![column.to_string()]),
                limit,
                offset,
            )
            .await;
        match result {
            Ok(rows) => Ok(rows),
            Err(error) => {
                tracing::debug!("Full-text search on {} failed: {error:#}", self.name);
                Ok(Vec::new())
            }
        }
    }

    /// Folds new rows into the indexes and drops versions older than Lance's 7-day retention.
    pub(crate) async fn optimize(&self) -> Result<()> {
        if !self.exists().await? {
            return Ok(());
        }
        self.store.read().await.optimize(false).await
    }

    /// Like [`Table::optimize`], but drops every older version at once, so deleted rows leave
    /// the disk now.
    pub(crate) async fn prune(&self) -> Result<()> {
        if !self.exists().await? {
            return Ok(());
        }
        self.store.read().await.prune_history().await
    }
}

fn owned(columns: &[&str]) -> Vec<String> {
    columns.iter().map(|column| column.to_string()).collect()
}

pub(crate) fn decode<T: DeserializeOwned>(rows: Vec<Value>) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| serde_json::from_value(row).map_err(Into::into))
        .collect()
}

/// `_distance` of a vector hit as cosine similarity; 0 for a BM25 hit.
pub(crate) fn similarity(row: &Value) -> f32 {
    row.get("_distance")
        .and_then(Value::as_f64)
        .map_or(0.0, |distance| (1.0 - distance) as f32)
}

/// The ids of `hits` in rank order; each row is kept the first time its id is seen.
pub(crate) fn ranked_ids(hits: Vec<Value>, rows: &mut HashMap<String, Value>) -> Vec<String> {
    let mut ids = Vec::new();
    for hit in hits {
        if let Some(id) = hit.get("id").and_then(Value::as_str).map(str::to_string) {
            rows.entry(id.clone()).or_insert(hit);
            ids.push(id);
        }
    }
    ids
}

/// Candidates fetched from each ranked list. It depends on the page size only, so the pages of
/// one search share one ranking and neither repeat nor skip a row.
pub(crate) fn search_pool(limit: usize) -> usize {
    limit.saturating_mul(10).clamp(30, 300)
}

/// One hybrid search: the query text, its vector when a vector search can run, the model the
/// vectors must come from, the columns to read and how many hits each list fetches.
pub(crate) struct Probe<'a> {
    pub text: &'a str,
    pub vector: Option<&'a [f32]>,
    pub model: &'a str,
    pub columns: &'a [&'a str],
    pub pool: usize,
}

impl Table {
    /// The query's vector when a vector search can run, else `None` (BM25 serves).
    pub(crate) async fn query_vector(
        &self,
        embedder: &dyn Embedder,
        text: &str,
    ) -> Result<Option<Vec<f32>>> {
        if self.stale_vectors(embedder.model_id()).await? > 0 {
            return Ok(None);
        }
        let vector = match embedder.embed_query(text).await {
            Ok(vector) => vector,
            Err(error) => {
                tracing::warn!("Embedding a memory query failed; using BM25 only: {error:#}");
                return Ok(None);
            }
        };
        Ok((self.vector_dimension().await? == Some(vector.len())).then_some(vector))
    }

    /// The vector and BM25 rankings under `filter`; vector hits come from rows the probe's
    /// model embedded.
    pub(crate) async fn rankings(
        &self,
        probe: &Probe<'_>,
        filter: &str,
        rows: &mut HashMap<String, Value>,
    ) -> Result<Vec<Vec<String>>> {
        let mut lists = Vec::new();
        if let Some(vector) = probe.vector {
            let embedded = format!("({filter}) AND embedding_model = {}", quote(probe.model));
            let hits = self
                .nearest(vector, Some(&embedded), probe.columns, probe.pool)
                .await?;
            lists.push(ranked_ids(hits, rows));
        }
        let hits = self
            .full_text(probe.text, Some(filter), probe.columns, probe.pool, 0)
            .await?;
        lists.push(ranked_ids(hits, rows));
        Ok(lists)
    }
}

/// The vectors for `texts`, or `None` for each when the embedder fails: the rows are then
/// stored without vectors and re-embedded while idle.
pub(crate) async fn embed_or_wait(
    embedder: &dyn Embedder,
    texts: &[String],
) -> Vec<Option<Vec<f32>>> {
    match embed_checked(embedder, texts).await {
        Ok(vectors) => vectors.into_iter().map(Some).collect(),
        Err(error) => {
            tracing::warn!(
                "Embedding {} memory rows failed; they are searchable by BM25 until idle maintenance embeds them: {error:#}",
                texts.len()
            );
            vec![None; texts.len()]
        }
    }
}
