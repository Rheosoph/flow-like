use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use flow_like_types::{Result, Value, json::json, reqwest::Url};
use futures::stream::BoxStream;
use lance::dataset::CommitBuilder;
use lance::dataset::transaction::{Operation, Transaction, UpdateMap};
use lance::session::Session;
use lance_io::object_store::{
    ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
};
use lance_object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, local::LocalFileSystem,
    path::Path,
};

use super::super::VectorStore;
use super::{DatabaseSelector, LanceDBVectorStore, connect_lance};

const ROOT: &str = "countsim://bucket/db";
const TABLE: &str = "items";

/// Counts object reads, the requests S3 bills as GETs.
#[derive(Debug)]
struct CountingStore {
    inner: LocalFileSystem,
    gets: AtomicUsize,
}

impl fmt::Display for CountingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("countsim")
    }
}

#[async_trait::async_trait]
impl ObjectStore for CountingStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        options: PutOptions,
    ) -> lance_object_store::Result<PutResult> {
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: PutMultipartOptions,
    ) -> lance_object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> lance_object_store::Result<GetResult> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, lance_object_store::Result<Path>>,
    ) -> BoxStream<'static, lance_object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> BoxStream<'static, lance_object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> lance_object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> lance_object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }

    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: RenameOptions,
    ) -> lance_object_store::Result<()> {
        self.inner.rename_opts(from, to, options).await
    }
}

/// Serves `countsim://` with S3's 64 KiB blocks. Local listings are unordered, so Lance
/// lists every manifest to find the latest instead of reading the first S3 page.
#[derive(Debug)]
struct CountingProvider(Arc<CountingStore>);

#[async_trait::async_trait]
impl ObjectStoreProvider for CountingProvider {
    async fn new_store(&self, url: Url, _: &ObjectStoreParams) -> lance::Result<LanceStore> {
        Ok(LanceStore::new(
            self.0.clone(),
            url,
            Some(64 * 1024),
            None,
            false,
            false,
            8,
            0,
            None,
        ))
    }
}

struct CountingBucket {
    directory: std::path::PathBuf,
    store: Arc<CountingStore>,
}

impl Drop for CountingBucket {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl CountingBucket {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "flow-like-compaction-{}",
            flow_like_types::create_id()
        ));
        std::fs::create_dir_all(&directory)?;
        let store = Arc::new(CountingStore {
            inner: LocalFileSystem::new_with_prefix(&directory)?,
            gets: AtomicUsize::new(0),
        });
        Ok(Self { directory, store })
    }

    /// A fresh Session per call, like one server executor run.
    async fn open(&self) -> Result<LanceDBVectorStore> {
        let registry = Arc::new(ObjectStoreRegistry::default());
        registry.insert("countsim", Arc::new(CountingProvider(self.store.clone())));
        let session = Arc::new(Session::new(0, 0, registry));
        let connection = connect_lance(ROOT).session(session).execute().await?;
        Ok(LanceDBVectorStore::from_connection(connection, TABLE.into()).await)
    }

    fn take_gets(&self) -> usize {
        self.store.gets.swap(0, Ordering::Relaxed)
    }
}

fn row(id: usize, round: usize) -> Value {
    json!({
        "id": format!("item-{id}"),
        "title": format!("Item {id} seen in round {round}"),
        "state": if id.is_multiple_of(3) { "read" } else { "new" },
        "seen": round as i64,
    })
}

fn rows(ids: std::ops::Range<usize>, round: usize) -> Vec<Value> {
    ids.map(|id| row(id, round)).collect()
}

/// One commit, and so one fragment, per row.
async fn insert_one_by_one(
    store: &mut LanceDBVectorStore,
    ids: std::ops::Range<usize>,
    round: usize,
) -> Result<()> {
    for id in ids {
        store.insert(rows(id..id + 1, round)).await?;
    }
    Ok(())
}

async fn fragment_ids(store: &LanceDBVectorStore) -> Result<Vec<u64>> {
    let table = store.raw().await?;
    let dataset = table.dataset().unwrap().get().await?;
    Ok(dataset
        .manifest()
        .fragments
        .iter()
        .map(|fragment| fragment.id)
        .collect())
}

async fn historical_row_counts(store: &LanceDBVectorStore, latest: u64) -> Result<Vec<usize>> {
    let mut counts = Vec::new();
    for version in 1..=latest {
        let historical = store
            .checkout(DatabaseSelector {
                version: Some(version),
                read_only: true,
                ..Default::default()
            })
            .await?;
        counts.push(historical.count(None).await?);
    }
    Ok(counts)
}

