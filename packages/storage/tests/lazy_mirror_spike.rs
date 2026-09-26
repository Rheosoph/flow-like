//! Feasibility spike for the lazy offline table mirror (todo/desktop-offline-writes.md,
//! "Mirroring"). A second local directory plays the cloud behind a counting, switchable
//! object store reached as `cloudsim://`. The device mirror is a manifest-only shallow
//! clone whose base path is the cloud URI. Base files are read through a read-through,
//! whole-file cache keyed by object path; only `data/`, `_deletions/` and `_indices/`
//! objects are cached because only those are immutable.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    ops::Range,
    path::{Path as FsPath, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow, ensure};
use arrow_array::{
    Array, FixedSizeListArray, Int64Array, RecordBatch, StringArray, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema};
use flow_like_storage::databases::vector::offline_replay::{
    self, ReplayMarker, ReplayMutation, ReplayOutcome,
};
use flow_like_types::{Value, json::json, reqwest::Url};
use futures::{StreamExt, TryStreamExt, stream::BoxStream};
use lance::{dataset::builder::DatasetBuilder, session::Session};
use lance_io::object_store::{
    ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
    uri_to_url,
};
use lance_table::{
    feature_flags::apply_feature_flags,
    io::{
        commit::{ManifestNamingScheme, commit_handler_from_url, write_manifest_file_to_path},
        deletion::relative_deletion_file_path,
        manifest::read_manifest_indexes,
    },
};
use lancedb::{
    Connection, Table,
    index::{
        Index,
        scalar::{BTreeIndexBuilder, FtsIndexBuilder, FullTextSearchQuery},
        vector::IvfFlatIndexBuilder,
    },
    query::{ExecutableQuery, QueryBase, Select},
    table::{CompactionOptions, Duration, OptimizeAction},
};
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
    local::LocalFileSystem, path::Path,
};

const CLOUD_URI: &str = "cloudsim://cloud/records.lance";
const TABLE: &str = "records";
const DIM: i32 = 4;
const MISS: &str = "is not cached on this device";

#[derive(Debug)]
struct MirrorCacheMiss {
    path: String,
    cause: String,
}

impl fmt::Display for MirrorCacheMiss {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "'{}' {MISS} and the cloud is unreachable: {}",
            self.path, self.cause
        )
    }
}

impl std::error::Error for MirrorCacheMiss {}

fn read_only(operation: &str, path: &Path) -> object_store::Error {
    object_store::Error::NotSupported {
        source: format!("{operation} of '{path}' reached a read-only cloud base").into(),
    }
}

fn record_write(log: &Mutex<Vec<String>>, operation: &str, path: &Path) -> object_store::Error {
    log.lock().unwrap().push(format!("{operation} {path}"));
    read_only(operation, path)
}

fn rejecting_deletes(
    log: Arc<Mutex<Vec<String>>>,
    locations: BoxStream<'static, object_store::Result<Path>>,
) -> BoxStream<'static, object_store::Result<Path>> {
    locations
        .map(move |location| Err(record_write(&log, "delete", &location?)))
        .boxed()
}

#[derive(Debug)]
struct CloudStore {
    inner: LocalFileSystem,
    offline: AtomicBool,
    reads: Mutex<BTreeMap<String, usize>>,
    lists: Mutex<Vec<String>>,
    writes: Arc<Mutex<Vec<String>>>,
}

impl CloudStore {
    fn reachable(&self) -> object_store::Result<()> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(object_store::Error::Generic {
                store: "cloudsim",
                source: "connection refused: the hub is unreachable".into(),
            });
        }
        Ok(())
    }

    fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    fn reads(&self) -> BTreeMap<String, usize> {
        self.reads.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }

    fn record_list(&self, prefix: Option<&Path>) {
        self.lists
            .lock()
            .unwrap()
            .push(prefix.map(ToString::to_string).unwrap_or_default());
    }
}

impl fmt::Display for CloudStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cloudsim")
    }
}

