use arrow_array::RecordBatch;
use arrow_schema::{DataType, Schema};
use datafusion::catalog::TableProvider;
use datafusion::prelude::*;
use flow_like_types::Cacheable;
use flow_like_types::async_trait;
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use lancedb::index::IndexConfig;
use lancedb::index::scalar::BTreeIndexBuilder;
use lancedb::index::scalar::BitmapIndexBuilder;
use lancedb::index::scalar::LabelListIndexBuilder;
use lancedb::query::QueryExecutionOptions;
use lancedb::table::AddColumnsResult;
use lancedb::table::AlterColumnsResult;
use lancedb::table::ColumnAlteration;
use lancedb::table::NewColumnTransform;
use lancedb::table::WriteOptions;
use lancedb::{
    Connection, Table, connect,
    index::{
        Index,
        scalar::{FtsIndexBuilder, FullTextSearchQuery},
    },
    query::{ExecutableQuery, QueryBase},
    table::{CompactionOptions, Duration, OptimizeOptions},
};

use std::{any::Any, path::PathBuf, sync::Arc};

use crate::arrow_utils::record_batch_to_value;
use crate::arrow_utils::{
    ValueBatchReader, value_to_batch_reader_with_fields,
    value_to_batch_reader_with_utc_timestamp_inference,
};
use crate::databases::df_provider::zero_column_safe_writable;

use super::VectorStore;

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema, Clone, Debug)]
pub struct IndexConfigDto {
    pub name: String,
    pub index_type: String, // render enum via Display
    pub columns: Vec<String>,
}

impl From<IndexConfig> for IndexConfigDto {
    fn from(idx: IndexConfig) -> Self {
        Self {
            name: idx.name,
            index_type: idx.index_type.to_string(),
            columns: idx.columns,
        }
    }
}

#[derive(Clone)]
pub struct LanceDBVectorStore {
    connection: Connection,
    table: Option<Table>,
    table_name: String,
    write_options: Option<WriteOptions>,
}

impl Cacheable for LanceDBVectorStore {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
impl LanceDBVectorStore {
    pub fn validate_table_name(table_name: &str) -> Result<()> {
        lancedb::utils::validate_table_name(table_name)?;
        Ok(())
    }

    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub async fn new(path: PathBuf, table_name: String) -> Result<Self> {
        Self::validate_table_name(&table_name)?;
        let connection = connect(path.to_str().unwrap()).execute().await.ok();
        let connection: Connection = connection.ok_or(anyhow!("Error connecting to LanceDB"))?;

        let table = connection.open_table(&table_name).execute().await.ok();

        Ok(LanceDBVectorStore {
            connection,
            table,
            table_name,
            write_options: None,
        })
    }

    pub async fn from_connection(connection: Connection, table_name: String) -> Self {
        // LanceDB 0.27's listing backend unwraps table-name validation while
        // deriving the table URI. Never call it with invalid input; callers
        // that need a table will receive the existing "Table not initialized"
        // error instead of taking down the runtime with a dependency panic.
        let table = if Self::validate_table_name(&table_name).is_ok() {
            connection.open_table(&table_name).execute().await.ok()
        } else {
            None
        };

        LanceDBVectorStore {
            connection,
            table,
            table_name,
            write_options: None,
        }
    }

    pub fn set_write_options(&mut self, options: WriteOptions) {
        self.write_options = Some(options);
    }

    /// Create an empty table from an explicit schema, without inserting a seed row.
    ///
    /// Returns `true` when this call created the table and `false` when the table already
    /// existed and `if_not_exists` was enabled.
    pub async fn create_empty_table(
        &mut self,
        schema: Schema,
        if_not_exists: bool,
    ) -> Result<bool> {
        let existed = self.table.is_some();
        if existed && if_not_exists {
            let existing_schema = self
                .table
                .as_ref()
                .expect("table existence was checked")
                .schema()
                .await?;
            if schemas_compatible_for_creation(existing_schema.as_ref(), &schema) {
                return Ok(false);
            }
            return Err(anyhow!(
                "Table '{}' already exists with a different schema",
                self.table_name
            ));
        }

        let requested_schema = Arc::new(schema);
        let mut builder = self
            .connection
            .create_empty_table(&self.table_name, requested_schema.clone());
        if let Some(opts) = &self.write_options {
            builder = builder.write_options(opts.clone());
        }

        let (table, created) = match builder.execute().await {
            Ok(table) => (table, true),
            Err(lancedb::Error::TableAlreadyExists { .. }) if if_not_exists => {
                let table = self
                    .connection
                    .open_table(&self.table_name)
                    .execute()
                    .await?;
                (table, false)
            }
            Err(error) => return Err(error.into()),
        };
        if !created
            && !schemas_compatible_for_creation(
                table.schema().await?.as_ref(),
                requested_schema.as_ref(),
            )
        {
            return Err(anyhow!(
                "Table '{}' already exists with a different schema",
                self.table_name
            ));
        }
        self.table = Some(table);
        Ok(created)
    }