#[tokio::test]
async fn ordinary_writes_do_not_compact_or_add_maintenance_commits() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    store.upsert(rows(0..1, 0), "id".into()).await?;
    // Initialize the upsert key before checking each operation's commit count.
    store.upsert(rows(1..2, 0), "id".into()).await?;
    for id in 2..40 {
        let before = store.raw().await?.version().await?;
        match id % 3 {
            0 => store.insert(rows(id..id + 1, 0)).await?,
            1 => store.upsert(rows(id..id + 1, 0), "id".into()).await?,
            _ => {
                let table = store.raw().await?;
                let fields = table.schema().await?.fields().iter().cloned().collect();
                let mut reader = crate::arrow_utils::value_to_batch_reader_with_fields(
                    rows(id..id + 1, 0),
                    Some(fields),
                )?;
                store.insert_record_batch(reader.next().unwrap()?).await?;
            }
        }
        assert_eq!(store.raw().await?.version().await?, before + 1);
        assert_eq!(fragment_ids(&store).await?.len(), id + 1);
    }
    let before = store.raw().await?.version().await?;
    store
        .update(
            "id = 'item-0'",
            std::collections::HashMap::from([("state".to_string(), json!("archived"))]),
        )
        .await?;
    assert_eq!(store.raw().await?.version().await?, before + 1);
    assert_eq!(fragment_ids(&store).await?.len(), 40);
    assert_eq!(store.count(None).await?, 40);
    assert_eq!(store.count(Some("state = 'archived'".into())).await?, 1);
    Ok(())
}

#[tokio::test]
async fn explicit_compaction_reduces_reads_and_retains_historical_versions() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    for round in 0..40 {
        store.insert(rows(round * 5..round * 5 + 5, round)).await?;
    }
    assert_eq!(fragment_ids(&store).await?.len(), 40);
    let read = bucket.open().await?;
    bucket.take_gets();
    assert_eq!(read.count(Some("state = 'new'".into())).await?, 133);
    assert_eq!(read.filter("seen > 30", None, 100, 0).await?.len(), 45);
    let fragmented_gets = bucket.take_gets();

    store.optimize(true).await?;
    assert_eq!(fragment_ids(&store).await?.len(), 1);
    let read = bucket.open().await?;
    bucket.take_gets();
    assert_eq!(read.count(Some("state = 'new'".into())).await?, 133);
    assert_eq!(read.filter("seen > 30", None, 100, 0).await?.len(), 45);
    let compacted_gets = bucket.take_gets();
    assert!(
        compacted_gets * 2 < fragmented_gets,
        "compaction must reduce object reads: {compacted_gets} vs {fragmented_gets}"
    );
    let first = store
        .checkout(DatabaseSelector {
            version: Some(1),
            read_only: true,
            ..Default::default()
        })
        .await?;
    assert_eq!(first.count(None).await?, 5);
    Ok(())
}

#[tokio::test]
async fn a_stale_writer_commits_after_explicit_compaction() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut compacting = bucket.open().await?;
    compacting.insert(rows(0..1, 0)).await?;
    let stale = bucket.open().await?;
    insert_one_by_one(&mut compacting, 1..20, 0).await?;
    compacting.optimize(true).await?;
    assert_eq!(fragment_ids(&compacting).await?.len(), 1);
    stale
        .update(
            "id = 'item-0'",
            std::collections::HashMap::from([("state".to_string(), json!("archived"))]),
        )
        .await?;
    stale.delete("id = 'item-3'").await?;
    let reader = bucket.open().await?;
    assert_eq!(reader.count(None).await?, 19);
    assert_eq!(reader.count(Some("state = 'archived'".into())).await?, 1);
    assert_eq!(reader.count(Some("id = 'item-3'".into())).await?, 0);
    Ok(())
}