#[async_trait::async_trait]
impl ObjectStore for CloudStore {
    async fn put_opts(
        &self,
        location: &Path,
        _: PutPayload,
        _: PutOptions,
    ) -> object_store::Result<PutResult> {
        Err(record_write(&self.writes, "put", location))
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        _: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        Err(record_write(&self.writes, "multipart put", location))
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.reachable()?;
        *self
            .reads
            .lock()
            .unwrap()
            .entry(location.to_string())
            .or_default() += 1;
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        rejecting_deletes(self.writes.clone(), locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        if let Err(error) = self.reachable() {
            return futures::stream::once(async move { Err(error) }).boxed();
        }
        self.record_list(prefix);
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.reachable()?;
        self.record_list(prefix);
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, _: &Path, to: &Path, _: CopyOptions) -> object_store::Result<()> {
        Err(record_write(&self.writes, "copy", to))
    }

    async fn rename_opts(&self, _: &Path, to: &Path, _: RenameOptions) -> object_store::Result<()> {
        Err(record_write(&self.writes, "rename", to))
    }
}

fn immutable(path: &Path) -> bool {
    let parts = path.parts().collect::<Vec<_>>();
    parts.windows(2).any(|pair| {
        pair[0].as_ref().ends_with(".lance")
            && matches!(pair[1].as_ref(), "data" | "_deletions" | "_indices")
    })
}

fn unavailable(path: &Path, error: object_store::Error) -> object_store::Error {
    if matches!(error, object_store::Error::NotFound { .. }) {
        return error;
    }
    object_store::Error::Generic {
        store: "lazy-mirror",
        source: Box::new(MirrorCacheMiss {
            path: path.to_string(),
            cause: error.to_string(),
        }),
    }
}

#[derive(Debug)]
struct MirrorStore {
    cloud: Arc<CloudStore>,
    cache: LocalFileSystem,
    downloads: Mutex<BTreeMap<String, usize>>,
    rejected: Arc<Mutex<Vec<String>>>,
    locks: Mutex<HashMap<String, Arc<futures::lock::Mutex<()>>>>,
}

impl MirrorStore {
    async fn ensure_cached(&self, path: &Path) -> object_store::Result<()> {
        let lock = self
            .locks
            .lock()
            .unwrap()
            .entry(path.to_string())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        match self.cache.head(path).await {
            Ok(_) => return Ok(()),
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }
        let bytes = match self.cloud.get(path).await {
            Ok(object) => object.bytes().await,
            Err(error) => Err(error),
        }
        .map_err(|error| unavailable(path, error))?;
        self.cache.put(path, bytes.into()).await?;
        *self
            .downloads
            .lock()
            .unwrap()
            .entry(path.to_string())
            .or_default() += 1;
        Ok(())
    }

    fn downloads(&self) -> BTreeMap<String, usize> {
        self.downloads.lock().unwrap().clone()
    }

    fn rejected(&self) -> Vec<String> {
        self.rejected.lock().unwrap().clone()
    }
}

impl fmt::Display for MirrorStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lazy mirror of cloudsim")
    }
}

#[async_trait::async_trait]
impl ObjectStore for MirrorStore {
    async fn put_opts(
        &self,
        location: &Path,
        _: PutPayload,
        _: PutOptions,
    ) -> object_store::Result<PutResult> {
        Err(record_write(&self.rejected, "put", location))
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        _: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        Err(record_write(&self.rejected, "multipart put", location))
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        if !immutable(location) {
            return self.cloud.get_opts(location, options).await;
        }
        self.ensure_cached(location).await?;
        self.cache.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        rejecting_deletes(self.rejected.clone(), locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.cloud.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.cloud.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, _: &Path, to: &Path, _: CopyOptions) -> object_store::Result<()> {
        Err(record_write(&self.rejected, "copy", to))
    }

    async fn rename_opts(&self, _: &Path, to: &Path, _: RenameOptions) -> object_store::Result<()> {
        Err(record_write(&self.rejected, "rename", to))
    }
}

#[derive(Debug)]
struct MirrorProvider(Arc<MirrorStore>);

#[async_trait::async_trait]
impl ObjectStoreProvider for MirrorProvider {
    async fn new_store(&self, url: Url, params: &ObjectStoreParams) -> lance::Result<LanceStore> {
        Ok(LanceStore::new(
            self.0.clone(),
            url,
            params.block_size,
            None,
            false,
            false,
            8,
            0,
            None,
        ))
    }
}

/// Lance opens v3 vector index files through the dataset's primary store with the
/// base-derived path, so the device store sends the cloud base's key prefix to the mirror.
#[derive(Debug)]
struct DeviceStore {
    local: Arc<LocalFileSystem>,
    cloud_root: Path,
    mirror: Arc<MirrorStore>,
}

impl DeviceStore {
    fn is_cloud(&self, path: &Path) -> bool {
        path.prefix_matches(&self.cloud_root)
    }
}

impl fmt::Display for DeviceStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("device mirror store")
    }
}

#[async_trait::async_trait]
impl ObjectStore for DeviceStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        if self.is_cloud(location) {
            return Err(record_write(&self.mirror.rejected, "put", location));
        }
        self.local.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        if self.is_cloud(location) {
            return Err(record_write(
                &self.mirror.rejected,
                "multipart put",
                location,
            ));
        }
        self.local.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        if self.is_cloud(location) {
            return self.mirror.get_opts(location, options).await;
        }
        self.local.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        let local = self.local.clone();
        let cloud_root = self.cloud_root.clone();
        let rejected = self.mirror.rejected.clone();
        locations
            .then(move |location| {
                let local = local.clone();
                let cloud_root = cloud_root.clone();
                let rejected = rejected.clone();
                async move {
                    let location = location?;
                    if location.prefix_matches(&cloud_root) {
                        return Err(record_write(&rejected, "delete", &location));
                    }
                    local.delete(&location).await?;
                    Ok(location)
                }
            })
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        match prefix {
            Some(prefix) if self.is_cloud(prefix) => self.mirror.list(Some(prefix)),
            _ => self.local.list(prefix),
        }
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        match prefix {
            Some(prefix) if self.is_cloud(prefix) => {
                self.mirror.list_with_delimiter(Some(prefix)).await
            }
            _ => self.local.list_with_delimiter(prefix).await,
        }
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        if self.is_cloud(from) || self.is_cloud(to) {
            return Err(record_write(&self.mirror.rejected, "copy", to));
        }
        self.local.copy_opts(from, to, options).await
    }

    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        if self.is_cloud(from) || self.is_cloud(to) {
            return Err(record_write(&self.mirror.rejected, "rename", to));
        }
        self.local.rename_opts(from, to, options).await
    }
}

