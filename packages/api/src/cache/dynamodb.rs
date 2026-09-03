//! DynamoDB cache backend with native TTL.
//!
//! One table, `AppCache`, partitioned by app so an app teardown is a single query. The
//! `expires_at` attribute holds epoch **seconds** — DynamoDB's TTL feature requires that
//! unit and silently ignores milliseconds — and must be configured as the table's TTL
//! attribute for automatic eviction.
//!
//! ## Provisioning
//!
//! The table is **not** created automatically; nothing in this workspace grants the API
//! `dynamodb:CreateTable`, and racing cold-start Lambdas are a poor place to provision
//! infrastructure. Create it once per environment:
//!
//! ```bash
//! aws dynamodb create-table \
//!   --table-name "${DYNAMODB_TABLE_PREFIX}AppCache" \
//!   --attribute-definitions \
//!       AttributeName=app_id,AttributeType=S \
//!       AttributeName=entry_key,AttributeType=S \
//!   --key-schema \
//!       AttributeName=app_id,KeyType=HASH \
//!       AttributeName=entry_key,KeyType=RANGE \
//!   --billing-mode PAY_PER_REQUEST
//!
//! aws dynamodb update-time-to-live \
//!   --table-name "${DYNAMODB_TABLE_PREFIX}AppCache" \
//!   --time-to-live-specification "Enabled=true,AttributeName=expires_at"
//! ```
//!
//! No secondary index is needed: `app_id` is the partition key, so per-app teardown is a
//! plain `Query`.
//!
//! Required IAM actions on the table ARN: `GetItem`, `PutItem`, `DeleteItem`, `Query`,
//! `BatchGetItem`, `BatchWriteItem`, and `DescribeTable` for the admin resource
//! dashboard's statistics probe.
//!
//! Forgetting the TTL specification is the failure mode to watch for — entries still read
//! as expired (the stored expiry is checked on read), but nothing is ever reclaimed, so
//! the table grows without bound.
//!
//! ## Oversized values
//!
//! DynamoDB rejects items above 400 KB, but the cache accepts values well beyond that.
//! A value whose serialized form exceeds [`MAX_INLINE_VALUE_BYTES`] is split into chunk
//! items in the same partition, plus a small **manifest** item stored under the entry's
//! normal sort key. The manifest records how many chunks there are, their total byte
//! length, and a `chunk_write_id` minted fresh for every write.
//!
//! The write id is what makes generations impossible to mix: chunk sort keys embed it
//! (`<entry sort key>#chunk#<write_id>#<index>`), so a manifest can only ever resolve the
//! exact chunk set written together with it. When a 5-chunk value is replaced by a
//! 4-chunk one, the old chunk #5 lives under the *old* write id and is unreachable from
//! the new manifest — it is deleted best-effort after the overwrite and reaped by the
//! table TTL regardless. Chunk sort keys cannot collide with real entries because the
//! entry sort key hashes every user-controlled segment (see [`CacheKey::sort_key`]),
//! and because a chunk key extends its manifest's sort key, a namespace's `begins_with`
//! range covers entries and their chunks alike.
//!
//! Write order is chunks first, manifest last: readers switch generations atomically at
//! the manifest, and a write that dies halfway leaves the previous entry fully intact.
//! Chunks carry the same TTL as their manifest. The one orphan case — a crash after the
//! chunks are written but before the manifest is — leaves unreferenced chunks that expire
//! with the entry's TTL, or persist until `delete_app` for TTL-less entries.

use super::types::*;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_dynamodb::{
    Client,
    primitives::Blob,
    types::{
        AttributeValue, DeleteRequest, KeysAndAttributes, PutRequest, ReturnValue, WriteRequest,
    },
};
use chrono::Utc;
use std::collections::HashMap;

const CACHE_TABLE: &str = "AppCache";
const PARTITION_KEY: &str = "app_id";
const SORT_KEY: &str = "entry_key";
const TTL_ATTRIBUTE: &str = "expires_at";

/// Presence of this attribute is what marks an item as a chunked-entry manifest.
const CHUNK_COUNT_ATTRIBUTE: &str = "chunk_count";
const CHUNK_WRITE_ID_ATTRIBUTE: &str = "chunk_write_id";
const CHUNK_TOTAL_BYTES_ATTRIBUTE: &str = "chunk_total_bytes";
const CHUNK_INDEX_ATTRIBUTE: &str = "chunk_index";
const CHUNK_DATA_ATTRIBUTE: &str = "chunk_data";

/// Serialized values up to this many bytes stay inline in a single item; larger ones are
/// chunked. Kept well under DynamoDB's 400 KB item ceiling to leave room for the sort
/// key (which embeds the user-supplied cache key) and the bookkeeping attributes.
const MAX_INLINE_VALUE_BYTES: usize = 300 * 1024;

/// Payload bytes per chunk item, sized with the same headroom as inline values.
const CHUNK_PAYLOAD_BYTES: usize = 300 * 1024;

/// Upper bound accepted when reading a manifest. A corrupt count must not be able to fan
/// out into an unbounded batch fetch. 4096 chunks × 300 KB ≈ 1.2 GB, far beyond any
/// value the API layer lets through.
const MAX_CHUNK_COUNT: usize = 4096;

/// Keys per `BatchGetItem`. The API caps a response at 16 MB; 40 × 300 KB ≈ 12 MB.
const CHUNK_GET_BATCH: usize = 40;

/// Writes per `BatchWriteItem` — the API maximum is 25.
const CHUNK_WRITE_BATCH: usize = 25;

/// How often a batch call is retried while DynamoDB keeps returning unprocessed
/// keys/items before the operation is reported as failed.
const MAX_BATCH_ATTEMPTS: u32 = 6;