    /// Drop the whole table (data AND schema). Unlike `purge`, this allows the table to be
    /// recreated with a different schema (e.g. a new embedding vector dimension) on the next insert.
    pub async fn drop_table(&mut self) -> Result<()> {
        let exists = self
            .connection
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|name| name == &self.table_name);
        if exists {
            self.connection.drop_table(&self.table_name, &[]).await?;
        }
        self.table = None;
        Ok(())
    }

    pub async fn list_tables(&self) -> Result<Vec<String>> {
        let tables = self.connection.table_names().execute().await?;
        Ok(tables)
    }

    pub async fn add_columns(
        &self,
        transform: NewColumnTransform,
        read_columns: Option<Vec<String>>,
    ) -> Result<AddColumnsResult> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let result = table.add_columns(transform, read_columns).await?;
        Ok(result)
    }

    pub async fn drop_columns(&self, column_names: &[&str]) -> Result<()> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        table.drop_columns(column_names).await?;
        Ok(())
    }

    pub async fn alter_column(
        &self,
        alteration: &[ColumnAlteration],
    ) -> Result<AlterColumnsResult> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let result = table.alter_columns(alteration).await?;
        Ok(result)
    }

    pub async fn list_indices(&self) -> Result<Vec<IndexConfigDto>> {
        let indices = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        let indices = indices.list_indices().await?;
        Ok(indices.into_iter().map(IndexConfigDto::from).collect())
    }

    pub async fn drop_index(&self, name: &str) -> Result<()> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        table.drop_index(name).await?;
        Ok(())
    }

    pub async fn update(
        &self,
        filter: &str,
        updates: std::collections::HashMap<String, Value>,
    ) -> Result<()> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let mut op = table.update();
        op = op.only_if(filter);

        for (column, value) in updates {
            let value_str = match &value {
                Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => "NULL".to_string(),
                _ => format!("'{}'", value.to_string().replace('\'', "''")),
            };
            op = op.column(&column, &value_str);
        }

        op.execute().await?;
        Ok(())
    }

    pub async fn add_column(&self, name: &str, sql_expression: &str) -> Result<()> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let transform = NewColumnTransform::SqlExpressions(vec![(
            name.to_string(),
            sql_expression.to_string(),
        )]);
        table.add_columns(transform, None).await?;
        Ok(())
    }

    pub async fn make_column_nullable(&self, column: &str, nullable: bool) -> Result<()> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let alteration = ColumnAlteration::new(column.to_string()).set_nullable(nullable);
        table.alter_columns(&[alteration]).await?;
        Ok(())
    }

    /// The returned provider supports SELECT, INSERT INTO and (via
    /// [`crate::databases::lance_dml`]) UPDATE/DELETE with a WHERE clause.
    /// Read-only surfaces registering it must validate their SQL first
    /// ([`crate::databases::sql_guard::validate_readonly_sql`]).
    pub async fn to_datafusion(&self) -> Result<Arc<dyn TableProvider>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        let df_table = table.base_table();
        let adapter =
            lancedb::table::datafusion::BaseTableAdapter::try_new(df_table.clone()).await?;
        Ok(zero_column_safe_writable(Arc::new(adapter), table))
    }

    pub async fn raw(&self) -> Result<Table> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        Ok(table)
    }

    pub async fn sql(
        &self,
        table_name: &str,
        sql: &str,
    ) -> Result<datafusion::dataframe::DataFrame> {
        crate::databases::sql_guard::validate_lance_dml_sql(sql)?;
        let table = self.to_datafusion().await?;
        let ctx = SessionContext::new();
        ctx.register_table(table_name, table)?;
        let results = ctx.sql(sql).await?;

        Ok(results)
    }

    pub async fn insert_record_batch(&mut self, batch: RecordBatch) -> Result<()> {
        let items = vec![batch];

        if self.table.is_none() {
            let mut builder = self.connection.create_table(&self.table_name, items);
            if let Some(opts) = &self.write_options {
                builder = builder.write_options(opts.clone());
            }
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table);
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' from record batch: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let table = self.table.clone().unwrap();
        let mut add = table.add(items);
        if let Some(opts) = &self.write_options {
            add = add.write_options(opts.clone());
        }
        match add.execute().await {
            Ok(_) => Ok(()),
            Err(err) => Err(anyhow!(err.to_string())),
        }
    }

    async fn write_batch_reader(&self, items: Vec<Value>) -> Result<ValueBatchReader> {
        if let Some(table) = &self.table {
            let schema = table.schema().await?;
            let fields = schema.fields().iter().cloned().collect();
            return value_to_batch_reader_with_fields(items, Some(fields));
        }

        value_to_batch_reader_with_utc_timestamp_inference(items)
    }
}