#[derive(Debug)]
struct DeviceProvider {
    mirror: Arc<MirrorStore>,
    cloud_root: Path,
}

#[async_trait::async_trait]
impl ObjectStoreProvider for DeviceProvider {
    async fn new_store(&self, url: Url, params: &ObjectStoreParams) -> lance::Result<LanceStore> {
        let store = DeviceStore {
            local: Arc::new(LocalFileSystem::new()),
            cloud_root: self.cloud_root.clone(),
            mirror: self.mirror.clone(),
        };
        Ok(LanceStore::new(
            Arc::new(store),
            url,
            params.block_size,
            None,
            false,
            false,
            16,
            0,
            None,
        ))
    }

    fn extract_path(&self, url: &Url) -> lance::Result<Path> {
        let file = Url::parse(&url.as_str().replacen("file-object-store:", "file:", 1))
            .map_err(|error| lance::Error::invalid_input(error.to_string()))?
            .to_file_path()
            .map_err(|_| lance::Error::invalid_input("invalid device table URI"))?;
        Path::from_absolute_path(&file)
            .map_err(|error| lance::Error::invalid_input(error.to_string()))
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Snapshot {
    table: Table,
    base_id: u32,
    cloud_version: u64,
}

struct Fixture {
    _scratch: Scratch,
    cloud_dir: PathBuf,
    mirror_dir: PathBuf,
    hub: Connection,
    cloud: Arc<CloudStore>,
    mirror: Arc<MirrorStore>,
    routed: bool,
    session: Arc<Session>,
    device: Connection,
}

fn device_session(mirror: &Arc<MirrorStore>, routed: bool) -> Arc<Session> {
    let registry = Arc::new(ObjectStoreRegistry::default());
    registry.insert("cloudsim", Arc::new(MirrorProvider(mirror.clone())));
    if routed {
        registry.insert(
            "file-object-store",
            Arc::new(DeviceProvider {
                mirror: mirror.clone(),
                cloud_root: Path::from("records.lance"),
            }),
        );
    }
    Arc::new(Session::new(16 * 1024 * 1024, 16 * 1024 * 1024, registry))
}

fn database_uri(directory: &FsPath) -> Result<String> {
    let url = uri_to_url(
        directory
            .to_str()
            .ok_or_else(|| anyhow!("mirror path is not UTF-8"))?,
    )?;
    Ok(url
        .as_str()
        .trim_end_matches('/')
        .replacen("file:", "file-object-store:", 1))
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("text", DataType::Utf8, true),
        Field::new(
            "vec",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
            true,
        ),
    ]))
}

fn text(id: i64) -> String {
    if id % 7 == 0 {
        format!("row {id} needle")
    } else {
        format!("row {id} hay")
    }
}

fn vector(id: i64) -> Vec<f32> {
    vec![id as f32, (id % 10) as f32, 1.0, 0.5]
}

fn batch(ids: Range<i64>) -> Result<RecordBatch> {
    let ids = ids.collect::<Vec<_>>();
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        ids.iter()
            .map(|id| Some(vector(*id).into_iter().map(Some).collect::<Vec<_>>())),
        DIM,
    );
    Ok(RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(StringArray::from_iter_values(
                ids.iter().map(|id| text(*id)),
            )),
            Arc::new(vectors),
        ],
    )?)
}

fn row(id: i64, text: &str) -> Value {
    json!({"id": id, "text": text, "vec": vector(id)})
}

fn files_under(directory: &FsPath) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                stack.push(entry.path());
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(directory)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    Ok(files)
}

fn under<'a>(paths: impl IntoIterator<Item = &'a String>, segment: &str) -> BTreeSet<String> {
    paths
        .into_iter()
        .filter(|path| path.contains(&format!("/{segment}/")))
        .cloned()
        .collect()
}

fn ids_of(batches: &[RecordBatch]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for batch in batches {
        let column = batch
            .column_by_name("id")
            .ok_or_else(|| anyhow!("result has no id column"))?
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| anyhow!("id column is not Int64"))?;
        ids.extend((0..column.len()).map(|index| column.value(index)));
    }
    ids.sort_unstable();
    Ok(ids)
}

async fn scan_ids(table: &Table, filter: Option<&str>) -> Result<Vec<i64>> {
    let mut query = table.query().select(Select::Columns(vec!["id".into()]));
    if let Some(filter) = filter {
        query = query.only_if(filter);
    }
    ids_of(&query.execute().await?.try_collect::<Vec<_>>().await?)
}

