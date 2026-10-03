use std::{future::Future, time::Duration};

use flow_like::flow::execution::{
    log_query::scan_log_summary,
    log_summary::{LogSummary, SUMMARY_VERSION},
};
use flow_like_storage::lancedb::Table;
use flow_like_types::{Result, tokio};
use serde::Serialize;

use crate::cache::{CacheBackendHandle, CacheStoreError, PlatformCache};

const CACHE_NAMESPACE: &str = "run-log-summaries-v1";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_SUMMARY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
struct SummaryIdentity {
    subject: String,
    app: String,
    board: String,
    run: String,
    uri: String,
    version: u64,
    // Versions restart if a table is recreated at the same path.
    timestamp_nanos: u128,
    transaction_file: Option<String>,
}

impl SummaryIdentity {
    fn cache_key(&self) -> Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(&(SUMMARY_VERSION, self))?)
            .to_hex()
            .to_string())
    }
}

pub(super) async fn platform_cache(handle: &CacheBackendHandle) -> Option<PlatformCache> {
    cache_operation("initialize", handle.platform()).await
}

async fn cache_operation<T>(
    action: &str,
    operation: impl Future<Output = std::result::Result<T, CacheStoreError>>,
) -> Option<T> {
    match tokio::time::timeout(CACHE_TIMEOUT, operation).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            tracing::debug!(action, error = %error, "Run log summary cache unavailable");
            None
        }
        Err(_) => {
            tracing::debug!(action, "Run log summary cache timed out");
            None
        }
    }
}

/// The caller checks permissions and reads the final sidecar with scoped
/// credentials on every request. Only fallback scan results enter the reserved
/// platform cache, which workflow cache routes cannot address or overwrite.
pub(super) async fn summarize(
    cache: Option<&PlatformCache>,
    subject: &str,
    app: &str,
    board: &str,
    run: &str,
    table: &Table,
) -> Result<LogSummary> {
    let Some(cache) = cache else {
        return scan_log_summary(table, None).await;
    };
    // Remote tables without a native manifest do not provide the identity
    // needed to safely reuse a scan result.
    let Some(wrapper) = table.dataset() else {
        return scan_log_summary(table, None).await;
    };

    // This is a request-local table, never the executor's writer handle.
    // Pin it before reading its identity so an append during the scan cannot
    // cache newer rows under an older version. The next request opens latest.
    let version = table.version().await?;
    table.checkout(version).await?;
    let identity = {
        let dataset = wrapper.get().await?;
        let manifest = dataset.manifest();
        SummaryIdentity {
            subject: subject.to_owned(),
            app: app.to_owned(),
            board: board.to_owned(),
            run: run.to_owned(),
            uri: dataset.uri().to_owned(),
            version: manifest.version,
            timestamp_nanos: manifest.timestamp_nanos,
            transaction_file: manifest.transaction_file.clone(),
        }
    };
    get_or_compute(
        Some(cache),
        &identity.cache_key()?,
        scan_log_summary(table, None),
    )
    .await
}