/// Treat the historical timezone-less millisecond timestamp as compatible
/// with the UTC-aware schema now emitted for new timestamp columns. The stored
/// schema remains authoritative; writes to that legacy shape are normalized at
/// the serialization boundary.
fn schemas_compatible_for_creation(existing: &Schema, requested: &Schema) -> bool {
    if existing == requested {
        return true;
    }

    existing.metadata() == requested.metadata()
        && existing.fields().len() == requested.fields().len()
        && existing
            .fields()
            .iter()
            .zip(requested.fields())
            .all(|(existing, requested)| {
                existing.name() == requested.name()
                    && existing.is_nullable() == requested.is_nullable()
                    && existing.metadata() == requested.metadata()
                    && (existing.data_type() == requested.data_type()
                        || matches!(
                            (existing.data_type(), requested.data_type()),
                            (
                                DataType::Timestamp(existing_unit, None),
                                DataType::Timestamp(requested_unit, Some(timezone)),
                            ) if existing_unit == requested_unit && timezone.eq_ignore_ascii_case("UTC")
                        ))
            })
}

pub fn record_batches_to_vec(batches: Option<Vec<RecordBatch>>) -> Result<Vec<Value>> {
    batches
        .as_ref()
        .ok_or(anyhow!("Error converting record batches to vec"))?;

    let batches = batches.unwrap();
    let mut items = vec![];

    for batch in batches {
        let values = record_batch_to_value(&batch);
        match values {
            Ok(mut values) => {
                items.append(&mut values);
            }
            Err(err) => {
                eprintln!("[LanceDB] Error converting batch to value: {err:#}");
            }
        }
    }

    Ok(items)
}

fn is_vector_data_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::FixedSizeList(field, _) | DataType::List(field) | DataType::LargeList(field) => {
            match field.data_type() {
                DataType::Float16 | DataType::Float32 | DataType::Float64 => true,
                nested => is_vector_data_type(nested),
            }
        }
        _ => false,
    }
}

fn split_hybrid_fields(
    schema: &Schema,
    fields: Option<Vec<String>>,
) -> (Option<String>, Option<Vec<String>>) {
    let Some(fields) = fields else {
        return (None, None);
    };

    let mut vector_column = None;
    let mut fts_fields = Vec::new();

    for field in fields {
        let is_vector_field = schema
            .field_with_name(&field)
            .map(|schema_field| is_vector_data_type(schema_field.data_type()))
            .unwrap_or(false);

        if vector_column.is_none() && is_vector_field {
            vector_column = Some(field);
        } else {
            fts_fields.push(field);
        }
    }

    let fts_fields = if fts_fields.is_empty() {
        None
    } else {
        Some(fts_fields)
    };

    (vector_column, fts_fields)
}

#[async_trait]
impl VectorStore for LanceDBVectorStore {
    async fn vector_search(
        &self,
        vector: Vec<f64>,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let mut query = table
            .query()
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .limit(limit)
            .offset(offset);

        if let Some(filter) = filter {
            query = query.only_if(filter);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn fts_search(
        &self,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let mut fts_query = FullTextSearchQuery::new(text.to_string());
        if let Some(fields) = fields {
            match fields.len() {
                1 => fts_query = fts_query.with_column(fields[0].clone())?,
                n if n > 1 => fts_query = fts_query.with_columns(&fields)?,
                _ => {}
            }
        }

        let mut query = table
            .query()
            .full_text_search(fts_query)
            .limit(limit)
            .offset(offset);

        if let Some(filter) = filter {
            query = query.only_if(filter);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn hybrid_search(
        &self,
        vector: Vec<f64>,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        rerank: bool,
    ) -> Result<Vec<Value>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        let schema = table.schema().await?;
        let (vector_column, fields) = split_hybrid_fields(&schema, fields);

        let mut fts_query = FullTextSearchQuery::new(text.to_string());
        if let Some(ref fields) = fields {
            match fields.len() {
                1 => fts_query = fts_query.with_column(fields[0].clone())?,
                n if n > 1 => fts_query = fts_query.with_columns(fields)?,
                _ => {}
            }
        }

        let mut query = table
            .query()
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .full_text_search(fts_query)
            .limit(limit)
            .offset(offset);

        if let Some(vector_column) = vector_column {
            query = query.column(&vector_column);
        }

        if rerank {
            let reranker = Arc::new(lancedb::rerankers::rrf::RRFReranker::new(60.0));
            query = query.rerank(reranker);
        }

        if let Some(filter) = filter {
            query = query.only_if(filter);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query
            .execute_hybrid(QueryExecutionOptions::default())
            .await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn filter(
        &self,
        filter: &str,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let mut query = table.query().limit(limit).only_if(filter).offset(offset);

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn upsert(&mut self, items: Vec<Value>, id_field: String) -> Result<()> {
        let items = self.write_batch_reader(items).await?;

        if self.table.is_none() {
            let mut builder = self.connection.create_table(&self.table_name, items);
            if let Some(opts) = &self.write_options {
                builder = builder.write_options(opts.clone());
            }
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table);
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' for upsert: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let table = self.table.clone().unwrap();
        table
            .merge_insert(&[&id_field])
            .when_matched_update_all(None)
            .when_not_matched_insert_all()
            .to_owned()
            .execute(items)
            .await?;
        Ok(())
    }

    async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
        let items = self.write_batch_reader(items).await?;

        if self.table.is_none() {
            let mut builder = self.connection.create_table(&self.table_name, items);
            if let Some(opts) = &self.write_options {
                builder = builder.write_options(opts.clone());
            }
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table);
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' for insert: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let table = self.table.clone().unwrap();
        let mut add = table.add(items);
        if let Some(opts) = &self.write_options {
            add = add.write_options(opts.clone());
        }
        match add.execute().await {
            Ok(_) => return Ok(()),
            Err(err) => {
                return Err(anyhow!(err.to_string()));
            }
        }
    }

    async fn delete(&self, filter: &str) -> Result<()> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        table.delete(filter).await?;
        return Ok(());
    }

    async fn optimize(&self, keep_versions: bool) -> Result<()> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;

        let older_than = if keep_versions {
            None
        } else {
            Some(Duration::milliseconds(1))
        };

        table
            .optimize(lancedb::table::OptimizeAction::Prune {
                delete_unverified: Some(true),
                error_if_tagged_old_versions: Some(true),
                older_than,
            })
            .await?;

        table
            .optimize(lancedb::table::OptimizeAction::Compact {
                options: CompactionOptions {
                    ..Default::default()
                },
                remap_options: None,
            })
            .await?;

        table
            .optimize(lancedb::table::OptimizeAction::Index(OptimizeOptions::new()))
            .await?;

        return Ok(());
    }

    async fn list(
        &self,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        let mut query = table.query().limit(limit).offset(offset);

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        record_batches_to_vec(Some(result))
    }

    async fn index(&self, column: &str, index_type: Option<&str>) -> Result<()> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        let index_type = index_type.unwrap_or("AUTO");
        let index_type = match index_type {
            "FULL TEXT" => Index::FTS(FtsIndexBuilder::default()),
            "BTREE" => Index::BTree(BTreeIndexBuilder::default()),
            "BITMAP" => Index::Bitmap(BitmapIndexBuilder::default()),
            "LABEL LIST" => Index::LabelList(LabelListIndexBuilder::default()),
            _ => Index::Auto,
        };

        table.create_index(&[column], index_type).execute().await?;
        Ok(())
    }

    async fn purge(&self) -> Result<()> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        table.delete("1=1").await?;
        Ok(())
    }

    async fn count(&self, filter: Option<String>) -> Result<usize> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        Ok(table.count_rows(filter).await?)
    }