async fn text_of(table: &Table, id: i64) -> Result<String> {
    let batches = table
        .query()
        .only_if(format!("id = {id}"))
        .select(Select::Columns(vec!["text".into()]))
        .execute()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let column = batches
        .first()
        .and_then(|batch| batch.column_by_name("text"))
        .ok_or_else(|| anyhow!("row {id} is missing"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow!("text column is not Utf8"))?
        .clone();
    Ok(column.value(0).to_string())
}

async fn fts_ids(table: &Table, term: &str) -> Result<Vec<i64>> {
    let batches = table
        .query()
        .full_text_search(FullTextSearchQuery::new(term.into()))
        .select(Select::Columns(vec!["id".into()]))
        .limit(1000)
        .execute()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    ids_of(&batches)
}

async fn nearest_ids(table: &Table, id: i64, limit: usize) -> Result<Vec<i64>> {
    let batches = table
        .query()
        .nearest_to(vector(id))?
        .limit(limit)
        .select(Select::Columns(vec!["id".into()]))
        .execute()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    ids_of(&batches)
}

fn is_cache_miss(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<MirrorCacheMiss>().is_some())
}

async fn apply(table: &Table, mutation: ReplayMutation) -> Result<u64> {
    let (expected_version, fingerprint) = offline_replay::revision(table).await?;
    let marker = ReplayMarker {
        operation_id: flow_like_types::create_id(),
        digest: "lazy-mirror-spike".into(),
        expected_version,
        expected_fingerprint: Some(fingerprint),
    };
    match offline_replay::replay(table, &marker, mutation).await? {
        ReplayOutcome::Applied { version, .. } => {
            table.checkout_latest().await?;
            Ok(version)
        }
        other => Err(anyhow!("local mutation was not applied: {other:?}")),
    }
}

fn queued_changes() -> Vec<ReplayMutation> {
    vec![
        ReplayMutation::Insert {
            items: vec![row(1000, "row 1000 needle fresh")],
        },
        ReplayMutation::Upsert {
            items: vec![row(150, "changed")],
            id_field: "id".into(),
        },
        ReplayMutation::Delete {
            filter: "`id` = 20".into(),
        },
    ]
}

impl Fixture {
    async fn new(routed: bool) -> Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "flow-like-lazy-mirror-{}",
            flow_like_types::create_id()
        ));
        let cloud_dir = root.join("cloud");
        let cache_dir = root.join("cache");
        let mirror_dir = root.join("mirror");
        for directory in [&cloud_dir, &cache_dir, &mirror_dir] {
            std::fs::create_dir_all(directory)?;
        }
        let hub = lancedb::connect(
            cloud_dir
                .to_str()
                .ok_or_else(|| anyhow!("cloud path is not UTF-8"))?,
        )
        .execute()
        .await?;
        let cloud = Arc::new(CloudStore {
            inner: LocalFileSystem::new_with_prefix(&cloud_dir)?,
            offline: AtomicBool::new(false),
            reads: Default::default(),
            lists: Default::default(),
            writes: Default::default(),
        });
        let mirror = Arc::new(MirrorStore {
            cloud: cloud.clone(),
            cache: LocalFileSystem::new_with_prefix(&cache_dir)?,
            downloads: Default::default(),
            rejected: Default::default(),
            locks: Default::default(),
        });
        let session = device_session(&mirror, routed);
        let device = lancedb::connect(&database_uri(&mirror_dir)?)
            .session(session.clone())
            .execute()
            .await?;
        Ok(Self {
            _scratch: Scratch(root),
            cloud_dir,
            mirror_dir,
            hub,
            cloud,
            mirror,
            routed,
            session,
            device,
        })
    }

    async fn seed(&self, fragments: i64) -> Result<Table> {
        let table = self
            .hub
            .create_table(TABLE, batch(0..100)?)
            .execute()
            .await?;
        for fragment in 1..fragments {
            table
                .add(batch(fragment * 100..(fragment + 1) * 100)?)
                .execute()
                .await?;
        }
        Ok(table)
    }

    async fn cloud_data_files(&self) -> Result<Vec<String>> {
        let table = self.hub.open_table(TABLE).execute().await?;
        let dataset = table
            .dataset()
            .ok_or_else(|| anyhow!("cloud table is not native"))?
            .get()
            .await?;
        Ok(dataset
            .manifest()
            .fragments
            .iter()
            .flat_map(|fragment| fragment.files.iter())
            .map(|file| format!("records.lance/data/{}", file.path))
            .collect())
    }

    async fn snapshot(&self, name: &str) -> Result<Snapshot> {
        let cloud = DatasetBuilder::from_uri(CLOUD_URI)
            .with_session(self.session.clone())
            .load()
            .await?;
        let store = cloud.object_store(None).await?;
        let indices =
            read_manifest_indexes(&store, cloud.manifest_location(), cloud.manifest()).await?;
        let base_id = cloud
            .manifest()
            .base_paths
            .keys()
            .max()
            .map_or(0, |id| id + 1);
        let mut manifest =
            cloud
                .manifest()
                .shallow_clone(None, CLOUD_URI.into(), base_id, None, String::new());
        manifest.transaction_file = None;
        apply_feature_flags(&mut manifest, false, false)?;
        let indices = indices
            .into_iter()
            .map(|mut index| {
                index.base_id.get_or_insert(base_id);
                index
            })
            .collect::<Vec<_>>();
        let uri = format!("{}/{name}.lance", database_uri(&self.mirror_dir)?);
        let (local, base) = LanceStore::from_uri_and_params(
            self.session.store_registry(),
            &uri,
            &ObjectStoreParams::default(),
        )
        .await?;
        commit_handler_from_url(&uri, &None)
            .await?
            .commit(
                &mut manifest,
                (!indices.is_empty()).then_some(indices),
                &base,
                local.as_ref(),
                write_manifest_file_to_path,
                ManifestNamingScheme::V2,
                None,
            )
            .await
            .map_err(|error| anyhow!("lazy snapshot commit failed: {error:?}"))?;
        Ok(Snapshot {
            table: self.device.open_table(name).execute().await?,
            base_id,
            cloud_version: cloud.manifest().version,
        })
    }

    async fn fresh_device(&self) -> Result<Connection> {
        Ok(lancedb::connect(&database_uri(&self.mirror_dir)?)
            .session(device_session(&self.mirror, self.routed))
            .execute()
            .await?)
    }

    async fn prefetch(&self, snapshot: &Snapshot) -> Result<BTreeSet<String>> {
        let dataset = snapshot
            .table
            .dataset()
            .ok_or_else(|| anyhow!("mirror table is not native"))?
            .get()
            .await?;
        let manifest = dataset.manifest();
        let base = manifest
            .base_paths
            .get(&snapshot.base_id)
            .ok_or_else(|| anyhow!("mirror has no cloud base"))?;
        let root = Url::parse(&base.path)?.path().trim_matches('/').to_string();
        let mut paths = BTreeSet::new();
        for fragment in manifest.fragments.iter() {
            for file in &fragment.files {
                if file.base_id == Some(snapshot.base_id) {
                    paths.insert(Path::parse(format!("{root}/data/{}", file.path))?);
                }
            }
            if let Some(deletion) = &fragment.deletion_file
                && deletion.base_id == Some(snapshot.base_id)
            {
                paths.insert(Path::parse(format!(
                    "{root}/{}",
                    relative_deletion_file_path(fragment.id, deletion)
                ))?);
            }
        }
        let local = dataset.object_store(None).await?;
        for index in read_manifest_indexes(&local, dataset.manifest_location(), manifest).await? {
            if index.base_id != Some(snapshot.base_id) {
                continue;
            }
            let directory = Path::parse(format!("{root}/_indices/{}", index.uuid))?;
            match &index.files {
                Some(files) => {
                    for file in files {
                        paths.insert(Path::parse(format!("{directory}/{}", file.path))?);
                    }
                }
                None => {
                    let listed = self
                        .mirror
                        .list(Some(&directory))
                        .try_collect::<Vec<_>>()
                        .await?;
                    paths.extend(listed.into_iter().map(|meta| meta.location));
                }
            }
        }
        for path in &paths {
            self.mirror.ensure_cached(path).await?;
        }
        Ok(paths.iter().map(ToString::to_string).collect())
    }
}