pub struct DynamoDbCacheStore {
    client: Client,
    table: String,
}

impl std::fmt::Debug for DynamoDbCacheStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamoDbCacheStore")
            .field("table", &self.table)
            .finish()
    }
}

/// Chunk bookkeeping read from (or written to) a manifest item.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkMeta {
    chunk_count: usize,
    write_id: String,
    total_bytes: usize,
}

/// Sort key of one chunk item. The write id makes the key unique per generation, so a
/// stale chunk of an older (or larger) previous value can never satisfy a newer
/// manifest's lookup.
fn chunk_sort_key(entry_sort_key: &str, write_id: &str, index: usize) -> String {
    format!("{entry_sort_key}#chunk#{write_id}#{index:05}")
}

/// Split a serialized value into chunk payloads. Splitting is by bytes, not characters —
/// chunks are stored as binary and only reinterpreted as UTF-8 after reassembly.
fn split_chunks(payload: &[u8]) -> Vec<&[u8]> {
    payload.chunks(CHUNK_PAYLOAD_BYTES).collect()
}

/// Reassemble fetched chunk payloads, refusing anything that is not the complete,
/// exact chunk set the manifest describes.
fn assemble_chunks(meta: &ChunkMeta, parts: Vec<Option<Vec<u8>>>) -> Option<Vec<u8>> {
    if parts.len() != meta.chunk_count {
        return None;
    }

    let mut assembled = Vec::with_capacity(meta.total_bytes);
    for part in parts {
        assembled.extend_from_slice(&part?);
    }

    if assembled.len() != meta.total_bytes {
        return None;
    }

    Some(assembled)
}

/// DynamoDB TTL wants epoch seconds; round up so a sub-second TTL does not floor to
/// "already expired". Expiries are epoch milliseconds and therefore positive, so the
/// manual round-up is exact.
fn expiry_seconds(expires_at_ms: i64) -> i64 {
    (expires_at_ms + 999) / 1_000
}

fn database_error<E: std::fmt::Display>(error: E) -> CacheStoreError {
    CacheStoreError::Database(error.to_string())
}

impl DynamoDbCacheStore {
    pub fn new(aws_config: &SdkConfig) -> Self {
        let prefix = std::env::var("DYNAMODB_TABLE_PREFIX").unwrap_or_default();
        Self {
            client: Client::new(aws_config),
            table: format!("{prefix}{CACHE_TABLE}"),
        }
    }

    pub async fn from_env() -> Result<Self, CacheStoreError> {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Ok(Self::new(&config))
    }

    /// Sort key: `e#<scope>#h(user)#h(namespace)#h(key)`. The hashed, fixed-width
    /// segments make one namespace a contiguous sort-key range, which is what lets
    /// `delete_namespace` run as a single `begins_with` query — chunk items extend
    /// their manifest's sort key, so the same range covers them too.
    fn sort_key(key: &CacheKey) -> String {
        key.sort_key()
    }

    fn primary_key(key: &CacheKey) -> HashMap<String, AttributeValue> {
        HashMap::from([
            (
                PARTITION_KEY.to_string(),
                AttributeValue::S(key.app_id.clone()),
            ),
            (SORT_KEY.to_string(), AttributeValue::S(Self::sort_key(key))),
        ])
    }

    /// Shared attributes of inline items and chunk manifests.
    fn base_item(
        key: &CacheKey,
        now_ms: i64,
        expires_at: Option<i64>,
    ) -> HashMap<String, AttributeValue> {
        let mut item = Self::primary_key(key);
        item.insert("key".to_string(), AttributeValue::S(key.key.clone()));
        item.insert(
            "scope".to_string(),
            AttributeValue::S(key.scope.as_str().to_string()),
        );
        item.insert(
            "user_id".to_string(),
            AttributeValue::S(key.user_id.clone()),
        );
        item.insert(
            "updated_at".to_string(),
            AttributeValue::N(now_ms.to_string()),
        );

        if let Some(expires_at) = expires_at {
            item.insert(
                TTL_ATTRIBUTE.to_string(),
                AttributeValue::N(expiry_seconds(expires_at).to_string()),
            );
        }

        item
    }

    fn encode_value(value: &serde_json::Value) -> Result<String, CacheStoreError> {
        serde_json::to_string(value).map_err(|e| CacheStoreError::Serialization(e.to_string()))
    }

    /// The item stored under the entry's sort key: the value itself when it fits, or a
    /// manifest pointing at this write's chunk set.
    fn entry_item(
        entry: &SetCacheEntry,
        encoded: &str,
        chunk_meta: Option<&ChunkMeta>,
        now_ms: i64,
    ) -> HashMap<String, AttributeValue> {
        let mut item = Self::base_item(&entry.key, now_ms, entry.expires_at);

        match chunk_meta {
            None => {
                item.insert("value".to_string(), AttributeValue::S(encoded.to_string()));
            }
            Some(meta) => {
                item.insert(
                    CHUNK_COUNT_ATTRIBUTE.to_string(),
                    AttributeValue::N(meta.chunk_count.to_string()),
                );
                item.insert(
                    CHUNK_WRITE_ID_ATTRIBUTE.to_string(),
                    AttributeValue::S(meta.write_id.clone()),
                );
                item.insert(
                    CHUNK_TOTAL_BYTES_ATTRIBUTE.to_string(),
                    AttributeValue::N(meta.total_bytes.to_string()),
                );
            }
        }

        item
    }

    fn attr_n(item: &HashMap<String, AttributeValue>, name: &str) -> Option<i64> {
        item.get(name)
            .and_then(|v| v.as_n().ok())
            .and_then(|n| n.parse::<i64>().ok())
    }