#[tokio::test]
async fn explicit_optimize_does_not_rewrite_retention_policy() -> Result<()> {
    for keep_versions in [true, false] {
        let bucket = CountingBucket::new()?;
        let mut store = bucket.open().await?;
        store.insert(rows(0..3, 0)).await?;
        let stale = bucket.open().await?;
        let table = store.raw().await?;
        let wrapper = table.dataset().unwrap();
        let mut dataset = wrapper.get().await?.as_ref().clone();
        let configured = [
            ("lance.auto_cleanup.interval", "20"),
            ("lance.auto_cleanup.older_than", "14days"),
            ("lance.auto_cleanup.retain_versions", "3"),
        ];
        dataset.update_config(configured).await?;
        wrapper.update(dataset);
        let before = table.version().await?;
        if keep_versions {
            let error = stale
                .optimize(true)
                .await
                .expect_err("a stale handle must refresh and refuse configured automatic cleanup");
            assert!(
                error
                    .to_string()
                    .contains("automatic cleanup is configured")
            );
            assert_eq!(stale.raw().await?.version().await?, before);
        } else {
            store.optimize(false).await?;
        }
        let table = bucket.open().await?.raw().await?;
        let dataset = table.dataset().unwrap().get().await?;
        for (key, expected) in configured {
            assert_eq!(
                dataset.config().get(key).map(String::as_str),
                Some(expected)
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn retaining_optimize_does_not_unpin_a_shared_raw_handle() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    insert_one_by_one(&mut store, 0..3, 0).await?;
    let table = store.raw().await?;
    table.checkout(1).await?;

    store
        .optimize(true)
        .await
        .expect_err("maintenance must refuse a pinned raw handle");
    assert_eq!(table.version().await?, 1);
    assert_eq!(store.raw().await?.version().await?, 1);
    assert_eq!(store.count(None).await?, 1);
    let latest = bucket.open().await?;
    assert_eq!(latest.raw().await?.version().await?, 3);
    assert_eq!(fragment_ids(&latest).await?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn retaining_optimize_rejects_auto_cleanup_before_any_maintenance_commit() -> Result<()> {
    for interval in [0, 1] {
        let bucket = CountingBucket::new()?;
        let mut store = bucket.open().await?;
        insert_one_by_one(&mut store, 0..3, 0).await?;
        let table = store.raw().await?;
        let wrapper = table.dataset().unwrap();
        let dataset = wrapper.get().await?;
        let transaction = Transaction::new(
            dataset.manifest().version,
            Operation::UpdateConfig {
                config_updates: Some(UpdateMap {
                    update_entries: vec![
                        (
                            "lance.auto_cleanup.interval".to_string(),
                            interval.to_string(),
                        )
                            .into(),
                        ("lance.auto_cleanup.older_than", "0s").into(),
                    ],
                    replace: false,
                }),
                table_metadata_updates: None,
                schema_metadata_updates: None,
                field_metadata_updates: Default::default(),
            },
            None,
        );
        // Suppress cleanup on this setup commit itself. LanceDB's add path does
        // not forward WriteParams.skip_auto_cleanup to its final CommitBuilder.
        let dataset = CommitBuilder::new(dataset)
            .with_skip_auto_cleanup(true)
            .execute(transaction)
            .await?;
        wrapper.update(dataset);
        let before = table.version().await?;
        assert_eq!(before, 4);
        let fragments = fragment_ids(&store).await?;
        assert_eq!(fragments.len(), 3);
        let historical_counts = historical_row_counts(&store, before).await?;
        assert_eq!(historical_counts, vec![1, 2, 3, 3]);
        let policy = wrapper.get().await?.config().clone();
        assert_eq!(
            policy.get("lance.auto_cleanup.interval"),
            Some(&interval.to_string())
        );
        assert_eq!(
            policy
                .get("lance.auto_cleanup.older_than")
                .map(String::as_str),
            Some("0s")
        );

        let error = store
            .optimize(true)
            .await
            .expect_err("keeping versions must reject even a per-commit cleanup policy");
        assert!(error.to_string().contains("keep_versions=true"));
        let reader = bucket.open().await?;
        assert_eq!(reader.raw().await?.version().await?, before);
        assert_eq!(fragment_ids(&reader).await?, fragments);
        assert_eq!(reader.count(None).await?, 3);
        assert_eq!(
            historical_row_counts(&reader, before).await?,
            historical_counts
        );
        assert_eq!(
            reader.raw().await?.dataset().unwrap().get().await?.config(),
            &policy
        );
    }
    Ok(())
}

#[tokio::test]
async fn cleanup_settings_without_an_interval_do_not_block_retaining_optimize() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    insert_one_by_one(&mut store, 0..3, 0).await?;
    let table = store.raw().await?;
    let wrapper = table.dataset().unwrap();
    let mut dataset = wrapper.get().await?.as_ref().clone();
    dataset
        .update_config([("lance.auto_cleanup.older_than", "0s")])
        .await?;
    wrapper.update(dataset);

    store.optimize(true).await?;
    assert_eq!(fragment_ids(&store).await?.len(), 1);
    let first = store
        .checkout(DatabaseSelector {
            version: Some(1),
            read_only: true,
            ..Default::default()
        })
        .await?;
    assert_eq!(first.count(None).await?, 1);
    assert_eq!(
        wrapper
            .get()
            .await?
            .config()
            .get("lance.auto_cleanup.older_than")
            .map(String::as_str),
        Some("0s")
    );
    Ok(())
}
