use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use flow_like_types::{Result, Value, json::json, reqwest::Url};
use futures::stream::BoxStream;
use lance::session::Session;
use lance_io::object_store::{
    ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
};
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, local::LocalFileSystem,
    path::Path,
};

use super::super::VectorStore;
use super::{
    AUTO_COMPACTION_FRAGMENTS, DatabaseSelector, LEGACY_AUTO_CLEANUP, LanceDBVectorStore,
    connect_lance,
};

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
    ) -> object_store::Result<PutResult> {
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }

    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: RenameOptions,
    ) -> object_store::Result<()> {
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

/// Appends through LanceDB directly, like writes made before automatic compaction.
async fn append_without_compaction(store: &LanceDBVectorStore, items: Vec<Value>) -> Result<()> {
    let table = store.raw().await?;
    let fields = table.schema().await?.fields().iter().cloned().collect();
    let reader = crate::arrow_utils::value_to_batch_reader_with_fields(items, Some(fields))?;
    table.add(reader).execute().await?;
    Ok(())
}

async fn set_config(store: &LanceDBVectorStore, entries: [(&str, &str); 2]) -> Result<()> {
    let table = store.raw().await?;
    let wrapper = table.dataset().unwrap();
    let mut dataset = wrapper.get().await?.as_ref().clone();
    dataset.update_config(entries).await?;
    wrapper.update(dataset);
    Ok(())
}

#[tokio::test]
async fn small_writes_are_compacted_without_pruning_versions() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    for round in 0..40 {
        let items = rows(round..round + 1, round);
        if round.is_multiple_of(2) {
            store.insert(items).await?;
        } else {
            store.upsert(items, "id".into()).await?;
        }
    }

    assert!(fragment_ids(&store).await?.len() < AUTO_COMPACTION_FRAGMENTS);
    assert_eq!(store.count(None).await?, 40);
    assert_eq!(store.count(Some("id = 'item-27'".into())).await?, 1);
    let first = store
        .checkout(DatabaseSelector {
            version: Some(1),
            read_only: true,
            ..Default::default()
        })
        .await?;
    assert_eq!(
        first.count(None).await?,
        1,
        "the first version must stay readable"
    );
    Ok(())
}

#[tokio::test]
async fn compaction_cuts_the_requests_of_the_next_run() -> Result<()> {
    let compacted = CountingBucket::new()?;
    let mut store = compacted.open().await?;
    for round in 0..40 {
        store.insert(rows(round * 5..round * 5 + 5, round)).await?;
    }
    let fragmented = CountingBucket::new()?;
    let mut store = fragmented.open().await?;
    store.insert(rows(0..5, 0)).await?;
    for round in 1..40 {
        append_without_compaction(&store, rows(round * 5..round * 5 + 5, round)).await?;
    }

    let mut gets = Vec::new();
    for bucket in [&compacted, &fragmented] {
        let store = bucket.open().await?;
        bucket.take_gets();
        assert_eq!(store.count(Some("state = 'new'".into())).await?, 133);
        assert_eq!(store.filter("seen > 30", None, 100, 0).await?.len(), 45);
        gets.push(bucket.take_gets());
    }
    assert!(
        gets[0] * 2 < gets[1],
        "a run reading compacted data must send far fewer requests: {gets:?}"
    );
    Ok(())
}

#[tokio::test]
async fn compaction_leaves_indexed_fragments_and_their_index() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    store.insert(rows(0..300, 0)).await?;
    store.index("seen", Some("BTREE")).await?;
    let indexed = fragment_ids(&store).await?;
    insert_one_by_one(&mut store, 300..300 + AUTO_COMPACTION_FRAGMENTS, 1).await?;

    let after = fragment_ids(&store).await?;
    assert_eq!(
        after.first(),
        indexed.first(),
        "the indexed fragment was rewritten"
    );
    assert_eq!(
        after.len(),
        2,
        "unindexed small writes must merge: {after:?}"
    );
    assert_eq!(store.list_indices().await?.len(), 1);
    assert_eq!(store.count(Some("seen = 0".into())).await?, 300);
    assert_eq!(
        store.count(Some("seen = 1".into())).await?,
        AUTO_COMPACTION_FRAGMENTS
    );
    Ok(())
}

#[tokio::test]
async fn compaction_leaves_a_large_leading_fragment() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut store = bucket.open().await?;
    store.insert(rows(0..10_000, 0)).await?;
    let large = fragment_ids(&store).await?;
    insert_one_by_one(&mut store, 10_000..10_000 + AUTO_COMPACTION_FRAGMENTS, 1).await?;

    let after = fragment_ids(&store).await?;
    assert_eq!(
        after.first(),
        large.first(),
        "the large fragment was rewritten"
    );
    assert_eq!(
        after.len(),
        2,
        "small writes behind it must merge: {after:?}"
    );
    assert_eq!(store.count(None).await?, 10_000 + AUTO_COMPACTION_FRAGMENTS);
    Ok(())
}

#[tokio::test]
async fn a_stale_writer_commits_after_another_run_compacts() -> Result<()> {
    let bucket = CountingBucket::new()?;
    let mut compacting = bucket.open().await?;
    compacting.insert(rows(0..1, 0)).await?;
    let stale = bucket.open().await?;
    insert_one_by_one(&mut compacting, 1..AUTO_COMPACTION_FRAGMENTS, 0).await?;
    assert_eq!(fragment_ids(&compacting).await?.len(), 1);

    stale
        .update(
            "id = 'item-0'",
            std::collections::HashMap::from([("state".to_string(), json!("archived"))]),
        )
        .await?;
    stale.delete("id = 'item-3'").await?;

    let reader = bucket.open().await?;
    assert_eq!(reader.count(None).await?, AUTO_COMPACTION_FRAGMENTS - 1);
    assert_eq!(reader.count(Some("state = 'archived'".into())).await?, 1);
    assert_eq!(reader.count(Some("id = 'item-3'".into())).await?, 0);
    Ok(())
}

#[tokio::test]
async fn pruning_optimize_drops_only_lances_legacy_auto_cleanup() -> Result<()> {
    let custom = [
        ("lance.auto_cleanup.interval", "5"),
        ("lance.auto_cleanup.older_than", "14days"),
    ];
    for (config, keep_versions, kept) in [
        (LEGACY_AUTO_CLEANUP, false, false),
        (LEGACY_AUTO_CLEANUP, true, true),
        (custom, false, true),
    ] {
        let bucket = CountingBucket::new()?;
        let mut store = bucket.open().await?;
        store.insert(rows(0..3, 0)).await?;
        set_config(&store, config).await?;
        store.optimize(keep_versions).await?;

        let table = bucket.open().await?.raw().await?;
        let dataset = table.dataset().unwrap().get().await?;
        assert_eq!(
            config
                .iter()
                .all(|(key, value)| dataset.config().get(*key).map(String::as_str) == Some(*value)),
            kept,
            "config {config:?} after optimize(keep_versions: {keep_versions})"
        );
    }
    Ok(())
}