    async fn schema(&self) -> Result<arrow_schema::Schema> {
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        let schema = table.schema().await?;
        let schema = schema.as_ref().clone();
        Ok(schema)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::databases::vector::buffered::{
        BufferedVectorStore, BufferedWriteError, BufferedWriteKind, BufferedWriteOrigin,
    };
    use arrow_schema::{Field, TimeUnit};
    use flow_like_types::{
        create_id,
        json::{from_value, json, to_value},
        tokio,
    };
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct TestStruct {
        id: i32,
        name: String,
        vector: Vec<f32>,
    }

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct TestStruct2 {
        id: i32,
        name: String,
    }

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct NullableFieldRow {
        id: i32,
        name: String,
        #[serde(default)]
        tag: Option<String>,
    }

    #[tokio::test]
    async fn metadata_only_connection_lists_empty_database_without_opening_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let connection = connect(&test_path).execute().await?;
        let db = LanceDBVectorStore::from_connection(connection, String::new()).await;

        assert!(db.list_tables().await?.is_empty());

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn create_empty_table_is_strictly_idempotent() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "schema_test".to_string()).await?;
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);

        assert!(db.create_empty_table(schema.clone(), true).await?);
        assert_eq!(db.count(None).await?, 0);
        assert!(!db.create_empty_table(schema, true).await?);

        let mismatch = Schema::new(vec![Field::new("id", DataType::Utf8, false)]);
        let error = db.create_empty_table(mismatch, true).await.unwrap_err();
        assert!(error.to_string().contains("different schema"));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn new_tables_infer_utc_dates_without_changing_legacy_string_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let timestamp = "2026-08-09T12:34:56.789Z";

        let mut inferred =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "inferred_dates".to_string())
                .await?;
        inferred
            .insert(vec![json!({ "created_at": timestamp })])
            .await?;
        assert_eq!(
            inferred
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        let mut legacy =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_strings".to_string())
                .await?;
        legacy
            .create_empty_table(
                Schema::new(vec![Field::new("created_at", DataType::LargeUtf8, false)]),
                false,
            )
            .await?;
        let mut legacy =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_strings".to_string())
                .await?;
        legacy
            .insert(vec![json!({ "created_at": timestamp })])
            .await?;
        assert_eq!(
            legacy
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::LargeUtf8
        );
        assert_eq!(legacy.list(None, 1, 0).await?[0]["created_at"], timestamp);

        let mut legacy_timestamp =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_timestamp".to_string())
                .await?;
        legacy_timestamp
            .create_empty_table(
                Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new(
                        "created_at",
                        DataType::Timestamp(TimeUnit::Millisecond, None),
                        false,
                    ),
                ]),
                false,
            )
            .await?;
        assert!(
            !legacy_timestamp
                .create_empty_table(
                    Schema::new(vec![
                        Field::new("id", DataType::Int64, false),
                        Field::new(
                            "created_at",
                            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                            false,
                        ),
                    ]),
                    true,
                )
                .await?
        );
        legacy_timestamp
            .insert(vec![json!({ "id": 1, "created_at": timestamp })])
            .await?;
        assert_eq!(legacy_timestamp.count(None).await?, 1);
        assert_eq!(
            legacy_timestamp
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, None)
        );

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn update_filters_match_serialized_row_values_by_column_type() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "row_identity".to_string()).await?;
        db.insert(vec![
            json!({ "created_at": "2026-08-16T12:00:00.000Z", "label": "it's a", "score": 1.5, "flag": true, "note": null }),
            json!({ "created_at": "2026-08-17T12:00:00.000Z", "label": "b", "score": 2.5, "flag": false, "note": "x" }),
        ])
        .await?;
        assert_eq!(
            db.schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        let rows = db.list(None, 10, 0).await?;
        let first = rows
            .iter()
            .find(|row| row["label"] == "it's a")
            .expect("row a should be listed");
        let created_at = first["created_at"]
            .as_i64()
            .expect("timestamps are serialized as native-unit integers");
        assert_eq!(created_at, 1_786_881_600_000);

        let bare_literal = db
            .update(
                &format!("created_at = {created_at}"),
                HashMap::from([("label".to_string(), json!("bare"))]),
            )
            .await;
        assert!(
            bare_literal.is_err(),
            "bare integer literals do not coerce to timestamps"
        );

        let filter = format!(
            "created_at = CAST({created_at} AS TIMESTAMP(3)) AND label = 'it''s a' AND score = 1.5 AND flag = true AND note IS NULL"
        );
        db.update(
            &filter,
            HashMap::from([("label".to_string(), json!("updated"))]),
        )
        .await?;

        let rows = db.list(None, 10, 0).await?;
        let labels: Vec<&str> = rows
            .iter()
            .filter_map(|row| row["label"].as_str())
            .collect();
        assert!(labels.contains(&"updated"), "labels: {labels:?}");
        assert!(labels.contains(&"b"), "labels: {labels:?}");
        assert!(!labels.contains(&"it's a"), "labels: {labels:?}");

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn upsert_round_trips_rows_read_back_with_integer_timestamps() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "round_trip".to_string()).await?;
        db.insert(vec![json!({
            "id": "a",
            "first_seen_at": "2026-08-16T12:00:00.000Z",
            "hits": 1
        })])
        .await?;

        let mut row = db.list(None, 10, 0).await?.remove(0);
        let first_seen_at = row["first_seen_at"]
            .as_i64()
            .expect("timestamps are read back as native-unit integers");
        row["hits"] = json!(2);

        db.upsert(vec![row], "id".to_string()).await?;

        let rows = db.list(None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["hits"], json!(2));
        assert_eq!(rows[0]["first_seen_at"].as_i64(), Some(first_seen_at));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn buffered_upsert_persists_rows_carrying_integer_timestamps() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "buffered_round_trip".to_string())
                .await?;
        let mut db = BufferedVectorStore::new(inner, 2);
        db.upsert(
            vec![json!({ "id": "a", "first_seen_at": "2026-08-16T12:00:00.000Z", "hits": 1 })],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let mut row = db.list(None, 10, 0).await?.remove(0);
        row["hits"] = json!(2);

        let origin = BufferedWriteOrigin::new(Arc::from("writer"), Some("operation".to_string()));
        db.upsert_with_origin(vec![row], "id".to_string(), origin)
            .await?;
        db.flush().await?;

        assert!(!db.has_write_failures());
        let rows = db.list(None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["hits"], json!(2));
        assert_eq!(rows[0]["first_seen_at"].as_i64(), Some(1_786_881_600_000));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn sql_supports_queries_that_project_no_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "count_star".to_string()).await?;
        db.insert(vec![
            json!({ "id": 1, "name": "a" }),
            json!({ "id": 2, "name": "b" }),
            json!({ "id": 3, "name": "c" }),
        ])
        .await?;

        let batches = db
            .sql("count_star", "SELECT COUNT(*) AS cnt FROM count_star")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["cnt"], json!(3));

        let batches = db
            .sql(
                "count_star",
                "SELECT COUNT(*) AS cnt FROM count_star WHERE id > 1",
            )
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["cnt"], json!(2));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn dml_statements_flow_through_a_registered_datafusion_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "people".to_string()).await?;
        db.insert(vec![
            json!({ "id": 1, "name": "a" }),
            json!({ "id": 2, "name": "b" }),
            json!({ "id": 3, "name": "c" }),
        ])
        .await?;

        let ctx = SessionContext::new();
        ctx.register_table("people", db.to_datafusion().await?)?;

        let count = |ctx: SessionContext| async move {
            let batches = ctx
                .sql("SELECT COUNT(*) AS cnt FROM people")
                .await?
                .collect()
                .await?;
            let rows: Vec<Value> = batches
                .iter()
                .map(record_batch_to_value)
                .collect::<Result<Vec<_>>>()?
                .concat();
            Ok::<Value, flow_like_types::Error>(rows[0]["cnt"].clone())
        };

        // EXPLAIN builds the DML plan without executing the mutation.
        ctx.sql("EXPLAIN DELETE FROM people WHERE id = 1")
            .await?
            .collect()
            .await?;
        assert_eq!(count(ctx.clone()).await?, json!(3));

        let batches = ctx
            .sql("UPDATE people SET name = 'z' WHERE id = 1")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));

        // The mutation is visible through the provider registered before it ran.
        let batches = ctx
            .sql("SELECT name FROM people WHERE id = 1")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["name"], json!("z"));

        let batches = ctx
            .sql("DELETE FROM people WHERE id = 3")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // No effective WHERE clause (missing, constant-true or constant-false —
        // indistinguishable after optimization) must refuse, not write the table.
        assert!(
            ctx.sql("DELETE FROM people")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert!(
            ctx.sql("DELETE FROM people WHERE false")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert!(
            ctx.sql("UPDATE people SET name = 'q'")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // Subquery DML shapes are refused before planning — DataFusion would
        // only forward the subquery's inner filters to the table, silently
        // mutating the wrong rows.
        assert!(
            db.sql(
                "people",
                "DELETE FROM people WHERE id IN (SELECT id FROM people WHERE name = 'z')",
            )
            .await
            .is_err()
        );
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // INSERT keeps working through the same provider.
        ctx.sql("INSERT INTO people (id, name) VALUES (4, 'd')")
            .await?
            .collect()
            .await?;
        assert_eq!(count(ctx.clone()).await?, json!(3));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn dml_translates_temporal_predicates() -> Result<()> {
        use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
        use arrow_schema::{DataType, Schema as ArrowSchema};

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "events".to_string()).await?;

        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Timestamp(TimeUnit::Microsecond, None), true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_609_459_200_000_000, // 2021-01-01
                    1_640_995_200_000_000, // 2022-01-01
                ])),
            ],
        )?;
        db.insert_record_batch(batch).await?;

        let ctx = SessionContext::new();
        ctx.register_table("events", db.to_datafusion().await?)?;

        let batches = ctx
            .sql("DELETE FROM events WHERE ts < '2021-06-01T00:00:00'")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));

        let batches = ctx.sql("SELECT id FROM events").await?.collect().await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!(2));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_ingest() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_first() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![1.0, 2.0, 3.0], None, None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[0]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_fts() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;
        db.index("name", Some("FULL TEXT")).await?;

        let search_results: Vec<Value> = db.fts_search("Alice", None, None, None, 10, 0).await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;
        assert_eq!(first_item, records[0]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_hybrid_search_without_vector_index() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;
        db.index("name", Some("FULL TEXT")).await?;

        let search_results: Vec<Value> = db
            .hybrid_search(
                vec![1.0, 2.0, 3.0],
                "Alice",
                None,
                None,
                Some(vec!["name".to_string()]),
                10,
                0,
                true,
            )
            .await?;

        assert!(!search_results.is_empty());
        let items: Vec<TestStruct> = search_results
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;
        assert!(items.iter().any(|item| item.id == 1));

        let search_results_with_vector_field: Vec<Value> = db
            .hybrid_search(
                vec![1.0, 2.0, 3.0],
                "Alice",
                None,
                None,
                Some(vec!["vector".to_string(), "name".to_string()]),
                10,
                0,
                true,
            )
            .await?;

        assert!(!search_results_with_vector_field.is_empty());

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_second() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![2.0, 3.0, 4.0], None, None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[1]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_filter() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![1.0, 2.0, 3.0], Some("id = 2"), None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[1]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_no_vec() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            },
            TestStruct2 {
                id: 2,
                name: "Bob".to_string(),
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let count = db.count(None).await?;

        assert_eq!(count, 2);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_casting() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string())
            .await
            .unwrap();
        let cacheable: Arc<dyn Cacheable> = Arc::new(db.clone());
        let resolved = cacheable
            .as_any()
            .downcast_ref::<LanceDBVectorStore>()
            .unwrap();
        let resolved = resolved.clone();
        assert_eq!(resolved.connection.uri(), db.connection.uri());

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_select() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let select = Some(vec!["id".to_string(), "name".to_string()]);
        let results: Vec<Value> = db.list(select, 10, 0).await?;

        assert!(!results.is_empty());

        let first_item: TestStruct2 = from_value(results[0].clone())?;

        assert_eq!(
            first_item,
            TestStruct2 {
                id: records[0].id,
                name: records[0].name.clone()
            }
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_upsert_rejects_missing_non_nullable_fields() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;

        let result = db
            .upsert(vec![json!({"id": 2, "name": "Bob"})], "id".to_string())
            .await;

        assert!(result.is_err());

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_upsert_nullable_option_field() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;

        let rows_in: Vec<Value> = vec![
            to_value(&NullableFieldRow {
                id: 1,
                name: "Alice".to_string(),
                tag: Some("alpha".to_string()),
            })?,
            to_value(&NullableFieldRow {
                id: 2,
                name: "Bob".to_string(),
                tag: None,
            })?,
        ];

        db.upsert(rows_in, "id".to_string()).await?;

        let rows: Vec<NullableFieldRow> = db
            .list(
                Some(vec![
                    "id".to_string(),
                    "name".to_string(),
                    "tag".to_string(),
                ]),
                10,
                0,
            )
            .await?
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row
            == &NullableFieldRow {
                id: 1,
                name: "Alice".to_string(),
                tag: Some("alpha".to_string()),
            }));
        assert!(rows.iter().any(|row| row
            == &NullableFieldRow {
                id: 2,
                name: "Bob".to_string(),
                tag: None,
            }));

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_buffered_upsert_deduplicates_same_id_before_flush() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);

        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;
        db.upsert(
            vec![json!({"id": 1, "name": "Alice Updated", "tag": "beta"})],
            "id".to_string(),
        )
        .await?;
        db.upsert(
            vec![json!({"id": 2, "name": "Bob", "tag": "gamma"})],
            "id".to_string(),
        )
        .await?;

        db.flush().await?;

        let mut rows: Vec<NullableFieldRow> = db
            .list(
                Some(vec![
                    "id".to_string(),
                    "name".to_string(),
                    "tag".to_string(),
                ]),
                10,
                0,
            )
            .await?
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;

        rows.sort_by_key(|row| row.id);

        assert_eq!(
            rows,
            vec![
                NullableFieldRow {
                    id: 1,
                    name: "Alice Updated".to_string(),
                    tag: Some("beta".to_string()),
                },
                NullableFieldRow {
                    id: 2,
                    name: "Bob".to_string(),
                    tag: Some("gamma".to_string()),
                },
            ]
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_buffered_upsert_rejects_missing_fields_against_existing_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);

        // First: establish the table schema by writing a record with "tag"
        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        // Now upsert a record that is MISSING the "tag" field
        db.upsert(vec![json!({"id": 2, "name": "Bob"})], "id".to_string())
            .await?;

        let result = db.flush().await;
        assert!(
            result.is_err(),
            "flush should fail when records are missing fields from the established schema"
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn buffered_write_failures_keep_the_exact_writer_origin() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 2);

        // Establish a non-nullable three-column schema first.
        db.upsert(
            vec![json!({"id": 1, "name": "seed", "tag": "seed"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let good_origin =
            BufferedWriteOrigin::new(Arc::from("writer-good"), Some("operation-good".to_string()));
        let bad_origin =
            BufferedWriteOrigin::new(Arc::from("writer-bad"), Some("operation-bad".to_string()));

        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "persisted", "tag": "valid"})],
            "id".to_string(),
            good_origin,
        )
        .await?;
        let error = db
            .upsert_with_origin(
                vec![json!({"id": 3, "name": "rejected"})],
                "id".to_string(),
                bad_origin.clone(),
            )
            .await
            .expect_err("the second row should trigger a threshold flush failure");

        let report = error
            .downcast_ref::<BufferedWriteError>()
            .expect("flush error should retain structured failures");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].origin.as_ref(), Some(&bad_origin));
        assert_eq!(report.failures[0].operation, BufferedWriteKind::Upsert);
        assert!(!report.failures[0].error.is_empty());
        assert!(!error.to_string().contains("rejected"));

        // The report survives the failed flush so a completion callback can
        // still create a node-attributed error after the buffer was drained.
        assert!(!db.is_dirty());
        assert!(db.has_write_failures());
        assert_eq!(
            db.write_failure_report()
                .expect("ensure-flush callers should still see the failure")
                .failures,
            report.failures
        );
        let retained = db.take_write_failures();
        assert_eq!(retained, report.failures);
        assert!(!db.has_write_failures());

        let insert_origin = BufferedWriteOrigin::new(
            Arc::from("insert-writer"),
            Some("insert-operation".to_string()),
        );
        db.insert_with_origin(
            vec![json!({"id": 4, "name": "invalid-insert"})],
            insert_origin.clone(),
        )
        .await?;
        let insert_error = db.flush().await.expect_err("insert row should fail");
        let insert_report = insert_error
            .downcast_ref::<BufferedWriteError>()
            .expect("insert flush should retain structured failures");
        assert_eq!(insert_report.failures.len(), 1);
        assert_eq!(
            insert_report.failures[0].origin.as_ref(),
            Some(&insert_origin)
        );
        assert_eq!(
            insert_report.failures[0].operation,
            BufferedWriteKind::Insert
        );
        db.take_write_failures();

        let rows = db.list(None, 10, 0).await?;
        assert!(rows.iter().any(|row| row["id"] == json!(2)));
        assert!(!rows.iter().any(|row| row["id"] == json!(3)));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn buffered_upsert_deduplication_keeps_last_writer_origin() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);
        db.upsert(
            vec![json!({"id": 1, "name": "seed", "tag": "seed"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let first_origin =
            BufferedWriteOrigin::new(Arc::from("writer-first"), Some("first".to_string()));
        let last_origin =
            BufferedWriteOrigin::new(Arc::from("writer-last"), Some("last".to_string()));
        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "first-invalid"})],
            "id".to_string(),
            first_origin,
        )
        .await?;
        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "last-invalid"})],
            "id".to_string(),
            last_origin.clone(),
        )
        .await?;

        let error = db.flush().await.expect_err("deduplicated row should fail");
        let report = error
            .downcast_ref::<BufferedWriteError>()
            .expect("flush error should retain structured failures");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].origin.as_ref(), Some(&last_origin));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_add_and_drop_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![to_value(&TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            })?],
            "id".to_string(),
        )
        .await?;

        db.add_column("counter", "CAST(0 AS INT)").await?;
        db.add_column("note", "CAST('' AS STRING)").await?;
        db.add_column("flag", "CAST(NULL AS STRING)").await?;

        let names: Vec<String> = db
            .schema()
            .await?
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert!(names.contains(&"counter".to_string()));
        assert!(names.contains(&"note".to_string()));
        assert!(names.contains(&"flag".to_string()));

        db.drop_columns(&["counter", "note"]).await?;

        let names: Vec<String> = db
            .schema()
            .await?
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert!(!names.contains(&"counter".to_string()));
        assert!(!names.contains(&"note".to_string()));
        assert!(names.contains(&"flag".to_string()));

        std::fs::remove_dir_all(&test_path).unwrap();
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_add_column_bare_null_fails() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![to_value(&TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            })?],
            "id".to_string(),
        )
        .await?;

        // LanceDB requires `CAST(NULL AS <type>)`; a bare `NULL` cannot be inferred.
        let bare_null = db.add_column("flag", "NULL").await;
        assert!(
            bare_null.is_err(),
            "bare NULL should fail; LanceDB requires CAST(NULL AS <type>)"
        );

        std::fs::remove_dir_all(&test_path).unwrap();
        Ok(())
    }

    /// The end of the parameter path: a value bound into an `only_if` predicate reaches the
    /// row it names, and a value that tries to close its own literal reaches none — proved
    /// against a real table rather than against the string this module produces.
    #[tokio::test]
    async fn bound_filter_values_stay_inside_their_literal() -> Result<()> {
        use crate::databases::lance_filter_params::{bind_filter_params, resolve_filter_params};

        fn bind(filter: &str, supplied: Value) -> Result<String> {
            bind_filter_params(filter, &resolve_filter_params(filter, &supplied)?)
        }

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "bound_filter".to_string()).await?;
        db.insert(vec![
            json!({ "id": "a", "name": "first" }),
            json!({ "id": "o'brien", "name": "second" }),
            json!({ "id": "c", "name": "third" }),
        ])
        .await?;

        let quoted = bind("id = $id", json!({ "id": "o'brien" }))?;
        let rows = db.filter(&quoted, None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "second");

        for attempt in [
            "' OR id != '",
            "a' OR 'a' = 'a",
            // The backslash case: this dialect does not read `\'` as an escape, so the
            // doubled quote is what keeps the tail inside the literal.
            "x\\' OR true --",
        ] {
            let filter = bind("id = $id", json!({ "id": attempt }))?;
            assert!(
                db.filter(&filter, None, 10, 0).await?.is_empty(),
                "matched rows for {attempt}: {filter}"
            );
        }

        let in_list = bind("id IN ($ids)", json!({ "ids": ["a", "c"] }))?;
        assert_eq!(db.filter(&in_list, None, 10, 0).await?.len(), 2);
        let empty_list = bind("id IN ($ids)", json!({ "ids": [] }))?;
        assert!(db.filter(&empty_list, None, 10, 0).await?.is_empty());

        // The delete predicate goes through the same parser as the query one.
        db.delete(&quoted).await?;
        assert_eq!(db.count(None).await?, 2);

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }
}

// impl VectorStoreIndex for LanceDBVectorStore {
//     fn top_n<T: for<'a> serde::Deserialize<'a> + rig::wasm_compat::WasmCompatSend>(
//             &self,
//             req: rig::vector_store::VectorSearchRequest<Self::Filter>,
//         ) -> impl std::future::Future<Output = std::result::Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError>>
//         + rig::wasm_compat::WasmCompatSend {
//         todo!("Implement top_n_ids")
//     }

//     fn top_n_ids(
//             &self,
//             req: rig::vector_store::VectorSearchRequest<Self::Filter>,
//         ) -> impl std::future::Future<Output = std::result::Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError>> + rig::wasm_compat::WasmCompatSend {
//         todo!("Implement top_n_ids")
//     }

//     type Filter;
// }
