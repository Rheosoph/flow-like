use super::VectorStore;
use super::lancedb::is_batch_wide_failure;
use arrow_schema::Schema;
use flow_like_types::{Cacheable, Result, Value, anyhow, async_trait};
use std::any::Any;
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

const DEFAULT_BATCH_SIZE: usize = 1000;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BufferedWriteOrigin {
    pub node_id: Arc<str>,
    pub operation_id: Option<String>,
}

impl BufferedWriteOrigin {
    pub fn new(node_id: Arc<str>, operation_id: Option<String>) -> Self {
        Self {
            node_id,
            operation_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BufferedWriteKind {
    Insert,
    Upsert,
}

impl fmt::Display for BufferedWriteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Insert => f.write_str("insert"),
            Self::Upsert => f.write_str("upsert"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferedWriteFailure {
    pub origin: Option<BufferedWriteOrigin>,
    pub operation: BufferedWriteKind,
    pub error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferedWriteError {
    pub failures: Vec<BufferedWriteFailure>,
}

impl BufferedWriteError {
    pub fn new(failures: Vec<BufferedWriteFailure>) -> Self {
        Self { failures }
    }

    pub fn skipped_records(&self) -> usize {
        self.failures.len()
    }
}

impl fmt::Display for BufferedWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Database flush completed with {} row write failure(s)",
            self.skipped_records()
        )
    }
}

impl std::error::Error for BufferedWriteError {}

#[derive(Clone)]
struct BufferedItem {
    value: Value,
    origin: Option<BufferedWriteOrigin>,
}

enum BufferedOp {
    Insert(Vec<BufferedItem>),
    Upsert(Vec<BufferedItem>, String),
}

pub struct BufferedVectorStore<T: VectorStore> {
    inner: T,
    buffer: Vec<BufferedOp>,
    buffered_count: usize,
    batch_size: usize,
    pending_failures: Vec<BufferedWriteFailure>,
}

impl<T: VectorStore + 'static> Cacheable for BufferedVectorStore<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl<T: VectorStore> BufferedVectorStore<T> {
    pub fn new(inner: T, batch_size: usize) -> Self {
        let batch_size = if batch_size == 0 {
            DEFAULT_BATCH_SIZE
        } else {
            batch_size
        };
        Self {
            inner,
            buffer: Vec::new(),
            buffered_count: 0,
            batch_size,
            pending_failures: Vec::new(),
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Drops every queued write without persisting it. Used when the target
    /// table is about to be deleted: flushing would recreate the table the
    /// caller is dropping, because `insert`/`upsert` bootstrap a missing table.
    pub fn discard_buffer(&mut self) {
        self.buffer.clear();
        self.buffered_count = 0;
    }

    pub fn take_write_failures(&mut self) -> Vec<BufferedWriteFailure> {
        std::mem::take(&mut self.pending_failures)
    }

    pub fn has_write_failures(&self) -> bool {
        !self.pending_failures.is_empty()
    }

    pub fn write_failure_report(&self) -> Option<BufferedWriteError> {
        (!self.pending_failures.is_empty())
            .then(|| BufferedWriteError::new(self.pending_failures.clone()))
    }

    pub fn pending_write_origins(&self) -> Vec<BufferedWriteOrigin> {
        self.buffer
            .iter()
            .flat_map(|operation| match operation {
                BufferedOp::Insert(items) | BufferedOp::Upsert(items, _) => items.iter(),
            })
            .filter_map(|item| item.origin.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn has_unattributed_pending_writes(&self) -> bool {
        self.buffer.iter().any(|operation| match operation {
            BufferedOp::Insert(items) | BufferedOp::Upsert(items, _) => {
                items.iter().any(|item| item.origin.is_none())
            }
        })
    }

    async fn flush_buffer(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let mut ops = std::mem::take(&mut self.buffer).into_iter().peekable();
        self.buffered_count = 0;

        // Coalesce consecutive ops of the same kind
        let mut failures = Vec::new();
        while let Some(op) = ops.next() {
            match op {
                BufferedOp::Insert(mut merged) => {
                    while let Some(BufferedOp::Insert(items)) =
                        ops.next_if(|next| matches!(next, BufferedOp::Insert(_)))
                    {
                        merged.extend(items);
                    }
                    failures.extend(Self::write_smart(&mut self.inner, merged, None).await);
                }
                BufferedOp::Upsert(mut merged, id_field) => {
                    while let Some(BufferedOp::Upsert(items, _)) = ops.next_if(
                        |next| matches!(next, BufferedOp::Upsert(_, next_id) if *next_id == id_field),
                    ) {
                        merged.extend(items);
                    }
                    failures
                        .extend(Self::write_smart(&mut self.inner, merged, Some(&id_field)).await);
                }
            }
        }

        if !failures.is_empty() {
            self.pending_failures.extend(failures.iter().cloned());
            return Err(anyhow!(BufferedWriteError::new(failures)));
        }

        Ok(())
    }

    /// Rows without a key never match an existing row, so each one is kept.
    /// Numbers compare by value, except for integer key columns: those reject
    /// `1.0`, and folding it into `1` would discard the row that can be written.
    fn upsert_key(value: &Value, numeric: bool) -> Option<String> {
        match value {
            Value::Null => None,
            Value::Number(number) if numeric => Some(match number.as_f64() {
                Some(float)
                    if number.is_f64() && float.fract() == 0.0 && float.abs() < i64::MAX as f64 =>
                {
                    (float as i64).to_string()
                }
                _ => number.to_string(),
            }),
            value => flow_like_types::json::to_string(value).ok(),
        }
    }

    fn dedupe_upsert_items(
        items: Vec<BufferedItem>,
        id_field: &str,
        numeric: bool,
    ) -> Vec<BufferedItem> {
        if items.len() <= 1 {
            return items;
        }

        let mut seen_ids = HashSet::new();
        let mut deduped = Vec::with_capacity(items.len());

        for item in items.into_iter().rev() {
            let key = item
                .value
                .get(id_field)
                .and_then(|id| Self::upsert_key(id, numeric));
            if key.is_none_or(|key| seen_ids.insert(key)) {
                deduped.push(item);
            }
        }

        deduped.reverse();
        deduped
    }

    /// Separates rows whose keys match the table's columns exactly from the rest.
    fn partition_by_schema(
        schema: &Schema,
        items: Vec<BufferedItem>,
    ) -> (Vec<BufferedItem>, Vec<BufferedItem>) {
        let columns: BTreeSet<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        items.into_iter().partition(|item| {
            item.value.as_object().is_some_and(|row| {
                row.len() == columns.len() && row.keys().all(|key| columns.contains(key.as_str()))
            })
        })
    }

    fn values(items: &[BufferedItem]) -> Vec<Value> {
        items.iter().map(|item| item.value.clone()).collect()
    }

    /// Writes schema-compatible rows as one batch and the rest separately.
    /// Returns every terminal record failure with its originating write.
    async fn write_smart(
        inner: &mut T,
        items: Vec<BufferedItem>,
        id_field: Option<&str>,
    ) -> Vec<BufferedWriteFailure> {
        if items.is_empty() {
            return Vec::new();
        }

        let schema = inner.schema().await.ok();
        let items = match id_field {
            Some(id_field) => {
                let integer_key = schema
                    .as_ref()
                    .and_then(|schema| schema.field_with_name(id_field).ok())
                    .is_some_and(|field| field.data_type().is_integer());
                Self::dedupe_upsert_items(items, id_field, !integer_key)
            }
            None => items,
        };

        // Without a table the first write creates it, so there is no schema to partition by.
        let Some(schema) = schema else {
            return Self::divide_and_conquer(inner, items, id_field).await;
        };

        let (compatible, outliers) = Self::partition_by_schema(&schema, items);
        let mut failures = Self::divide_and_conquer(inner, compatible, id_field).await;
        if !outliers.is_empty() {
            eprintln!(
                "[BufferedVectorStore] Schema mismatch: {} outlier records, writing them separately",
                outliers.len()
            );
            failures.extend(Self::divide_and_conquer(inner, outliers, id_field).await);
        }
        failures
    }

    /// Splits a rejected batch until the rejected rows are isolated. A failure
    /// no subset can avoid (storage, credentials, a missing table, losing every
    /// commit race) fails the batch once instead: each split would repeat it,
    /// and for a keyed upsert each attempt is a full-table merge.
    fn divide_and_conquer<'a>(
        inner: &'a mut T,
        mut items: Vec<BufferedItem>,
        id_field: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Vec<BufferedWriteFailure>> + Send + 'a>> {
        Box::pin(async move {
            if items.is_empty() {
                return Vec::new();
            }
            let (operation, result) = match id_field {
                Some(id_field) => (
                    BufferedWriteKind::Upsert,
                    inner
                        .upsert(Self::values(&items), id_field.to_string())
                        .await,
                ),
                None => (
                    BufferedWriteKind::Insert,
                    inner.insert(Self::values(&items)).await,
                ),
            };
            let Err(error) = result else {
                return Vec::new();
            };
            if items.len() == 1 || is_batch_wide_failure(&error) {
                eprintln!(
                    "[BufferedVectorStore] Skipping {operation} of {} record(s): {error:#}",
                    items.len()
                );
                return Self::failures(&items, operation, &error);
            }
            eprintln!(
                "[BufferedVectorStore] {operation} of {} records failed, isolating the rejected rows: {error:#}",
                items.len()
            );
            let right = items.split_off(items.len() / 2);
            let mut failures = Self::divide_and_conquer(inner, items, id_field).await;
            failures.extend(Self::divide_and_conquer(inner, right, id_field).await);
            failures
        })
    }

    fn failures(
        items: &[BufferedItem],
        operation: BufferedWriteKind,
        error: &flow_like_types::Error,
    ) -> Vec<BufferedWriteFailure> {
        items
            .iter()
            .map(|item| BufferedWriteFailure {
                origin: item.origin.clone(),
                operation,
                error: format!("{error:#}"),
            })
            .collect()
    }

    /// A threshold flush also writes rows other nodes queued. Their failures stay
    /// recorded for completion and later reads; the enqueuing node only fails
    /// for its own rows.
    async fn enqueue(
        &mut self,
        items: Vec<Value>,
        id_field: Option<String>,
        origin: Option<BufferedWriteOrigin>,
    ) -> Result<()> {
        self.inner.ensure_writable()?;
        if self.inner.is_durably_managed() {
            return match id_field {
                Some(id_field) => self.inner.upsert(items, id_field).await,
                None => self.inner.insert(items).await,
            };
        }
        self.buffered_count += items.len();
        let items = items
            .into_iter()
            .map(|value| BufferedItem {
                value,
                origin: origin.clone(),
            })
            .collect();
        self.buffer.push(match id_field {
            Some(id_field) => BufferedOp::Upsert(items, id_field),
            None => BufferedOp::Insert(items),
        });
        if self.buffered_count < self.batch_size {
            return Ok(());
        }

        let Err(error) = self.flush_buffer().await else {
            return Ok(());
        };
        let own: Vec<_> = match (origin.as_ref(), error.downcast_ref::<BufferedWriteError>()) {
            (Some(origin), Some(report)) => report
                .failures
                .iter()
                .filter(|failure| failure.origin.as_ref() == Some(origin))
                .cloned()
                .collect(),
            _ => return Err(error),
        };
        if own.is_empty() {
            return Ok(());
        }
        Err(anyhow!(BufferedWriteError::new(own)))
    }

    pub async fn upsert_with_origin(
        &mut self,
        items: Vec<Value>,
        id_field: String,
        origin: BufferedWriteOrigin,
    ) -> Result<()> {
        self.enqueue(items, Some(id_field), Some(origin)).await
    }

    pub async fn insert_with_origin(
        &mut self,
        items: Vec<Value>,
        origin: BufferedWriteOrigin,
    ) -> Result<()> {
        self.enqueue(items, None, Some(origin)).await
    }
}

#[async_trait]
impl<T: VectorStore + 'static> VectorStore for BufferedVectorStore<T> {
    fn is_durably_managed(&self) -> bool {
        self.inner.is_durably_managed()
    }

    fn ensure_writable(&self) -> Result<()> {
        self.inner.ensure_writable()
    }

    async fn vector_search(
        &self,
        vector: Vec<f64>,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before reading"
            ));
        }
        self.inner
            .vector_search(vector, filter, select, limit, offset)
            .await
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
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before reading"
            ));
        }
        self.inner
            .fts_search(text, filter, select, fields, limit, offset)
            .await
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
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before reading"
            ));
        }
        self.inner
            .hybrid_search(vector, text, filter, select, fields, limit, offset, rerank)
            .await
    }

    async fn filter(
        &self,
        filter: &str,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before reading"
            ));
        }
        self.inner.filter(filter, select, limit, offset).await
    }

    async fn upsert(&mut self, items: Vec<Value>, id_field: String) -> Result<()> {
        self.enqueue(items, Some(id_field), None).await
    }

    async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
        self.enqueue(items, None, None).await
    }

    async fn delete(&self, filter: &str) -> Result<()> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before deleting"
            ));
        }
        self.inner.delete(filter).await
    }

    async fn index(&self, column: &str, index_type: Option<&str>) -> Result<()> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before indexing"
            ));
        }
        self.inner.index(column, index_type).await
    }

    async fn optimize(&self, keep_versions: bool) -> Result<()> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before optimizing"
            ));
        }
        self.inner.optimize(keep_versions).await
    }

    async fn list(
        &self,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before reading"
            ));
        }
        self.inner.list(select, limit, offset).await
    }

    async fn purge(&self) -> Result<()> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before purging"
            ));
        }
        self.inner.purge().await
    }

    async fn count(&self, filter: Option<String>) -> Result<usize> {
        if self.is_dirty() {
            return Err(anyhow!(
                "BufferedVectorStore has unflushed writes — call flush() before counting"
            ));
        }
        self.inner.count(filter).await
    }

    async fn schema(&self) -> Result<arrow_schema::Schema> {
        self.inner.schema().await
    }

    async fn flush(&mut self) -> Result<()> {
        self.flush_buffer().await
    }

    fn is_dirty(&self) -> bool {
        !self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::vector::schema::TableInputRejected;
    use arrow_schema::{DataType, Field};
    use flow_like_types::json::json;

    /// Rejects every batch holding a row flagged `bad`, or every write during an outage.
    #[derive(Default)]
    struct ScriptedStore {
        attempts: usize,
        outage: bool,
        written: Vec<Value>,
    }

    impl ScriptedStore {
        fn write(&mut self, items: Vec<Value>) -> Result<()> {
            self.attempts += 1;
            if self.outage {
                return Err(object_store::Error::Generic {
                    store: "test",
                    source: "credentials expired".into(),
                }
                .into());
            }
            if items.iter().any(|item| item["bad"] == json!(true)) {
                return Err(TableInputRejected("row rejected".to_string()).into());
            }
            self.written.extend(items);
            Ok(())
        }
    }

    #[async_trait]
    impl VectorStore for ScriptedStore {
        async fn vector_search(
            &self,
            _: Vec<f64>,
            _: Option<&str>,
            _: Option<Vec<String>>,
            _: usize,
            _: usize,
        ) -> Result<Vec<Value>> {
            unimplemented!()
        }

        async fn fts_search(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<Vec<String>>,
            _: Option<Vec<String>>,
            _: usize,
            _: usize,
        ) -> Result<Vec<Value>> {
            unimplemented!()
        }

        async fn hybrid_search(
            &self,
            _: Vec<f64>,
            _: &str,
            _: Option<&str>,
            _: Option<Vec<String>>,
            _: Option<Vec<String>>,
            _: usize,
            _: usize,
            _: bool,
        ) -> Result<Vec<Value>> {
            unimplemented!()
        }

        async fn filter(
            &self,
            _: &str,
            _: Option<Vec<String>>,
            _: usize,
            _: usize,
        ) -> Result<Vec<Value>> {
            unimplemented!()
        }

        async fn upsert(&mut self, items: Vec<Value>, _: String) -> Result<()> {
            self.write(items)
        }

        async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
            self.write(items)
        }

        async fn delete(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn index(&self, _: &str, _: Option<&str>) -> Result<()> {
            unimplemented!()
        }

        async fn optimize(&self, _: bool) -> Result<()> {
            unimplemented!()
        }

        async fn list(&self, _: Option<Vec<String>>, _: usize, _: usize) -> Result<Vec<Value>> {
            unimplemented!()
        }

        async fn purge(&self) -> Result<()> {
            unimplemented!()
        }

        async fn count(&self, _: Option<String>) -> Result<usize> {
            unimplemented!()
        }

        async fn schema(&self) -> Result<Schema> {
            Ok(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("bad", DataType::Boolean, false),
            ]))
        }
    }

    fn rows(count: i64, bad: impl Fn(i64) -> bool) -> Vec<Value> {
        (0..count)
            .map(|id| json!({"id": id, "bad": bad(id)}))
            .collect()
    }

    fn origin(node: &str) -> BufferedWriteOrigin {
        BufferedWriteOrigin::new(Arc::from(node), None)
    }

    #[tokio::test]
    async fn rejected_rows_are_isolated_and_the_rest_written() -> Result<()> {
        let mut db = BufferedVectorStore::new(ScriptedStore::default(), 1000);
        db.upsert(rows(64, |id| id == 7 || id == 40), "id".into())
            .await?;

        let error = db.flush().await.expect_err("two rows are rejected");
        let report = error.downcast_ref::<BufferedWriteError>().unwrap();
        assert_eq!(report.failures.len(), 2);
        assert_eq!(db.inner().written.len(), 62);
        Ok(())
    }

    #[tokio::test]
    async fn a_batch_wide_failure_is_attempted_once() -> Result<()> {
        let store = ScriptedStore {
            outage: true,
            ..ScriptedStore::default()
        };
        let mut db = BufferedVectorStore::new(store, 1000);
        db.upsert(rows(64, |_| false), "id".into()).await?;
        db.insert(rows(64, |_| false)).await?;

        let error = db.flush().await.expect_err("storage is down");
        let report = error.downcast_ref::<BufferedWriteError>().unwrap();
        assert_eq!(report.failures.len(), 128);
        assert_eq!(db.inner().attempts, 2);
        Ok(())
    }

    #[tokio::test]
    async fn a_threshold_flush_fails_only_the_node_whose_rows_were_rejected() -> Result<()> {
        let mut db = BufferedVectorStore::new(ScriptedStore::default(), 2);
        db.upsert_with_origin(rows(1, |_| true), "id".into(), origin("rejected"))
            .await?;

        db.insert_with_origin(vec![json!({"id": 5, "bad": false})], origin("trigger"))
            .await
            .expect("the triggering node's own row was written");

        let report = db
            .write_failure_report()
            .expect("the rejection stays recorded");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].origin, Some(origin("rejected")));

        db.take_write_failures();
        let error = db
            .insert_with_origin(
                vec![json!({"id": 6, "bad": true}), json!({"id": 7, "bad": true})],
                origin("trigger"),
            )
            .await
            .expect_err("the triggering node's own rows were rejected");
        let report = error.downcast_ref::<BufferedWriteError>().unwrap();
        assert_eq!(report.failures.len(), 2);
        Ok(())
    }

    #[test]
    fn upserts_without_a_key_are_never_merged() {
        let items = [
            json!({"id": null}),
            json!({"id": null}),
            json!({"name": "x"}),
        ]
        .into_iter()
        .map(|value| BufferedItem {
            value,
            origin: None,
        })
        .collect();

        let kept = BufferedVectorStore::<ScriptedStore>::dedupe_upsert_items(items, "id", true);

        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn numeric_keys_merge_by_value_unless_the_key_column_is_an_integer() {
        let items = || {
            [
                json!({"id": 1, "v": "first"}),
                json!({"id": 1.0, "v": "last"}),
            ]
            .into_iter()
            .map(|value| BufferedItem {
                value,
                origin: None,
            })
            .collect::<Vec<_>>()
        };

        let by_value =
            BufferedVectorStore::<ScriptedStore>::dedupe_upsert_items(items(), "id", true);
        assert_eq!(by_value.len(), 1);
        assert_eq!(by_value[0].value["v"], json!("last"));

        let integer_column =
            BufferedVectorStore::<ScriptedStore>::dedupe_upsert_items(items(), "id", false);
        assert_eq!(integer_column.len(), 2);
    }
}