    fn attr_s<'a>(item: &'a HashMap<String, AttributeValue>, name: &str) -> Option<&'a str> {
        item.get(name)
            .and_then(|v| v.as_s().ok())
            .map(String::as_str)
    }

    fn stored_expiry_ms(item: &HashMap<String, AttributeValue>) -> Option<i64> {
        Self::attr_n(item, TTL_ATTRIBUTE).map(|seconds| seconds * 1_000)
    }

    fn is_chunk_manifest(item: &HashMap<String, AttributeValue>) -> bool {
        item.contains_key(CHUNK_COUNT_ATTRIBUTE)
    }

    /// Parse the chunk bookkeeping out of a manifest item. `None` means the manifest is
    /// malformed — callers treat that as a miss so the key heals on the next write.
    fn chunk_meta(item: &HashMap<String, AttributeValue>) -> Option<ChunkMeta> {
        let chunk_count = Self::attr_n(item, CHUNK_COUNT_ATTRIBUTE)?;
        let total_bytes = Self::attr_n(item, CHUNK_TOTAL_BYTES_ATTRIBUTE)?;
        let write_id = Self::attr_s(item, CHUNK_WRITE_ID_ATTRIBUTE)?;

        if chunk_count <= 0 || chunk_count as usize > MAX_CHUNK_COUNT {
            return None;
        }
        // Bounded by what the chunk set could physically hold, so a corrupted manifest
        // cannot make reassembly attempt a giant allocation.
        if total_bytes < 0 || total_bytes as usize > chunk_count as usize * CHUNK_PAYLOAD_BYTES {
            return None;
        }
        if write_id.is_empty() {
            return None;
        }

        Some(ChunkMeta {
            chunk_count: chunk_count as usize,
            write_id: write_id.to_string(),
            total_bytes: total_bytes as usize,
        })
    }

    fn item_to_inline_entry(
        item: &HashMap<String, AttributeValue>,
    ) -> Result<CacheEntry, CacheStoreError> {
        let raw_value = item
            .get("value")
            .and_then(|v| v.as_s().ok())
            .ok_or_else(|| CacheStoreError::Serialization("Item is missing `value`".to_string()))?;

        let value = serde_json::from_str(raw_value)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;

        Ok(CacheEntry {
            key: Self::attr_s(item, "key").unwrap_or_default().to_string(),
            value,
            expires_at: Self::stored_expiry_ms(item),
            updated_at: Self::attr_n(item, "updated_at").unwrap_or_default(),
        })
    }

    /// Run a set of `BatchWriteItem` requests, retrying unprocessed items until they are
    /// all through or the attempt budget is spent.
    async fn run_batch_writes(&self, requests: Vec<WriteRequest>) -> Result<(), CacheStoreError> {
        for batch in requests.chunks(CHUNK_WRITE_BATCH) {
            let mut pending = batch.to_vec();
            let mut attempts = 0u32;

            while !pending.is_empty() {
                let output = self
                    .client
                    .batch_write_item()
                    .request_items(&self.table, pending)
                    .send()
                    .await
                    .map_err(database_error)?;

                pending = output
                    .unprocessed_items()
                    .and_then(|tables| tables.get(&self.table))
                    .cloned()
                    .unwrap_or_default();

                if pending.is_empty() {
                    break;
                }

                attempts += 1;
                if attempts >= MAX_BATCH_ATTEMPTS {
                    return Err(CacheStoreError::Database(format!(
                        "DynamoDB kept returning unprocessed writes after {MAX_BATCH_ATTEMPTS} attempts"
                    )));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50 * u64::from(attempts)))
                    .await;
            }
        }

        Ok(())
    }

    /// Store this write's chunk set. Chunks are keyed by the fresh write id, so they are
    /// invisible until the manifest referencing them lands.
    async fn write_chunks(
        &self,
        key: &CacheKey,
        meta: &ChunkMeta,
        payload: &[u8],
        expires_at: Option<i64>,
        now_ms: i64,
    ) -> Result<(), CacheStoreError> {
        let entry_sort_key = Self::sort_key(key);

        let mut requests = Vec::with_capacity(meta.chunk_count);
        for (index, part) in split_chunks(payload).into_iter().enumerate() {
            let mut item = HashMap::from([
                (
                    PARTITION_KEY.to_string(),
                    AttributeValue::S(key.app_id.clone()),
                ),
                (
                    SORT_KEY.to_string(),
                    AttributeValue::S(chunk_sort_key(&entry_sort_key, &meta.write_id, index)),
                ),
                (
                    CHUNK_INDEX_ATTRIBUTE.to_string(),
                    AttributeValue::N(index.to_string()),
                ),
                (
                    CHUNK_WRITE_ID_ATTRIBUTE.to_string(),
                    AttributeValue::S(meta.write_id.clone()),
                ),
                (
                    CHUNK_DATA_ATTRIBUTE.to_string(),
                    AttributeValue::B(Blob::new(part)),
                ),
                (
                    "updated_at".to_string(),
                    AttributeValue::N(now_ms.to_string()),
                ),
            ]);
            if let Some(expires_at) = expires_at {
                item.insert(
                    TTL_ATTRIBUTE.to_string(),
                    AttributeValue::N(expiry_seconds(expires_at).to_string()),
                );
            }

            let put = PutRequest::builder()
                .set_item(Some(item))
                .build()
                .map_err(database_error)?;
            requests.push(WriteRequest::builder().put_request(put).build());
        }

        self.run_batch_writes(requests).await
    }

    async fn delete_chunk_set(
        &self,
        app_id: &str,
        entry_sort_key: &str,
        meta: &ChunkMeta,
    ) -> Result<(), CacheStoreError> {
        let mut requests = Vec::with_capacity(meta.chunk_count);
        for index in 0..meta.chunk_count {
            let delete = DeleteRequest::builder()
                .key(PARTITION_KEY, AttributeValue::S(app_id.to_string()))
                .key(
                    SORT_KEY,
                    AttributeValue::S(chunk_sort_key(entry_sort_key, &meta.write_id, index)),
                )
                .build()
                .map_err(database_error)?;
            requests.push(WriteRequest::builder().delete_request(delete).build());
        }

        self.run_batch_writes(requests).await
    }

    /// Best-effort removal of the chunk set an overwritten or deleted manifest pointed
    /// at. Failures are logged, not propagated: the chunks are already unreachable (no
    /// live manifest references their write id) and the table TTL reaps them anyway.
    ///
    /// `own_write_id` is the id of the chunk set the caller just wrote, if any. The
    /// SDK retries writes whose response was lost, and such a retry replaces the item
    /// the first attempt committed — making ALL_OLD *this write's own manifest*. Its
    /// chunks are live and must not be deleted.
    async fn cleanup_replaced_item(
        &self,
        key: &CacheKey,
        old_item: Option<&HashMap<String, AttributeValue>>,
        own_write_id: Option<&str>,
    ) {
        let Some(item) = old_item else {
            return;
        };
        if !Self::is_chunk_manifest(item) {
            return;
        }
        let Some(meta) = Self::chunk_meta(item) else {
            tracing::warn!(
                key = %key.key,
                "Replaced cache manifest was malformed; its chunks (if any) are left to the table TTL"
            );
            return;
        };

        if own_write_id == Some(meta.write_id.as_str()) {
            return;
        }

        if let Err(error) = self
            .delete_chunk_set(&key.app_id, &Self::sort_key(key), &meta)
            .await
        {
            tracing::warn!(
                key = %key.key,
                error = %error,
                "Failed to delete replaced cache chunks; they are unreachable and expire with the table TTL"
            );
        }
    }

    /// After a failed or condition-rejected manifest put: did *our* manifest land
    /// anyway? An SDK-internal retry can commit and then observe its own write as a
    /// failure. Strongly consistent read; `None` means it could not be determined.
    async fn own_manifest_landed(&self, key: &CacheKey, write_id: &str) -> Option<bool> {
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .consistent_read(true)
            .expression_attribute_names("#wid", CHUNK_WRITE_ID_ATTRIBUTE)
            .projection_expression("#wid")
            .send()
            .await
            .ok()?;

        match output.item {
            Some(item) => Some(Self::attr_s(&item, CHUNK_WRITE_ID_ATTRIBUTE) == Some(write_id)),
            None => Some(false),
        }
    }

    /// Handle the aftermath of a chunked write whose manifest put errored: keep the
    /// chunks when our manifest actually committed (the write succeeded despite the
    /// error), delete them when another manifest is live, and leave them to the TTL
    /// when the state cannot be determined — an orphaned chunk set is recoverable,
    /// a live manifest without chunks is not.
    ///
    /// Returns `true` when our write turned out to have committed.
    async fn reconcile_failed_chunked_write(&self, key: &CacheKey, meta: &ChunkMeta) -> bool {
        match self.own_manifest_landed(key, &meta.write_id).await {
            Some(true) => true,
            Some(false) => {
                let _ = self
                    .delete_chunk_set(&key.app_id, &Self::sort_key(key), meta)
                    .await;
                false
            }
            None => false,
        }
    }

    /// Fetch and reassemble the chunk set a manifest points at. `Ok(None)` means the set
    /// was incomplete or mismatched — the caller reports a miss rather than serving a
    /// partial or mixed-generation value.
    async fn fetch_chunks(
        &self,
        key: &CacheKey,
        meta: &ChunkMeta,
    ) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let entry_sort_key = Self::sort_key(key);
        let mut parts: Vec<Option<Vec<u8>>> = vec![None; meta.chunk_count];

        let keys: Vec<HashMap<String, AttributeValue>> = (0..meta.chunk_count)
            .map(|index| {
                HashMap::from([
                    (
                        PARTITION_KEY.to_string(),
                        AttributeValue::S(key.app_id.clone()),
                    ),
                    (
                        SORT_KEY.to_string(),
                        AttributeValue::S(chunk_sort_key(&entry_sort_key, &meta.write_id, index)),
                    ),
                ])
            })
            .collect();

        for batch in keys.chunks(CHUNK_GET_BATCH) {
            let mut pending = batch.to_vec();
            let mut attempts = 0u32;

            while !pending.is_empty() {
                // Consistent reads: the manifest was written *after* its chunks, so any
                // manifest we can see has a fully persisted chunk set — but only a
                // strongly consistent read is guaranteed to observe it.
                let request_keys = KeysAndAttributes::builder()
                    .set_keys(Some(pending.clone()))
                    .consistent_read(true)
                    .build()
                    .map_err(database_error)?;

                let output = self
                    .client
                    .batch_get_item()
                    .request_items(&self.table, request_keys)
                    .send()
                    .await
                    .map_err(database_error)?;

                if let Some(items) = output
                    .responses()
                    .and_then(|tables| tables.get(&self.table))
                {
                    for item in items {
                        let Some(index) = Self::attr_n(item, CHUNK_INDEX_ATTRIBUTE) else {
                            continue;
                        };
                        let index = index as usize;
                        if index >= meta.chunk_count {
                            continue;
                        }
                        // The write id is part of the sort key, so this can only differ
                        // on a corrupted item — refuse it rather than risk mixing.
                        if Self::attr_s(item, CHUNK_WRITE_ID_ATTRIBUTE) != Some(&meta.write_id) {
                            continue;
                        }
                        let Some(data) = item.get(CHUNK_DATA_ATTRIBUTE).and_then(|v| v.as_b().ok())
                        else {
                            continue;
                        };
                        parts[index] = Some(data.as_ref().to_vec());
                    }
                }

                pending = output
                    .unprocessed_keys()
                    .and_then(|tables| tables.get(&self.table))
                    .map(|keys_and_attrs| keys_and_attrs.keys().to_vec())
                    .unwrap_or_default();

                if pending.is_empty() {
                    break;
                }

                attempts += 1;
                if attempts >= MAX_BATCH_ATTEMPTS {
                    return Err(CacheStoreError::Database(format!(
                        "DynamoDB kept returning unprocessed chunk reads after {MAX_BATCH_ATTEMPTS} attempts"
                    )));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50 * u64::from(attempts)))
                    .await;
            }
        }

        Ok(assemble_chunks(meta, parts))
    }

    /// Resolve a fetched entry item — inline or manifest — into a `CacheEntry`.
    async fn resolve_item(
        &self,
        key: &CacheKey,
        item: &HashMap<String, AttributeValue>,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        if !Self::is_chunk_manifest(item) {
            return Ok(Some(Self::item_to_inline_entry(item)?));
        }

        let Some(meta) = Self::chunk_meta(item) else {
            tracing::warn!(key = %key.key, "Chunked cache manifest is malformed; treating as a miss");
            return Ok(None);
        };

        let Some(bytes) = self.fetch_chunks(key, &meta).await? else {
            // Expected during a concurrent overwrite (the old chunk set is being torn
            // down while we hold the old manifest); anything else is corruption. Either
            // way a miss is the safe answer — the caller recomputes and overwrites.
            tracing::warn!(
                key = %key.key,
                chunk_count = meta.chunk_count,
                "Chunked cache entry was incomplete; treating as a miss"
            );
            return Ok(None);
        };

        let value = serde_json::from_slice(&bytes)
            .map_err(|e| CacheStoreError::Serialization(e.to_string()))?;

        Ok(Some(CacheEntry {
            key: Self::attr_s(item, "key").unwrap_or_default().to_string(),
            value,
            expires_at: Self::stored_expiry_ms(item),
            updated_at: Self::attr_n(item, "updated_at").unwrap_or_default(),
        }))
    }

    /// Fetch and resolve one entry. Reads are eventually consistent by default (cheap,
    /// right for the hot path); `get_or_set` retries use a consistent read so a lagging
    /// replica cannot fake a miss.
    async fn get_internal(
        &self,
        key: &CacheKey,
        consistent: bool,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .consistent_read(consistent)
            .send()
            .await
            .map_err(database_error)?;

        let Some(item) = output.item else {
            return Ok(None);
        };

        // DynamoDB TTL deletion can lag by up to 48 hours, so the stored expiry is the
        // authority for reads.
        let now_ms = Utc::now().timestamp_millis();
        if Self::stored_expiry_ms(&item).is_some_and(|expires| expires <= now_ms) {
            return Ok(None);
        }

        self.resolve_item(key, &item).await
    }

    /// Prepare a write: serialize once and decide inline vs chunked.
    fn plan_write(entry: &SetCacheEntry) -> Result<(String, Option<ChunkMeta>), CacheStoreError> {
        let encoded = Self::encode_value(&entry.value)?;
        if encoded.len() <= MAX_INLINE_VALUE_BYTES {
            return Ok((encoded, None));
        }

        let chunk_count = encoded.len().div_ceil(CHUNK_PAYLOAD_BYTES);
        if chunk_count > MAX_CHUNK_COUNT {
            return Err(CacheStoreError::InvalidInput(format!(
                "Cache value of {} bytes exceeds what the DynamoDB backend can store",
                encoded.len()
            )));
        }

        let meta = ChunkMeta {
            chunk_count,
            write_id: uuid::Uuid::new_v4().simple().to_string(),
            total_bytes: encoded.len(),
        };
        Ok((encoded, Some(meta)))
    }
}