fn expected_after_changes(base: Range<i64>) -> Vec<i64> {
    let mut ids = base.filter(|id| *id != 20).collect::<Vec<_>>();
    ids.push(1000);
    ids.sort_unstable();
    ids
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lazy_reads_fetch_only_touched_files_and_uncached_reads_fail_offline() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let hub_table = fixture.seed(3).await?;
    hub_table
        .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
        .execute()
        .await?;
    let data_files = fixture.cloud_data_files().await?;
    ensure!(data_files.len() == 3, "expected one data file per append");

    let snapshot = fixture.snapshot("lazy").await?;
    ensure!(
        fixture.mirror.downloads().is_empty(),
        "creating the snapshot must not download table files"
    );
    let metadata_reads = fixture.cloud.reads();
    ensure!(
        metadata_reads
            .keys()
            .all(|path| path.contains("/_versions/")),
        "snapshot creation read more than manifests: {metadata_reads:?}"
    );
    let local_files = files_under(&fixture.mirror_dir.join("lazy.lance"))?;
    ensure!(
        local_files
            .iter()
            .all(|file| file.starts_with("_versions/")),
        "the device keeps only version metadata: {local_files:?}"
    );

    ensure!(scan_ids(&snapshot.table, Some("id = 150")).await? == vec![150]);
    let downloads = fixture.mirror.downloads();
    let downloaded_data = under(downloads.keys(), "data");
    let downloaded_indices = under(downloads.keys(), "_indices");
    eprintln!(
        "point lookup downloaded {} data file(s) and {} index file(s) of {} data files",
        downloaded_data.len(),
        downloaded_indices.len(),
        data_files.len()
    );
    ensure!(
        downloaded_data == BTreeSet::from([data_files[1].clone()]),
        "an indexed point lookup must fetch only the fragment holding the row: {downloaded_data:?}"
    );
    ensure!(
        !downloaded_indices.is_empty(),
        "the cloud BTree index was used"
    );
    ensure!(scan_ids(&snapshot.table, Some("id = 20")).await? == vec![20]);
    ensure!(
        under(fixture.mirror.downloads().keys(), "data")
            == BTreeSet::from([data_files[0].clone(), data_files[1].clone()])
    );

    fixture.cloud.set_offline(true);
    ensure!(scan_ids(&snapshot.table, Some("id = 150")).await? == vec![150]);
    let restarted = fixture
        .fresh_device()
        .await?
        .open_table("lazy")
        .execute()
        .await?;
    ensure!(scan_ids(&restarted, Some("id = 150")).await? == vec![150]);
    ensure!(scan_ids(&restarted, Some("id = 20")).await? == vec![20]);
    ensure!(restarted.count_rows(None).await? == 300);

    let miss = scan_ids(&restarted, Some("id = 250"))
        .await
        .expect_err("a row in an uncached fragment must not be readable offline");
    eprintln!(
        "uncached point lookup: typed={} error={miss:#}",
        is_cache_miss(&miss)
    );
    ensure!(format!("{miss:#}").contains(MISS));
    ensure!(format!("{miss:#}").contains(&data_files[2]));

    let mut stream = restarted
        .query()
        .select(Select::Columns(vec!["id".into()]))
        .execute()
        .await?;
    let mut partial_rows = 0;
    let mut failure = None;
    while let Some(next) = stream.next().await {
        match next {
            Ok(batch) => partial_rows += batch.num_rows(),
            Err(error) => {
                failure = Some(anyhow::Error::from(error));
                break;
            }
        }
    }
    let failure = failure.ok_or_else(|| anyhow!("an offline full scan returned all rows"))?;
    eprintln!(
        "offline full scan: {partial_rows} of 300 rows streamed before the error, typed={}",
        is_cache_miss(&failure)
    );
    ensure!(format!("{failure:#}").contains(MISS));
    ensure!(partial_rows < 300);
    ensure!(
        scan_ids(&restarted, None).await.is_err(),
        "a collected offline scan must fail as a whole"
    );
    ensure!(fixture.cloud.writes().is_empty() && fixture.mirror.rejected().is_empty());
    Ok(())
}

