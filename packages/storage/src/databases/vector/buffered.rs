use super::VectorStore;
use flow_like_types::{Cacheable, Result, Value, anyhow, async_trait};
use std::any::Any;
use std::collections::BTreeSet;

const DEFAULT_BATCH_SIZE: usize = 1000;

enum BufferedOp {
    Insert(Vec<Value>),
    Upsert(Vec<Value>, String),
}

pub struct BufferedVectorStore<T: VectorStore> {
    inner: T,
    buffer: Vec<BufferedOp>,
    buffered_count: usize,
    batch_size: usize,
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
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    async fn flush_buffer(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let ops = std::mem::take(&mut self.buffer);
        self.buffered_count = 0;

        // Coalesce consecutive ops of the same kind
        let mut i = 0;
        let mut total_skipped: usize = 0;
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
                    total_skipped += Self::insert_smart(&mut self.inner, merged).await;
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
                    total_skipped +=
                        Self::upsert_smart(&mut self.inner, merged, current_id).await;
                }
            }
        }

        if total_skipped > 0 {
            return Err(anyhow!(
                "Flush completed with {} skipped record(s) due to incompatible schemas",
                total_skipped
            ));
        }

        Ok(())
    }

    /// Collect the set of JSON keys from a Value (empty set if not an object).
    fn value_keys(v: &Value) -> BTreeSet<String> {
        match v.as_object() {
            Some(map) => map.keys().cloned().collect(),
            None => BTreeSet::new(),
        }
    }

    /// Partition items by schema compatibility.
    /// Uses the existing table schema if available, otherwise derives the
    /// reference key-set from the first element. Items whose JSON keys match
    /// the reference go into `compatible`; the rest into `outliers`.
    async fn partition_by_schema(
        inner: &T,
        items: Vec<Value>,
    ) -> (Vec<Value>, Vec<Value>) {
        if items.is_empty() {
            return (items, Vec::new());
        }

        let reference_keys: BTreeSet<String> = if let Ok(schema) = inner.schema().await {
            schema.fields().iter().map(|f| f.name().clone()).collect()
        } else {
            Self::value_keys(&items[0])
        };

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

    /// 3-tier insert: schema filter → divide & conquer → single record.
    /// Returns the number of skipped records.
    async fn insert_smart(inner: &mut T, items: Vec<Value>) -> usize {
        if items.is_empty() {
            return 0;
        }

        let (compatible, outliers) = Self::partition_by_schema(inner, items).await;
        let mut skipped = 0;

        // Tier 1: batch-insert schema-compatible records
        if !compatible.is_empty() {
            if let Err(_) = inner.insert(compatible.clone()).await {
                // Tier 2: divide & conquer on the "compatible" batch
                skipped += Self::insert_divide_and_conquer(inner, compatible).await;
            }
        }

        // Outliers go straight to divide & conquer (may form sub-groups)
        if !outliers.is_empty() {
            println!(
                "Schema mismatch: {} outlier record(s) detected, using divide & conquer",
                outliers.len()
            );
            skipped += Self::insert_divide_and_conquer(inner, outliers).await;
        }

        skipped
    }

    /// 3-tier upsert: schema filter → divide & conquer → single record.
    /// Returns the number of skipped records.
    async fn upsert_smart(inner: &mut T, items: Vec<Value>, id_field: String) -> usize {
        if items.is_empty() {
            return 0;
        }

        let (compatible, outliers) = Self::partition_by_schema(inner, items).await;
        let mut skipped = 0;

        if !compatible.is_empty() {
            if let Err(_) = inner.upsert(compatible.clone(), id_field.clone()).await {
                skipped += Self::upsert_divide_and_conquer(inner, compatible, id_field.clone()).await;
            }
        }

        if !outliers.is_empty() {
            println!(
                "Schema mismatch: {} outlier record(s) detected, using divide & conquer",
                outliers.len()
            );
            skipped += Self::upsert_divide_and_conquer(inner, outliers, id_field).await;
        }

        skipped
    }

    /// Tier 2: binary-split recursive fallback.
    /// Returns the number of skipped records.
    fn insert_divide_and_conquer<'a>(
        inner: &'a mut T,
        items: Vec<Value>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>> {
        Box::pin(async move {
            if items.is_empty() {
                return 0;
            }
            if inner.insert(items.clone()).await.is_ok() {
                return 0;
            }
            // Tier 3: single record — skip on failure
            if items.len() == 1 {
                println!("Skipping insert for incompatible record");
                return 1;
            }
            let mid = items.len() / 2;
            let (left, right) = items.split_at(mid);
            let skipped = Self::insert_divide_and_conquer(inner, left.to_vec()).await;
            skipped + Self::insert_divide_and_conquer(inner, right.to_vec()).await
        })
    }

    /// Tier 2: binary-split recursive fallback.
    /// Returns the number of skipped records.
    fn upsert_divide_and_conquer<'a>(
        inner: &'a mut T,
        items: Vec<Value>,
        id_field: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>> {
        Box::pin(async move {
            if items.is_empty() {
                return 0;
            }
            if inner.upsert(items.clone(), id_field.clone()).await.is_ok() {
                return 0;
            }
            if items.len() == 1 {
                println!("Skipping upsert for incompatible record");
                return 1;
            }
            let mid = items.len() / 2;
            let (left, right) = items.split_at(mid);
            let skipped = Self::upsert_divide_and_conquer(inner, left.to_vec(), id_field.clone()).await;
            skipped + Self::upsert_divide_and_conquer(inner, right.to_vec(), id_field).await
        })
    }

    async fn maybe_flush(&mut self) -> Result<()> {
        if self.buffered_count >= self.batch_size {
            self.flush_buffer().await?;
        }
        Ok(())
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
        let count = items.len();
        self.buffer.push(BufferedOp::Upsert(items, id_field));
        self.buffered_count += count;
        self.maybe_flush().await
    }

    async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
        let count = items.len();
        self.buffer.push(BufferedOp::Insert(items));
        self.buffered_count += count;
        self.maybe_flush().await
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