#[async_trait]
impl CacheStore for DynamoDbCacheStore {
    fn backend_name(&self) -> &'static str {
        "dynamodb"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
        self.get_internal(key, false).await
    }

    async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        // Project only the TTL attribute: enough to judge liveness, and the value stays
        // in DynamoDB instead of being billed and transferred. Chunked entries need no
        // special casing — their manifest carries the same TTL attribute.
        let output = self
            .client
            .get_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .expression_attribute_names("#ttl", TTL_ATTRIBUTE)
            .projection_expression("#ttl")
            .send()
            .await
            .map_err(database_error)?;

        let Some(item) = output.item else {
            return Ok(false);
        };

        let expires_at_ms = Self::stored_expiry_ms(&item);
        Ok(!expires_at_ms.is_some_and(|expires| expires <= Utc::now().timestamp_millis()))
    }

    async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let (encoded, chunk_meta) = Self::plan_write(&entry)?;

        if let Some(meta) = &chunk_meta
            && let Err(error) = self
                .write_chunks(
                    &entry.key,
                    meta,
                    encoded.as_bytes(),
                    entry.expires_at,
                    now_ms,
                )
                .await
        {
            // No manifest references this write id yet, so the partial chunk set is
            // safe to remove.
            let _ = self
                .delete_chunk_set(&entry.key.app_id, &Self::sort_key(&entry.key), meta)
                .await;
            return Err(error);
        }

        let item = Self::entry_item(&entry, &encoded, chunk_meta.as_ref(), now_ms);

        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(item))
            .return_values(ReturnValue::AllOld)
            .send()
            .await;

        let stored = CacheEntry {
            key: entry.key.key.clone(),
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        };

        let output = match result {
            Ok(output) => output,
            Err(error) => {
                // The put may still have committed (retried write with a lost
                // response). Only report failure when our manifest is verifiably
                // not the live one.
                if let Some(meta) = &chunk_meta
                    && self.reconcile_failed_chunked_write(&entry.key, meta).await
                {
                    return Ok(stored);
                }
                return Err(database_error(error));
            }
        };

        self.cleanup_replaced_item(
            &entry.key,
            output.attributes.as_ref(),
            chunk_meta.as_ref().map(|meta| meta.write_id.as_str()),
        )
        .await;

        Ok(stored)
    }

    async fn try_insert(
        &self,
        entry: SetCacheEntry,
    ) -> Result<Option<CacheEntry>, CacheStoreError> {
        let now_ms = Utc::now().timestamp_millis();
        let (encoded, chunk_meta) = Self::plan_write(&entry)?;

        if let Some(meta) = &chunk_meta
            && let Err(error) = self
                .write_chunks(
                    &entry.key,
                    meta,
                    encoded.as_bytes(),
                    entry.expires_at,
                    now_ms,
                )
                .await
        {
            let _ = self
                .delete_chunk_set(&entry.key.app_id, &Self::sort_key(&entry.key), meta)
                .await;
            return Err(error);
        }

        let item = Self::entry_item(&entry, &encoded, chunk_meta.as_ref(), now_ms);
        let now_seconds = now_ms / 1_000;

        let inserted = CacheEntry {
            key: entry.key.key.clone(),
            value: entry.value,
            expires_at: entry.expires_at,
            updated_at: now_ms,
        };

        // DynamoDB's TTL reaper can lag by hours, so `attribute_not_exists` alone would
        // permanently refuse to refill a key whose value expired. The second clause lets
        // a lapsed item be overwritten. An item with no TTL attribute at all fails the
        // comparison, which is correct — it never expires.
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .set_item(Some(item))
            .condition_expression("attribute_not_exists(#pk) OR #ttl <= :now")
            .expression_attribute_names("#pk", PARTITION_KEY)
            .expression_attribute_names("#ttl", TTL_ATTRIBUTE)
            .expression_attribute_values(":now", AttributeValue::N(now_seconds.to_string()))
            .return_values(ReturnValue::AllOld)
            .send()
            .await;

        match result {
            Ok(output) => {
                // When the condition let an *expired* entry be overwritten, that entry's
                // chunk set (if it had one) just became unreachable.
                self.cleanup_replaced_item(
                    &entry.key,
                    output.attributes.as_ref(),
                    chunk_meta.as_ref().map(|meta| meta.write_id.as_str()),
                )
                .await;

                Ok(Some(inserted))
            }
            Err(error) => {
                // `as_service_error` rather than `into_service_error`: the latter panics
                // on transport failures, which are exactly the errors that must surface
                // as errors instead of taking down the process.
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_conditional_check_failed_exception())
                {
                    // Either a live entry won the race, or *our own* first attempt
                    // committed and an SDK-internal retry then failed the condition
                    // against it. In the latter case the insert succeeded and the
                    // chunks are live — deleting them would wedge the key.
                    if let Some(meta) = &chunk_meta {
                        if self.reconcile_failed_chunked_write(&entry.key, meta).await {
                            return Ok(Some(inserted));
                        }
                        return Ok(None);
                    }
                    return Ok(None);
                }

                if let Some(meta) = &chunk_meta
                    && self.reconcile_failed_chunked_write(&entry.key, meta).await
                {
                    return Ok(Some(inserted));
                }
                Err(database_error(error))
            }
        }
    }

    /// Same contract as the default implementation, but the retries after a lost
    /// insert use strongly consistent reads: `try_insert`'s condition check is
    /// strongly consistent, so it can reject against an entry a lagging replica has
    /// not shown us yet — the default read-retry could then miss three times in a row
    /// and report contention while a live value exists the whole time.
    async fn get_or_set(
        &self,
        entry: SetCacheEntry,
    ) -> Result<(CacheEntry, bool), CacheStoreError> {
        if let Some(existing) = self.get_internal(&entry.key, false).await? {
            return Ok((existing, false));
        }

        for _ in 0..2 {
            if let Some(created) = self.try_insert(entry.clone()).await? {
                return Ok((created, true));
            }
            if let Some(existing) = self.get_internal(&entry.key, true).await? {
                return Ok((existing, false));
            }
            // The winning entry vanished between the failed insert and the consistent
            // read (deleted, or its TTL lapsed). Try to claim the key once more.
        }

        Err(CacheStoreError::Contention(format!(
            "key '{}' was written and removed repeatedly during get-or-set",
            entry.key.key
        )))
    }

    async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
        let output = self
            .client
            .delete_item()
            .table_name(&self.table)
            .set_key(Some(Self::primary_key(key)))
            .return_values(aws_sdk_dynamodb::types::ReturnValue::AllOld)
            .send()
            .await
            .map_err(database_error)?;

        let existed = output
            .attributes
            .as_ref()
            .is_some_and(|attrs| !attrs.is_empty());
        self.cleanup_replaced_item(key, output.attributes.as_ref(), None)
            .await;

        Ok(existed)
    }

    async fn delete_namespace(
        &self,
        app_id: &str,
        scope: flow_like_types::cache::CacheScope,
        user_id: &str,
        namespace: &str,
    ) -> Result<i64, CacheStoreError> {
        if namespace.is_empty() {
            return Err(CacheStoreError::InvalidInput(
                "Namespace invalidation requires a non-empty namespace".to_string(),
            ));
        }

        // One contiguous range: the hashed, fixed-width sort-key layout puts every
        // entry of the namespace — and every chunk item, whose sort key extends its
        // manifest's — under this prefix. The query reads exactly what it deletes.
        // Strongly consistent: this operation's whole contract is "everything is gone
        // afterwards", so it must see writes acknowledged just before the call — an
        // entry a lagging replica hides would survive its own invalidation.
        let prefix = CacheKey::namespace_sort_prefix(scope, user_id, namespace);
        let mut deleted = 0i64;
        let mut last_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let output = self
                .client
                .query()
                .table_name(&self.table)
                .consistent_read(true)
                .key_condition_expression("#pk = :app_id AND begins_with(#sk, :prefix)")
                .expression_attribute_names("#pk", PARTITION_KEY)
                .expression_attribute_names("#sk", SORT_KEY)
                .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
                .expression_attribute_values(":prefix", AttributeValue::S(prefix.clone()))
                .projection_expression(format!("{PARTITION_KEY}, {SORT_KEY}"))
                .set_exclusive_start_key(last_key)
                .send()
                .await
                .map_err(database_error)?;

            let mut requests = Vec::new();
            for item in output.items() {
                let (Some(pk), Some(sk)) = (item.get(PARTITION_KEY), item.get(SORT_KEY)) else {
                    continue;
                };

                // Chunk items are removed with their manifests but are not entries.
                if sk.as_s().is_ok_and(|sort| !sort.contains("#chunk#")) {
                    deleted += 1;
                }

                let delete = DeleteRequest::builder()
                    .key(PARTITION_KEY, pk.clone())
                    .key(SORT_KEY, sk.clone())
                    .build()
                    .map_err(database_error)?;
                requests.push(WriteRequest::builder().delete_request(delete).build());
            }

            self.run_batch_writes(requests).await?;

            last_key = output.last_evaluated_key;
            if last_key.is_none() {
                break;
            }
        }

        Ok(deleted)
    }

    async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
        // Chunk items share the app's partition key, so this sweep removes them too.
        // Strongly consistent for the same reason as delete_namespace: teardown must
        // see every acknowledged write.
        let mut deleted = 0i64;
        let mut last_key: Option<HashMap<String, AttributeValue>> = None;

        loop {
            let output = self
                .client
                .query()
                .table_name(&self.table)
                .consistent_read(true)
                .key_condition_expression("#pk = :app_id")
                .expression_attribute_names("#pk", PARTITION_KEY)
                .expression_attribute_values(":app_id", AttributeValue::S(app_id.to_string()))
                .projection_expression(format!("{PARTITION_KEY}, {SORT_KEY}"))
                .set_exclusive_start_key(last_key)
                .send()
                .await
                .map_err(database_error)?;

            let mut requests = Vec::new();
            for item in output.items() {
                let (Some(pk), Some(sk)) = (item.get(PARTITION_KEY), item.get(SORT_KEY)) else {
                    continue;
                };
                let delete = DeleteRequest::builder()
                    .key(PARTITION_KEY, pk.clone())
                    .key(SORT_KEY, sk.clone())
                    .build()
                    .map_err(database_error)?;
                requests.push(WriteRequest::builder().delete_request(delete).build());
                deleted += 1;
            }

            self.run_batch_writes(requests).await?;

            last_key = output.last_evaluated_key;
            if last_key.is_none() {
                break;
            }
        }

        Ok(deleted)
    }

    async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
        // DynamoDB evicts via the TTL attribute — chunk items carry it too. A manual
        // scan would cost far more than it reclaims, and reads already filter lapsed
        // entries.
        Ok(0)
    }

    async fn stats(&self) -> Result<Option<CacheStoreStats>, CacheStoreError> {
        // `DescribeTable` is the only O(1) source of these numbers; a `Scan` would cost
        // read capacity proportional to the table and get slower as the cache grows.
        let description = self
            .client
            .describe_table()
            .table_name(&self.table)
            .send()
            .await
            .map_err(database_error)?
            .table
            .ok_or_else(|| {
                CacheStoreError::Database(format!(
                    "DescribeTable returned no description for table '{}'",
                    self.table
                ))
            })?;

        let status = description
            .table_status()
            .map(|status| status.as_str().to_string());
        let mut note = String::from(
            "DynamoDB refreshes item count and table size approximately every six hours, so \
             both lag recent writes and TTL deletions. The item count exceeds the number of \
             cache entries whenever a value was large enough to be stored as chunk items.",
        );
        if let Some(status) = status.as_deref().filter(|status| *status != "ACTIVE") {
            note.push_str(&format!(" Table status is {status}, not ACTIVE."));
        }

        Ok(Some(CacheStoreStats {
            entries: description.item_count(),
            size_bytes: description.table_size_bytes(),
            // DescribeTable carries no timestamp for when the counts were last rolled up,
            // so the caveat has to travel in the note instead of an `observed_at`.
            note: Some(note),
            ..CacheStoreStats::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(chunk_count: usize, write_id: &str, total_bytes: usize) -> ChunkMeta {
        ChunkMeta {
            chunk_count,
            write_id: write_id.to_string(),
            total_bytes,
        }
    }

    #[test]
    fn chunk_sort_keys_are_generation_scoped_and_ordered() {
        let entry = CacheKey::app("app-1", "ns", "big").sort_key();

        let old_gen = chunk_sort_key(&entry, "writeA", 4);
        let new_gen = chunk_sort_key(&entry, "writeB", 4);
        assert_ne!(
            old_gen, new_gen,
            "the same index in two generations must never share a key"
        );

        // Zero-padding keeps lexicographic order equal to numeric order, which keeps
        // chunk sets contiguous when browsing the table.
        assert!(chunk_sort_key(&entry, "w", 2) < chunk_sort_key(&entry, "w", 10));
    }

    #[test]
    fn chunk_sort_keys_cannot_collide_with_real_entries() {
        // A user could name their key exactly like a chunk suffix; hashing the key
        // segment makes the resulting sort key different from any chunk key.
        let entry = CacheKey::app("app-1", "ns", "big");
        let chunk_key = chunk_sort_key(&entry.sort_key(), "deadbeef", 0);

        let adversarial = CacheKey::app("app-1", "ns", "big#chunk#deadbeef#00000");
        assert_ne!(chunk_key, adversarial.sort_key());
    }

    #[test]
    fn namespace_range_covers_entries_and_their_chunks() {
        use flow_like_types::cache::CacheScope;

        let entry = CacheKey::app("app-1", "reports", "big");
        let prefix = CacheKey::namespace_sort_prefix(CacheScope::App, "", "reports");

        assert!(entry.sort_key().starts_with(&prefix));
        assert!(
            chunk_sort_key(&entry.sort_key(), "w", 3).starts_with(&prefix),
            "chunk items must fall inside their namespace's delete range"
        );
        assert!(
            !CacheKey::app("app-1", "billing", "big")
                .sort_key()
                .starts_with(&prefix)
        );
    }

    #[test]
    fn split_and_assemble_roundtrip() {
        let payload = vec![7u8; CHUNK_PAYLOAD_BYTES * 2 + 123];
        let chunks = split_chunks(&payload);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), CHUNK_PAYLOAD_BYTES);
        assert_eq!(chunks[2].len(), 123);

        let meta = meta(3, "w", payload.len());
        let parts: Vec<Option<Vec<u8>>> = chunks.iter().map(|c| Some(c.to_vec())).collect();
        assert_eq!(assemble_chunks(&meta, parts), Some(payload));
    }

    #[test]
    fn assemble_refuses_missing_or_short_chunk_sets() {
        let meta_3 = meta(3, "w", 10);

        let missing_middle = vec![Some(vec![1u8; 4]), None, Some(vec![3u8; 3])];
        assert_eq!(assemble_chunks(&meta_3, missing_middle), None);

        // All chunks present but the byte count disagrees with the manifest — e.g. a
        // truncated chunk item — must not be served.
        let short = vec![Some(vec![1u8; 4]), Some(vec![2u8; 3]), Some(vec![3u8; 2])];
        assert_eq!(assemble_chunks(&meta_3, short), None);

        let wrong_arity = vec![Some(vec![1u8; 5]), Some(vec![2u8; 5])];
        assert_eq!(assemble_chunks(&meta_3, wrong_arity), None);
    }

    #[test]
    fn plan_write_switches_to_chunks_exactly_at_the_threshold() {
        let inline_entry = SetCacheEntry {
            key: CacheKey::app("app", "", "small"),
            value: serde_json::json!("x".repeat(1024)),
            expires_at: None,
        };
        let (_, meta) = DynamoDbCacheStore::plan_write(&inline_entry).unwrap();
        assert!(meta.is_none());

        // A string of this length serializes to more than the inline ceiling once the
        // surrounding quotes are added.
        let big_entry = SetCacheEntry {
            key: CacheKey::app("app", "", "big"),
            value: serde_json::json!("x".repeat(MAX_INLINE_VALUE_BYTES)),
            expires_at: None,
        };
        let (encoded, meta) = DynamoDbCacheStore::plan_write(&big_entry).unwrap();
        let meta = meta.expect("oversized value must be chunked");
        assert_eq!(meta.total_bytes, encoded.len());
        assert_eq!(
            meta.chunk_count,
            encoded.len().div_ceil(CHUNK_PAYLOAD_BYTES)
        );
        assert!(!meta.write_id.is_empty());

        // Two writes of the same value must land in distinct generations.
        let (_, second) = DynamoDbCacheStore::plan_write(&big_entry).unwrap();
        assert_ne!(meta.write_id, second.unwrap().write_id);
    }

    #[test]
    fn chunk_meta_rejects_garbage_manifests() {
        let mut item = HashMap::from([
            (
                CHUNK_COUNT_ATTRIBUTE.to_string(),
                AttributeValue::N("3".to_string()),
            ),
            (
                CHUNK_WRITE_ID_ATTRIBUTE.to_string(),
                AttributeValue::S("w".to_string()),
            ),
            (
                CHUNK_TOTAL_BYTES_ATTRIBUTE.to_string(),
                AttributeValue::N("900".to_string()),
            ),
        ]);
        assert_eq!(
            DynamoDbCacheStore::chunk_meta(&item),
            Some(meta(3, "w", 900))
        );

        item.insert(
            CHUNK_COUNT_ATTRIBUTE.to_string(),
            AttributeValue::N("0".to_string()),
        );
        assert_eq!(DynamoDbCacheStore::chunk_meta(&item), None);

        item.insert(
            CHUNK_COUNT_ATTRIBUTE.to_string(),
            AttributeValue::N((MAX_CHUNK_COUNT + 1).to_string()),
        );
        assert_eq!(DynamoDbCacheStore::chunk_meta(&item), None);

        item.insert(
            CHUNK_COUNT_ATTRIBUTE.to_string(),
            AttributeValue::N("not-a-number".to_string()),
        );
        assert_eq!(DynamoDbCacheStore::chunk_meta(&item), None);
    }

    #[test]
    fn expiry_seconds_rounds_up() {
        assert_eq!(expiry_seconds(1), 1);
        assert_eq!(expiry_seconds(999), 1);
        assert_eq!(expiry_seconds(1_000), 1);
        assert_eq!(expiry_seconds(1_001), 2);
    }
}