async fn write_and_refresh(key_index: bool) -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let hub_table = fixture.seed(3).await?;
    if key_index {
        hub_table
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
    }
    let data_files = fixture.cloud_data_files().await?;
    let cloud_before = files_under(&fixture.cloud_dir)?;

    let first = fixture.snapshot("first").await?;
    ensure!(scan_ids(&first.table, Some("`id` IN (150, 1000)")).await? == vec![150]);
    let after_key_check = fixture.mirror.downloads();
    for change in queued_changes() {
        apply(&first.table, change).await?;
    }
    let after_writes = fixture.mirror.downloads();
    eprintln!(
        "key_index={key_index}: freeze key check downloaded data {:?}; the three writes downloaded data {:?}",
        under(after_key_check.keys(), "data"),
        under(after_writes.keys(), "data"),
    );
    if key_index {
        ensure!(
            under(after_writes.keys(), "data") == BTreeSet::from([data_files[1].clone()]),
            "indexed key lookups, the upsert and the key delete fetch only the upserted row's fragment"
        );
    } else {
        ensure!(
            under(after_key_check.keys(), "data") == data_files.iter().cloned().collect(),
            "an unindexed key check scans the key column of every fragment"
        );
    }

    ensure!(
        fixture.cloud.writes().is_empty(),
        "the cloud store saw a write"
    );
    ensure!(fixture.mirror.rejected().is_empty(), "the base saw a write");
    ensure!(
        files_under(&fixture.cloud_dir)? == cloud_before,
        "local writes changed cloud files"
    );
    let local_files = files_under(&fixture.mirror_dir.join("first.lance"))?;
    ensure!(local_files.iter().any(|file| file.starts_with("data/")));
    ensure!(
        local_files
            .iter()
            .any(|file| file.starts_with("_deletions/"))
    );
    ensure!(scan_ids(&first.table, None).await? == expected_after_changes(0..300));
    ensure!(text_of(&first.table, 150).await? == "changed");

    hub_table.add(batch(300..400)?).execute().await?;
    let before_refresh = fixture.mirror.downloads();
    let second = fixture.snapshot("second").await?;
    ensure!(second.cloud_version > first.cloud_version);
    ensure!(
        fixture.mirror.downloads() == before_refresh,
        "a refresh must fetch metadata only"
    );
    for change in queued_changes() {
        apply(&second.table, change).await?;
    }
    ensure!(scan_ids(&second.table, None).await? == expected_after_changes(0..400));
    ensure!(text_of(&second.table, 150).await? == "changed");
    let downloads = fixture.mirror.downloads();
    ensure!(
        downloads.values().all(|count| *count == 1),
        "an immutable file was downloaded twice: {downloads:?}"
    );
    let new_files = fixture
        .cloud_data_files()
        .await?
        .into_iter()
        .filter(|file| !data_files.contains(file))
        .collect::<Vec<_>>();
    ensure!(new_files.len() == 1 && downloads.contains_key(&new_files[0]));
    ensure!(fixture.cloud.writes().is_empty() && fixture.mirror.rejected().is_empty());
    ensure!(files_under(&fixture.cloud_dir)?.is_superset(&cloud_before));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_writes_stay_local_and_refresh_reuses_cached_files() -> Result<()> {
    write_and_refresh(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unindexed_key_checks_download_every_base_file() -> Result<()> {
    write_and_refresh(false).await
}

async fn indexed_cloud(fixture: &Fixture) -> Result<Table> {
    let hub_table = fixture.seed(3).await?;
    hub_table.delete("id = 5").await?;
    hub_table
        .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
        .execute()
        .await?;
    hub_table
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    hub_table
        .create_index(
            &["vec"],
            Index::IvfFlat(IvfFlatIndexBuilder::default().num_partitions(2)),
        )
        .execute()
        .await?;
    Ok(hub_table)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrouted_primary_store_breaks_cross_store_vector_indices() -> Result<()> {
    let fixture = Fixture::new(false).await?;
    let hub_table = indexed_cloud(&fixture).await?;
    let snapshot = fixture.snapshot("unrouted").await?;
    fixture.prefetch(&snapshot).await?;
    ensure!(scan_ids(&snapshot.table, Some("id = 150")).await? == vec![150]);
    ensure!(fts_ids(&snapshot.table, "needle").await? == fts_ids(&hub_table, "needle").await?);
    let error = nearest_ids(&snapshot.table, 150, 3)
        .await
        .expect_err("Lance reads v3 vector index files through the primary store");
    eprintln!("unrouted vector search: {error:#}");
    ensure!(
        format!("{error:#}").contains("index.idx") && format!("{error:#}").contains("not found")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetch_makes_indexed_reads_and_local_writes_work_offline() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let hub_table = indexed_cloud(&fixture).await?;

    let all = scan_ids(&hub_table, None).await?;
    let needles = fts_ids(&hub_table, "needle").await?;
    let nearest = nearest_ids(&hub_table, 150, 3).await?;
    let small = hub_table.count_rows(Some("id < 50".into())).await?;
    ensure!(all.len() == 299 && !needles.is_empty() && nearest.contains(&150));

    let snapshot = fixture.snapshot("prefetched").await?;
    let dataset = snapshot
        .table
        .dataset()
        .ok_or_else(|| anyhow!("mirror table is not native"))?
        .get()
        .await?;
    let local = dataset.object_store(None).await?;
    let local_indices =
        read_manifest_indexes(&local, dataset.manifest_location(), dataset.manifest()).await?;
    ensure!(
        local_indices.len() == 3
            && local_indices
                .iter()
                .all(|index| index.base_id == Some(snapshot.base_id)),
        "the mirror references the cloud's three indices"
    );

    let prefetched = fixture.prefetch(&snapshot).await?;
    ensure!(under(&prefetched, "data").len() == 3);
    ensure!(under(&prefetched, "_deletions").len() == 1);
    ensure!(under(&prefetched, "_indices").len() >= 3);
    eprintln!(
        "prefetch fetched {} files: {} data, {} deletion, {} index",
        prefetched.len(),
        under(&prefetched, "data").len(),
        under(&prefetched, "_deletions").len(),
        under(&prefetched, "_indices").len()
    );

    fixture.cloud.set_offline(true);
    let reads_offline = fixture.cloud.reads();
    let lists_offline = fixture.cloud.lists.lock().unwrap().len();
    for table in [
        snapshot.table.clone(),
        fixture
            .fresh_device()
            .await?
            .open_table("prefetched")
            .execute()
            .await?,
    ] {
        ensure!(scan_ids(&table, None).await? == all);
        ensure!(scan_ids(&table, Some("id = 150")).await? == vec![150]);
        ensure!(fts_ids(&table, "needle").await? == needles);
        ensure!(nearest_ids(&table, 150, 3).await? == nearest);
        ensure!(table.count_rows(Some("id < 50".into())).await? == small);
    }

    apply(
        &snapshot.table,
        ReplayMutation::Insert {
            items: vec![row(1000, "row 1000 needle fresh")],
        },
    )
    .await?;
    apply(
        &snapshot.table,
        ReplayMutation::Delete {
            filter: "`id` = 7".into(),
        },
    )
    .await?;
    let mut expected_needles = needles
        .iter()
        .copied()
        .filter(|id| *id != 7)
        .collect::<Vec<_>>();
    expected_needles.push(1000);
    expected_needles.sort_unstable();
    ensure!(fts_ids(&snapshot.table, "needle").await? == expected_needles);
    ensure!(nearest_ids(&snapshot.table, 1000, 1).await? == vec![1000]);
    ensure!(scan_ids(&snapshot.table, Some("id = 7")).await?.is_empty());
    ensure!(scan_ids(&snapshot.table, Some("id = 5")).await?.is_empty());
    ensure!(
        fixture.cloud.reads() == reads_offline
            && fixture.cloud.lists.lock().unwrap().len() == lists_offline,
        "offline reads after prefetch reached the cloud"
    );
    ensure!(fixture.cloud.writes().is_empty() && fixture.mirror.rejected().is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn synced_rows_move_to_new_cloud_files_and_refresh_prefetch_fetches_only_the_delta()
-> Result<()> {
    let fixture = Fixture::new(true).await?;
    let hub_table = fixture.seed(3).await?;
    hub_table
        .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
        .execute()
        .await?;
    let first = fixture.snapshot("first").await?;
    let initial = fixture.prefetch(&first).await?;

    fixture.cloud.set_offline(true);
    let change = ReplayMutation::Insert {
        items: vec![row(1000, "row 1000 needle fresh")],
    };
    apply(&first.table, change.clone()).await?;
    ensure!(scan_ids(&first.table, Some("id = 1000")).await? == vec![1000]);
    fixture.cloud.set_offline(false);
    apply(&hub_table, change).await?;

    let second = fixture.snapshot("second").await?;
    let synced_file = fixture
        .cloud_data_files()
        .await?
        .into_iter()
        .find(|file| !initial.contains(file))
        .ok_or_else(|| anyhow!("the cloud replay wrote no new data file"))?;
    fixture.cloud.set_offline(true);
    for filter in ["id = 1000", "id = 150"] {
        let miss = scan_ids(&second.table, Some(filter))
            .await
            .expect_err("every filter scans the new unindexed fragment, which was never fetched");
        ensure!(format!("{miss:#}").contains(MISS) && format!("{miss:#}").contains(&synced_file));
    }

    fixture.cloud.set_offline(false);
    let before = fixture.mirror.downloads();
    let current = fixture.prefetch(&second).await?;
    let fetched = fixture
        .mirror
        .downloads()
        .into_iter()
        .filter(|(path, _)| !before.contains_key(path))
        .map(|(path, _)| path)
        .collect::<BTreeSet<_>>();
    let unseen = current
        .difference(&initial)
        .cloned()
        .collect::<BTreeSet<_>>();
    eprintln!("refresh prefetch fetched only {fetched:?}");
    ensure!(fetched == unseen && under(&fetched, "data").len() == 1);
    ensure!(fixture.mirror.downloads().values().all(|count| *count == 1));

    fixture.cloud.set_offline(true);
    ensure!(scan_ids(&second.table, Some("id = 1000")).await? == vec![1000]);
    let mut expected = (0..300).collect::<Vec<_>>();
    expected.push(1000);
    ensure!(scan_ids(&second.table, None).await? == expected);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_compaction_and_prune_strand_uncached_files_until_refresh() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let hub_table = fixture.seed(3).await?;
    hub_table
        .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
        .execute()
        .await?;
    let stale = fixture.snapshot("stale").await?;
    ensure!(scan_ids(&stale.table, Some("id = 150")).await? == vec![150]);

    hub_table
        .optimize(OptimizeAction::Compact {
            options: CompactionOptions::default(),
            remap_options: None,
        })
        .await?;
    hub_table
        .optimize(OptimizeAction::Prune {
            older_than: Some(Duration::zero()),
            delete_unverified: Some(true),
            error_if_tagged_old_versions: Some(false),
        })
        .await?;
    ensure!(fixture.cloud_data_files().await?.len() == 1);

    ensure!(scan_ids(&stale.table, Some("id = 150")).await? == vec![150]);
    let gone = scan_ids(&stale.table, Some("id = 20"))
        .await
        .expect_err("a pruned, never cached base file cannot be read");
    eprintln!("stale snapshot after cloud prune: {gone:#}");
    ensure!(!format!("{gone:#}").contains(MISS) && format!("{gone:#}").contains("not found"));

    let fresh = fixture.snapshot("fresh").await?;
    ensure!(scan_ids(&fresh.table, Some("id = 20")).await? == vec![20]);
    ensure!(scan_ids(&fresh.table, None).await? == (0..300).collect::<Vec<_>>());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skip_branch_and_idle_cleanup_keep_the_cloud_base_untouched() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    fixture.seed(2).await?;
    let snapshot = fixture.snapshot("branchy").await?;
    let base_version = snapshot.table.version().await?;
    ensure!(base_version == snapshot.cloud_version);
    apply(
        &snapshot.table,
        ReplayMutation::Insert {
            items: vec![row(1000, "row 1000 needle fresh")],
        },
    )
    .await?;
    snapshot
        .table
        .create_branch("skip_spike", ("main", base_version))
        .await?;
    let branch = fixture
        .device
        .open_table("branchy")
        .branch("skip_spike")
        .execute()
        .await?;
    ensure!(scan_ids(&branch, None).await? == (0..200).collect::<Vec<_>>());
    offline_replay::compact_idle_local(&snapshot.table).await?;
    let mut expected = (0..200).collect::<Vec<_>>();
    expected.push(1000);
    ensure!(scan_ids(&snapshot.table, None).await? == expected);
    ensure!(scan_ids(&branch, None).await? == (0..200).collect::<Vec<_>>());
    ensure!(fixture.cloud.writes().is_empty() && fixture.mirror.rejected().is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stock_shallow_clone_cannot_target_another_store() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    fixture.seed(1).await?;
    let result = fixture
        .device
        .clone_table("copy", CLOUD_URI)
        .execute()
        .await;
    let rejected = fixture.mirror.rejected();
    eprintln!(
        "stock clone result: {:?}; rejected writes: {rejected:?}",
        result.as_ref().err()
    );
    ensure!(
        result.is_err(),
        "a cross-store shallow clone unexpectedly succeeded"
    );
    ensure!(
        rejected.iter().any(|write| write.contains("copy.lance")),
        "the clone's target files were routed through the source store"
    );
    ensure!(!fixture.mirror_dir.join("copy.lance").exists());
    Ok(())
}
