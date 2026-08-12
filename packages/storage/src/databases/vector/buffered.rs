use super::VectorStore;
use flow_like_types::{Cacheable, Result, Value, anyhow, async_trait};
use std::any::Any;
use std::collections::{BTreeSet, HashSet};
use std::fmt;
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

        let ops = std::mem::take(&mut self.buffer);
        self.buffered_count = 0;

        // Coalesce consecutive ops of the same kind
        let mut i = 0;
        let mut failures = Vec::new();
        while i < ops.len() {
            match &ops[i] {
                BufferedOp::Insert(_) => {
                    let mut merged = Vec::new();
                    while i < ops.len() {
                        if let BufferedOp::Insert(items) = &ops[i] {
                            merged.extend(items.iter().cloned());
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    failures.extend(Self::insert_smart(&mut self.inner, merged).await);
                }
                BufferedOp::Upsert(_, id_field) => {
                    let current_id = id_field.clone();
                    let mut merged = Vec::new();
                    while i < ops.len() {
                        if let BufferedOp::Upsert(items, id) = &ops[i] {
                            if *id == current_id {
                                merged.extend(items.iter().cloned());
                                i += 1;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    failures.extend(Self::upsert_smart(&mut self.inner, merged, current_id).await);
                }
            }
        }

        if !failures.is_empty() {
            self.pending_failures.extend(failures.iter().cloned());
            return Err(anyhow!(BufferedWriteError::new(failures)));
        }

        Ok(())
    }

    /// Collect the set of JSON keys from a Value (empty set if not an object).
    fn value_keys(item: &BufferedItem) -> BTreeSet<String> {
        match item.value.as_object() {
            Some(map) => map.keys().cloned().collect(),
            None => BTreeSet::new(),
        }
    }

    fn dedupe_upsert_items(items: Vec<BufferedItem>, id_field: &str) -> Vec<BufferedItem> {
        if items.len() <= 1 {
            return items;
        }

        let mut seen_ids = HashSet::new();
        let mut deduped = Vec::with_capacity(items.len());

        for item in items.into_iter().rev() {
            let Some(id_value) = item.value.get(id_field) else {
                deduped.push(item);
                continue;
            };

            let Ok(id_key) = flow_like_types::json::to_string(id_value) else {
                deduped.push(item);
                continue;
            };

            if seen_ids.insert(id_key) {
                deduped.push(item);
            }
        }

        deduped.reverse();
        deduped
    }

    /// Partition items by schema compatibility.
    /// Uses the existing table schema (if any) to separate records whose
    /// key set matches the schema from those that don't.
    async fn partition_by_schema(
        inner: &T,
        items: Vec<BufferedItem>,
    ) -> (Vec<BufferedItem>, Vec<BufferedItem>) {
        if items.is_empty() {
            return (items, Vec::new());
        }

        let Ok(schema) = inner.schema().await else {
            return (items, Vec::new());
        };

        let reference_keys: BTreeSet<String> = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();

        let mut compatible = Vec::new();
        let mut outliers = Vec::new();
        for item in items {
            if Self::value_keys(&item) == reference_keys {
                compatible.push(item);
            } else {
                outliers.push(item);
            }
        }
        (compatible, outliers)
    }

    fn values(items: &[BufferedItem]) -> Vec<Value> {
        items.iter().map(|item| item.value.clone()).collect()
    }

    /// 3-tier insert: schema filter → divide & conquer → single record.
    /// Returns every terminal record failure with its originating write.
    async fn insert_smart(inner: &mut T, items: Vec<BufferedItem>) -> Vec<BufferedWriteFailure> {
        if items.is_empty() {
            return Vec::new();
        }

        // If the table doesn't exist yet, try inserting everything directly
        // to bootstrap the table — schema partitioning is meaningless without a table.
        let has_table = inner.schema().await.is_ok();

        if !has_table {
            match inner.insert(Self::values(&items)).await {
                Ok(()) => return Vec::new(),
                Err(err) => {
                    eprintln!(
                        "[BufferedVectorStore] Batch insert failed (no existing table, {} records): {err:#}",
                        items.len()
                    );
                    return Self::insert_divide_and_conquer(inner, items).await;
                }
            }
        }

        let (compatible, outliers) = Self::partition_by_schema(inner, items).await;
        let mut failures = Vec::new();

        // Tier 1: batch-insert schema-compatible records
        if !compatible.is_empty()
            && let Err(err) = inner.insert(Self::values(&compatible)).await
        {
            eprintln!(
                "[BufferedVectorStore] Batch insert of {} compatible records failed: {err:#}",
                compatible.len()
            );
            failures.extend(Self::insert_divide_and_conquer(inner, compatible).await);
        }

        // Outliers go straight to divide & conquer (may form sub-groups)
        if !outliers.is_empty() {
            eprintln!(
                "[BufferedVectorStore] Schema mismatch: {} outlier records, using divide & conquer",
                outliers.len()
            );
            failures.extend(Self::insert_divide_and_conquer(inner, outliers).await);
        }

        failures
    }

    /// 3-tier upsert: schema filter → divide & conquer → single record.
    /// Returns every terminal record failure with its originating write.
    async fn upsert_smart(
        inner: &mut T,
        items: Vec<BufferedItem>,
        id_field: String,
    ) -> Vec<BufferedWriteFailure> {
        let items = Self::dedupe_upsert_items(items, &id_field);

        if items.is_empty() {
            return Vec::new();
        }

        // If the table doesn't exist yet, try upserting everything directly
        // to bootstrap the table — schema partitioning is meaningless without a table.
        let has_table = inner.schema().await.is_ok();

        if !has_table {
            match inner.upsert(Self::values(&items), id_field.clone()).await {
                Ok(()) => return Vec::new(),
                Err(err) => {
                    eprintln!(
                        "[BufferedVectorStore] Batch upsert failed (no existing table, {} records): {err:#}",
                        items.len()
                    );
                    return Self::upsert_divide_and_conquer(inner, items, id_field).await;
                }
            }
        }

        let (compatible, outliers) = Self::partition_by_schema(inner, items).await;
        let mut failures = Vec::new();

        if !compatible.is_empty()
            && let Err(err) = inner
                .upsert(Self::values(&compatible), id_field.clone())
                .await
        {
            eprintln!(
                "[BufferedVectorStore] Batch upsert of {} compatible records failed: {err:#}",
                compatible.len()
            );
            failures
                .extend(Self::upsert_divide_and_conquer(inner, compatible, id_field.clone()).await);
        }

        if !outliers.is_empty() {
            eprintln!(
                "[BufferedVectorStore] Schema mismatch: {} outlier records, using divide & conquer",
                outliers.len()
            );
            failures.extend(Self::upsert_divide_and_conquer(inner, outliers, id_field).await);
        }

        failures
    }

    /// Tier 2: binary-split recursive fallback.
    /// Returns every terminal record failure with its originating write.
    fn insert_divide_and_conquer<'a>(
        inner: &'a mut T,
        items: Vec<BufferedItem>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<BufferedWriteFailure>> + Send + 'a>>
    {
        Box::pin(async move {
            if items.is_empty() {
                return Vec::new();
            }
            match inner.insert(Self::values(&items)).await {
                Ok(()) => return Vec::new(),
                Err(err) if items.len() == 1 => {
                    eprintln!("[BufferedVectorStore] Skipping insert for record: {err:#}");
                    return vec![BufferedWriteFailure {
                        origin: items[0].origin.clone(),
                        operation: BufferedWriteKind::Insert,
                        error: format!("{err:#}"),
                    }];
                }
                Err(_) => {}
            }
            let mid = items.len() / 2;
            let (left, right) = items.split_at(mid);
            let mut failures = Self::insert_divide_and_conquer(inner, left.to_vec()).await;
            failures.extend(Self::insert_divide_and_conquer(inner, right.to_vec()).await);
            failures
        })
    }

    /// Tier 2: binary-split recursive fallback.
    /// Returns every terminal record failure with its originating write.
    fn upsert_divide_and_conquer<'a>(
        inner: &'a mut T,
        items: Vec<BufferedItem>,
        id_field: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<BufferedWriteFailure>> + Send + 'a>>
    {
        Box::pin(async move {
            if items.is_empty() {
                return Vec::new();
            }
            match inner.upsert(Self::values(&items), id_field.clone()).await {
                Ok(()) => return Vec::new(),
                Err(err) if items.len() == 1 => {
                    eprintln!("[BufferedVectorStore] Skipping upsert for record: {err:#}");
                    return vec![BufferedWriteFailure {
                        origin: items[0].origin.clone(),
                        operation: BufferedWriteKind::Upsert,
                        error: format!("{err:#}"),
                    }];
                }
                Err(_) => {}
            }
            let mid = items.len() / 2;
            let (left, right) = items.split_at(mid);
            let mut failures =
                Self::upsert_divide_and_conquer(inner, left.to_vec(), id_field.clone()).await;
            failures.extend(Self::upsert_divide_and_conquer(inner, right.to_vec(), id_field).await);
            failures
        })
    }

    async fn maybe_flush(&mut self) -> Result<()> {
        if self.buffered_count >= self.batch_size {
            self.flush_buffer().await?;
        }
        Ok(())
    }

    async fn enqueue_upsert(
        &mut self,
        items: Vec<Value>,
        id_field: String,
        origin: Option<BufferedWriteOrigin>,
    ) -> Result<()> {
        let count = items.len();
        let items = items
            .into_iter()
            .map(|value| BufferedItem {
                value,
                origin: origin.clone(),
            })
            .collect();
        self.buffer.push(BufferedOp::Upsert(items, id_field));
        self.buffered_count += count;
        self.maybe_flush().await
    }

    async fn enqueue_insert(
        &mut self,
        items: Vec<Value>,
        origin: Option<BufferedWriteOrigin>,
    ) -> Result<()> {
        let count = items.len();
        let items = items
            .into_iter()
            .map(|value| BufferedItem {
                value,
                origin: origin.clone(),
            })
            .collect();
        self.buffer.push(BufferedOp::Insert(items));
        self.buffered_count += count;
        self.maybe_flush().await
    }

    pub async fn upsert_with_origin(
        &mut self,
        items: Vec<Value>,
        id_field: String,
        origin: BufferedWriteOrigin,
    ) -> Result<()> {
        self.enqueue_upsert(items, id_field, Some(origin)).await
    }

    pub async fn insert_with_origin(
        &mut self,
        items: Vec<Value>,
        origin: BufferedWriteOrigin,
    ) -> Result<()> {
        self.enqueue_insert(items, Some(origin)).await
    }
}

#[async_trait]
impl<T: VectorStore + 'static> VectorStore for BufferedVectorStore<T> {
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
        self.enqueue_upsert(items, id_field, None).await
    }

    async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
        self.enqueue_insert(items, None).await
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