async fn get_or_compute(
    cache: Option<&PlatformCache>,
    key: &str,
    compute: impl Future<Output = Result<LogSummary>>,
) -> Result<LogSummary> {
    if let Some(cache) = cache
        && let Some(Some(summary)) =
            cache_operation("read", cache.get::<LogSummary>(CACHE_NAMESPACE, key)).await
        && summary.version == SUMMARY_VERSION
    {
        return Ok(summary);
    }

    // Failed scans are never cached. Cache failures must not hide a valid scan.
    let summary = compute.await?;
    if let Some(cache) = cache {
        match serde_json::to_vec(&summary) {
            Ok(bytes) if bytes.len() <= MAX_SUMMARY_BYTES => {
                cache_operation(
                    "write",
                    cache.set(CACHE_NAMESPACE, key, &summary, CACHE_TTL),
                )
                .await;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(error = %error, "Could not serialize run log summary for caching");
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CacheEntry, CacheKey, CacheStore, PLATFORM_APP_ID, SetCacheEntry};
    use async_trait::async_trait;
    use flow_like::flow::execution::{LogLevel, log::LogMessage, log_summary::LogSummaryBuilder};
    use flow_like_storage::{
        arrow_array::{RecordBatchIterator, RecordBatchReader},
        lancedb,
    };
    use flow_like_types::{anyhow, cache::CacheScope};
    use std::{
        collections::HashMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicI64, Ordering},
        },
    };

    type CacheResult<T> = std::result::Result<T, CacheStoreError>;

    #[derive(Debug)]
    struct MemoryStore {
        entries: Mutex<HashMap<String, CacheEntry>>,
        now_ms: AtomicI64,
        fail_reads: AtomicBool,
        fail_writes: AtomicBool,
        hang_reads: AtomicBool,
        hang_writes: AtomicBool,
    }

    impl Default for MemoryStore {
        fn default() -> Self {
            Self {
                entries: Mutex::new(HashMap::new()),
                now_ms: AtomicI64::new(chrono::Utc::now().timestamp_millis()),
                fail_reads: AtomicBool::new(false),
                fail_writes: AtomicBool::new(false),
                hang_reads: AtomicBool::new(false),
                hang_writes: AtomicBool::new(false),
            }
        }
    }

    impl MemoryStore {
        fn slot(key: &CacheKey) -> String {
            format!("{}|{}", key.app_id, key.sort_key())
        }
    }

    #[async_trait]
    impl CacheStore for MemoryStore {
        fn backend_name(&self) -> &'static str {
            "memory"
        }

        async fn get(&self, key: &CacheKey) -> CacheResult<Option<CacheEntry>> {
            if self.hang_reads.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(CacheStoreError::Connection("unavailable".into()));
            }
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(&Self::slot(key))
                .filter(|entry| !entry.is_expired_at(self.now_ms.load(Ordering::SeqCst)))
                .cloned())
        }

        async fn exists(&self, key: &CacheKey) -> CacheResult<bool> {
            Ok(self.get(key).await?.is_some())
        }

        async fn set(&self, entry: SetCacheEntry) -> CacheResult<CacheEntry> {
            if self.hang_writes.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail_writes.load(Ordering::SeqCst) {
                return Err(CacheStoreError::Connection("unavailable".into()));
            }
            let slot = Self::slot(&entry.key);
            let value = CacheEntry {
                key: entry.key.key,
                value: entry.value,
                expires_at: entry.expires_at,
                updated_at: self.now_ms.load(Ordering::SeqCst),
            };
            self.entries.lock().unwrap().insert(slot, value.clone());
            Ok(value)
        }

        async fn try_insert(&self, _entry: SetCacheEntry) -> CacheResult<Option<CacheEntry>> {
            unimplemented!("summary caching does not acquire reservations")
        }
        async fn delete(&self, _key: &CacheKey) -> CacheResult<bool> {
            unimplemented!()
        }
        async fn delete_namespace(
            &self,
            _app: &str,
            _scope: CacheScope,
            _user: &str,
            _namespace: &str,
        ) -> CacheResult<i64> {
            unimplemented!()
        }
        async fn delete_app(&self, _app: &str) -> CacheResult<i64> {
            unimplemented!()
        }
        async fn delete_expired(&self) -> CacheResult<i64> {
            unimplemented!()
        }
    }

    fn identity() -> SummaryIdentity {
        SummaryIdentity {
            subject: "reader".into(),
            app: "app".into(),
            board: "board".into(),
            run: "run".into(),
            uri: "memory://logs/runs/app/board/run.lance".into(),
            version: 1,
            timestamp_nanos: 123,
            transaction_file: Some("commit-1.txn".into()),
        }
    }

    fn summary(total: u64) -> LogSummary {
        let mut value = LogSummaryBuilder::default().finish(false, None);
        value.total = total;
        value
    }

    #[tokio::test]
    async fn separate_api_instances_reuse_the_shared_platform_cache_until_expiry() {
        let store = Arc::new(MemoryStore::default());
        let first_instance = PlatformCache::new(store.clone());
        let second_instance = PlatformCache::new(store.clone());
        let key = identity().cache_key().unwrap();
        get_or_compute(Some(&first_instance), &key, async { Ok(summary(7)) })
            .await
            .unwrap();
        let reused = get_or_compute(Some(&second_instance), &key, async {
            panic!("second instance rescanned")
        })
        .await
        .unwrap();
        assert_eq!(reused.total, 7);
        assert!(
            store
                .get(&CacheKey::app("app", CACHE_NAMESPACE, &key))
                .await
                .unwrap()
                .is_none()
        );
        let stored = store
            .get(&CacheKey::app(PLATFORM_APP_ID, CACHE_NAMESPACE, &key))
            .await
            .unwrap()
            .unwrap();
        store
            .now_ms
            .store(stored.expires_at.unwrap(), Ordering::SeqCst);
        assert_eq!(
            get_or_compute(Some(&second_instance), &key, async { Ok(summary(8)) })
                .await
                .unwrap()
                .total,
            8
        );
    }

    #[tokio::test]
    async fn another_principal_scope_or_table_identity_cannot_reuse_the_summary() {
        let store = Arc::new(MemoryStore::default());
        let cache = PlatformCache::new(store);
        get_or_compute(Some(&cache), &identity().cache_key().unwrap(), async {
            Ok(summary(1))
        })
        .await
        .unwrap();
        let mut alternatives = Vec::new();
        let mut other = identity();
        other.subject = "different-reader".into();
        alternatives.push(other);
        let mut other = identity();
        other.app = "different-app".into();
        alternatives.push(other);
        let mut other = identity();
        other.board = "different-board".into();
        alternatives.push(other);
        let mut other = identity();
        other.run = "different-run".into();
        alternatives.push(other);
        let mut other = identity();
        other.uri = "memory://other-bucket/run.lance".into();
        alternatives.push(other);
        let mut other = identity();
        other.version += 1;
        alternatives.push(other);
        let mut other = identity();
        other.timestamp_nanos += 1;
        alternatives.push(other);
        let mut other = identity();
        other.transaction_file = Some("recreated.txn".into());
        alternatives.push(other);
        for other in alternatives {
            let actual = get_or_compute(Some(&cache), &other.cache_key().unwrap(), async {
                Ok(summary(2))
            })
            .await
            .unwrap();
            assert_eq!(actual.total, 2);
        }
    }

    #[tokio::test]
    async fn cache_errors_do_not_hide_valid_scans_and_scan_errors_are_not_cached() {
        let store = Arc::new(MemoryStore::default());
        let cache = PlatformCache::new(store.clone());
        let key = identity().cache_key().unwrap();
        store.fail_reads.store(true, Ordering::SeqCst);
        store.fail_writes.store(true, Ordering::SeqCst);
        assert_eq!(
            get_or_compute(Some(&cache), &key, async { Ok(summary(3)) })
                .await
                .unwrap()
                .total,
            3
        );
        assert!(store.entries.lock().unwrap().is_empty());
        assert_eq!(
            get_or_compute(None, &key, async { Ok(summary(4)) })
                .await
                .unwrap()
                .total,
            4
        );
        store.fail_reads.store(false, Ordering::SeqCst);
        store.fail_writes.store(false, Ordering::SeqCst);
        assert!(
            get_or_compute(Some(&cache), &key, async { Err(anyhow!("failed scan")) })
                .await
                .is_err()
        );
        assert!(store.entries.lock().unwrap().is_empty());
        cache
            .set(
                CACHE_NAMESPACE,
                &key,
                &serde_json::json!({"invalid":"summary"}),
                CACHE_TTL,
            )
            .await
            .unwrap();
        assert_eq!(
            get_or_compute(Some(&cache), &key, async { Ok(summary(5)) })
                .await
                .unwrap()
                .total,
            5
        );
        let mut incompatible = summary(99);
        incompatible.version = SUMMARY_VERSION + 1;
        cache
            .set(CACHE_NAMESPACE, &key, &incompatible, CACHE_TTL)
            .await
            .unwrap();
        assert_eq!(
            get_or_compute(Some(&cache), &key, async { Ok(summary(6)) })
                .await
                .unwrap()
                .total,
            6
        );
    }

    #[tokio::test(start_paused = true)]
    async fn slow_cache_operations_have_bounded_latency() {
        let started = tokio::time::Instant::now();
        let unavailable: Option<PlatformCache> = cache_operation(
            "initialize",
            std::future::pending::<CacheResult<PlatformCache>>(),
        )
        .await;
        assert!(unavailable.is_none());
        assert_eq!(started.elapsed(), CACHE_TIMEOUT);
        let store = Arc::new(MemoryStore::default());
        store.hang_reads.store(true, Ordering::SeqCst);
        store.hang_writes.store(true, Ordering::SeqCst);
        let cache = PlatformCache::new(store);
        let started = tokio::time::Instant::now();
        assert_eq!(
            get_or_compute(Some(&cache), &identity().cache_key().unwrap(), async {
                Ok(summary(4))
            })
            .await
            .unwrap()
            .total,
            4
        );
        assert_eq!(started.elapsed(), CACHE_TIMEOUT * 2);
    }

    #[tokio::test]
    async fn large_summaries_are_returned_without_writing_chunked_cache_entries() {
        let store = Arc::new(MemoryStore::default());
        let cache = PlatformCache::new(store.clone());
        let mut large = summary(1);
        large
            .nodes
            .insert("n".repeat(MAX_SUMMARY_BYTES), [1, 0, 0, 0, 0]);
        let result = get_or_compute(Some(&cache), &identity().cache_key().unwrap(), async {
            Ok(large)
        })
        .await
        .unwrap();
        assert_eq!(result.total, 1);
        assert!(store.entries.lock().unwrap().is_empty());
    }

    fn rows(messages: &[&str]) -> Box<dyn RecordBatchReader + Send> {
        let batch = LogMessage::into_arrow(
            messages
                .iter()
                .map(|message| LogMessage::new(message, LogLevel::Info, None)),
        )
        .unwrap();
        let schema = batch.schema();
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema))
    }

    #[tokio::test]
    async fn appended_or_recreated_tables_do_not_return_an_old_summary() {
        let cache = PlatformCache::new(Arc::new(MemoryStore::default()));
        let db = lancedb::connect(&format!(
            "memory://summary-cache-{}",
            flow_like_types::create_id()
        ))
        .execute()
        .await
        .unwrap();
        let writer = db
            .create_table("run", rows(&["first"]))
            .execute()
            .await
            .unwrap();
        let first = db.open_table("run").execute().await.unwrap();
        assert_eq!(
            summarize(Some(&cache), "reader", "app", "board", "run", &first)
                .await
                .unwrap()
                .total,
            1
        );
        writer.add(rows(&["second"])).execute().await.unwrap();
        assert_eq!(first.count_rows(None).await.unwrap(), 1);
        let appended = db.open_table("run").execute().await.unwrap();
        assert_eq!(
            summarize(Some(&cache), "reader", "app", "board", "run", &appended)
                .await
                .unwrap()
                .total,
            2
        );
        db.drop_table("run", &[]).await.unwrap();
        let replacement = db
            .create_table("run", rows(&["a", "b", "c"]))
            .execute()
            .await
            .unwrap();
        assert_eq!(replacement.version().await.unwrap(), 1);
        assert_eq!(
            summarize(Some(&cache), "reader", "app", "board", "run", &replacement)
                .await
                .unwrap()
                .total,
            3
        );
    }
}
