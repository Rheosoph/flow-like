//! `MirrorMode::Lazy`: the cloud is a directory behind a counting, switchable store that
//! `TestHost::remote_table` and `remote_objects` reach as `cloudsim://`.

use super::{
    files::Breaker,
    manager::{TestHost, replay_server, resource_key},
};
use crate::{
    BufferedTable, BufferingConfig,
    fs::unix_time,
    manager::{
        FastForward, LazyMirrorOptions, MirrorMode, RefreshOutcome, TableActivation, TableSetup,
        TableState, WriteManager, WriteManagerOptions,
    },
    outbox::QueueLanes,
    table::TableOverlay,
};
use anyhow::{Context, Result};
use flow_like_device_protocol::{
    DESKTOP_OFFLINE_LIMITS, OfflineExpected, OfflineMutation, OfflineReplayRequest,
    OfflineResource, StoragePurpose, format_limit,
};
use flow_like_storage::{
    arrow_array::{FixedSizeListArray, Int64Array, RecordBatch, StringArray, types::Float32Type},
    arrow_schema::{DataType, Field, Schema},
    databases::vector::{
        VectorStore,
        lancedb::{
            DatabaseSelector, LanceDBVectorStore, LogicalTableMutation, LogicalTableMutationAdapter,
        },
        offline_replay,
    },
    lance::session::Session,
    lance_io::object_store::{
        ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
    },
    lancedb::{
        self, Connection, Table,
        index::{
            Index,
            scalar::{BTreeIndexBuilder, BitmapIndexBuilder, FtsIndexBuilder},
            vector::{IvfFlatIndexBuilder, IvfPqIndexBuilder},
        },
        table::{CompactionOptions, Duration as LanceDuration, OptimizeAction},
    },
    object_store::{
        self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
        ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
        local::LocalFileSystem, path::Path as ObjectPath,
    },
};
use flow_like_types::reqwest::Url;
use futures_util::{StreamExt, stream::BoxStream};
use lance_table::io::{
    commit::{ManifestNamingScheme, commit_handler_from_url, write_manifest_file_to_path},
    manifest::read_manifest_indexes,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fmt,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MIB: u64 = 1024 * 1024;
const PROJECT_DB: &str = "cloudsim://bucket/apps/project/storage/db";
const E36: &str = "Table 'measurements' needs data that is not on this device yet. Reconnect to the hub, or turn on \"Download everything\" for this table while connected.";
const E37: &str = "Part of table 'measurements' was reorganized in the cloud after this device took its copy. The copy refreshes automatically once the table has no queued changes; try again then.";
const ALLOWANCE: &str = "Daily download limit for offline copies reached.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Get {
    Whole,
    Range,
    Head,
}

#[derive(Debug, Default)]
struct Faults {
    offline: bool,
    /// Whole GETs of keys containing one of these wait until it is removed.
    stalled: HashSet<String>,
}

/// The cloud bucket: switchable, counting and able to stall whole-file downloads.
#[derive(Debug)]
struct CloudStore {
    inner: LocalFileSystem,
    faults: Mutex<Faults>,
    gets: Mutex<Vec<(String, Get)>>,
}

impl CloudStore {
    fn faults(&self) -> MutexGuard<'_, Faults> {
        self.faults.lock().unwrap()
    }

    fn reachable(&self) -> object_store::Result<()> {
        if self.faults().offline {
            return Err(object_store::Error::Generic {
                store: "cloudsim",
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "connection refused",
                )),
            });
        }
        Ok(())
    }

    fn count(&self, key: &str, kind: Get) -> usize {
        self.gets
            .lock()
            .unwrap()
            .iter()
            .filter(|(get, got)| get == key && *got == kind)
            .count()
    }

    fn requests(&self) -> usize {
        self.gets.lock().unwrap().len()
    }

    fn stall(&self, pattern: &str) {
        self.faults().stalled.insert(pattern.into());
    }

    fn release(&self) {
        self.faults().stalled.clear();
    }

    fn stalled(&self, key: &str) -> bool {
        self.faults()
            .stalled
            .iter()
            .any(|pattern| key.contains(pattern))
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
        location: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.reachable()?;
        self.inner.put_opts(location, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        location: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.reachable()?;
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.reachable()?;
        let kind = match (options.head, &options.range) {
            (true, _) => Get::Head,
            (false, Some(_)) => Get::Range,
            (false, None) => Get::Whole,
        };
        self.gets.lock().unwrap().push((location.to_string(), kind));
        while kind == Get::Whole && self.stalled(location.as_ref()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.reachable()?;
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        if let Err(error) = self.reachable() {
            return futures_util::stream::once(async move { Err(error) }).boxed();
        }
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        self.reachable()?;
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.reachable()?;
        self.inner.copy_opts(from, to, options).await
    }

    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.reachable()?;
        self.inner.rename_opts(from, to, options).await
    }
}

/// The lease's Lance binding: URL paths below `prefix.0` become keys below `prefix.1`.
#[derive(Debug)]
struct CloudProvider {
    store: Arc<CloudStore>,
    prefix: Option<(String, String)>,
}

#[async_trait::async_trait]
impl ObjectStoreProvider for CloudProvider {
    async fn new_store(
        &self,
        url: Url,
        params: &ObjectStoreParams,
    ) -> flow_like_storage::lance::Result<LanceStore> {
        Ok(LanceStore::new(
            self.store.clone(),
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

    fn extract_path(&self, url: &Url) -> flow_like_storage::lance::Result<ObjectPath> {
        let path = match &self.prefix {
            Some((url_prefix, key_prefix)) => match url.path().strip_prefix(url_prefix.as_str()) {
                Some(suffix) => format!("{key_prefix}{suffix}"),
                None => url.path().to_string(),
            },
            None => url.path().to_string(),
        };
        ObjectPath::parse(path)
            .map_err(|error| flow_like_storage::lance::Error::invalid_input(error.to_string()))
    }
}

struct Cloud {
    directory: tempfile::TempDir,
    store: Arc<CloudStore>,
    registry: Arc<ObjectStoreRegistry>,
    /// Writes and replays go through a plain file connection to the same directory.
    hub: Connection,
    /// The cloud as the device reaches it.
    remote: Connection,
    url: String,
}

impl Cloud {
    async fn project() -> Result<Self> {
        Self::new(PROJECT_DB, "apps/project/storage/db", None).await
    }

    async fn user() -> Result<Self> {
        Self::new(
            "cloudsim://bucket/users/auth0%7C123/apps/project/db",
            "users/auth0|123/apps/project/db",
            Some((
                "/users/auth0%7C123/apps/project".into(),
                "users/auth0|123/apps/project".into(),
            )),
        )
        .await
    }

    async fn new(url: &str, key: &str, prefix: Option<(String, String)>) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let database = key
            .split('/')
            .fold(directory.path().to_path_buf(), |path, part| path.join(part));
        std::fs::create_dir_all(&database)?;
        let store = Arc::new(CloudStore {
            inner: LocalFileSystem::new_with_prefix(directory.path())?,
            faults: Default::default(),
            gets: Default::default(),
        });
        let registry = Arc::new(ObjectStoreRegistry::empty());
        registry.insert(
            "cloudsim",
            Arc::new(CloudProvider {
                store: store.clone(),
                prefix,
            }),
        );
        let session = Arc::new(Session::new(
            16 * MIB as usize,
            16 * MIB as usize,
            registry.clone(),
        ));
        let remote = lancedb::connect(url).session(session).execute().await?;
        let hub = lancedb::connect(database.to_str().context("UTF-8")?)
            .execute()
            .await?;
        Ok(Self {
            directory,
            store,
            registry,
            hub,
            remote,
            url: url.into(),
        })
    }

    async fn seed(&self, name: &str, fragments: i64, rows: i64, text: usize) -> Result<Table> {
        let table = self
            .hub
            .create_table(name, batch(0..rows, text)?)
            .execute()
            .await?;
        for fragment in 1..fragments {
            table
                .add(batch(fragment * rows..(fragment + 1) * rows, text)?)
                .execute()
                .await?;
        }
        Ok(table)
    }

    async fn table(&self, name: &str) -> Result<Table> {
        Ok(self.hub.open_table(name).execute().await?)
    }

    fn offline(&self, offline: bool) {
        self.store.faults().offline = offline;
    }

    fn file(&self, key: &str) -> PathBuf {
        key.split('/')
            .fold(self.directory.path().to_path_buf(), |path, part| {
                path.join(part)
            })
    }

    async fn revision(&self, name: &str) -> Result<OfflineExpected> {
        let table = self.remote.open_table(name).execute().await?;
        let (version, fingerprint) = offline_replay::revision(&table).await?;
        Ok(OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint),
        })
    }

    /// A shallow clone of `source` committed into this cloud database, with the source's
    /// cloud URI as its base path, as Lance's own clone would record it.
    async fn shallow_clone(&self, source: &str, target: &str) -> Result<()> {
        let source = self.remote.open_table(source).execute().await?;
        let dataset = source.dataset().context("native")?.get().await?;
        let store = dataset.object_store(None).await?;
        let indices =
            read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest()).await?;
        let base_id = dataset
            .manifest()
            .base_paths
            .keys()
            .max()
            .map_or(0, |id| id + 1);
        let mut manifest = dataset.manifest().shallow_clone(
            None,
            dataset.uri().into(),
            base_id,
            None,
            String::new(),
        );
        manifest.transaction_file = None;
        lance_table::feature_flags::apply_feature_flags(&mut manifest, false, false)?;
        let indices = indices
            .into_iter()
            .map(|mut index| {
                index.base_id.get_or_insert(base_id);
                index
            })
            .collect::<Vec<_>>();
        let uri = format!("{}/{target}.lance", self.url);
        let (store, base) = LanceStore::from_uri_and_params(
            self.registry.clone(),
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
                store.as_ref(),
                write_manifest_file_to_path,
                ManifestNamingScheme::V2,
                None,
            )
            .await
            .map_err(|error| anyhow::anyhow!("cloud clone failed: {error:?}"))?;
        Ok(())
    }

    /// A new cloud version whose indexes carry neither details nor file lists, as Lance
    /// wrote them before either existed.
    async fn legacy_indices(&self, name: &str) -> Result<()> {
        let table = self.table(name).await?;
        let dataset = table.dataset().context("native")?.get().await?;
        let store = dataset.object_store(None).await?;
        let mut indices =
            read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest()).await?;
        for index in &mut indices {
            index.index_details = None;
            index.files = None;
        }
        let mut manifest = dataset.manifest().clone();
        manifest.version += 1;
        manifest.transaction_file = None;
        manifest.transaction_section = None;
        commit_handler_from_url(dataset.uri(), &None)
            .await?
            .commit(
                &mut manifest,
                Some(indices),
                &dataset.branch_location().path,
                store.as_ref(),
                write_manifest_file_to_path,
                ManifestNamingScheme::V2,
                None,
            )
            .await
            .map_err(|error| anyhow::anyhow!("legacy index commit failed: {error:?}"))?;
        Ok(())
    }

    async fn compact_and_prune(&self, name: &str) -> Result<()> {
        let table = self.table(name).await?;
        table
            .optimize(OptimizeAction::Compact {
                options: CompactionOptions::default(),
                remap_options: None,
            })
            .await?;
        table
            .optimize(OptimizeAction::Prune {
                older_than: Some(LanceDuration::zero()),
                delete_unverified: Some(true),
                error_if_tagged_old_versions: Some(false),
            })
            .await?;
        Ok(())
    }
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("value", DataType::Int64, true),
        Field::new("text", DataType::Utf8, true),
        Field::new(
            "vec",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 4),
            true,
        ),
    ]))
}

fn vector(id: i64) -> Vec<f32> {
    vec![id as f32, (id % 10) as f32, 1.0, 0.5]
}

/// Barely compressible filler of `bytes` printable characters.
fn filler(id: i64, bytes: usize) -> String {
    let mut random = vec![0u8; bytes];
    blake3::Hasher::new()
        .update(&id.to_le_bytes())
        .finalize_xof()
        .fill(&mut random);
    for byte in &mut random {
        *byte = b'!' + *byte % 94;
    }
    String::from_utf8(random).expect("printable ASCII is UTF-8")
}

fn text(id: i64, bytes: usize) -> String {
    let word = if id % 7 == 0 { "needle" } else { "hay" };
    format!("row {id} {word} {}", filler(id, bytes))
}

fn batch(ids: Range<i64>, bytes: usize) -> Result<RecordBatch> {
    let ids = ids.collect::<Vec<_>>();
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        ids.iter()
            .map(|id| Some(vector(*id).into_iter().map(Some).collect::<Vec<_>>())),
        4,
    );
    Ok(RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(Int64Array::from(
                ids.iter().map(|id| id % 5).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from_iter_values(
                ids.iter().map(|id| text(*id, bytes)),
            )),
            Arc::new(vectors),
        ],
    )?)
}

fn row(id: i64) -> Value {
    json!({"id": id, "value": id % 5, "text": format!("row {id} needle fresh"), "vec": vector(id)})
}

fn selection(table: &str) -> BufferedTable {
    BufferedTable {
        purpose: StoragePurpose::Storage,
        database: "db".into(),
        table: table.into(),
        primary_key: "id".into(),
    }
}

fn user_selection() -> BufferedTable {
    BufferedTable {
        purpose: StoragePurpose::User,
        ..selection("measurements")
    }
}

fn setup(prefetch: bool) -> TableSetup {
    TableSetup {
        activation: TableActivation::Active,
        validate_key: false,
        prefetch,
    }
}

fn lazy_options() -> LazyMirrorOptions {
    LazyMirrorOptions {
        max_lazy_file_bytes: 64 * MIB,
        max_refresh_fetch_bytes: 64 * MIB,
        max_download_bytes_per_day: 1024 * MIB,
    }
}

fn limits(max_mirror_bytes: u64) -> BufferingConfig {
    BufferingConfig {
        max_mirror_bytes,
        ..BufferingConfig::default()
    }
}

fn options(root: &std::path::Path, breaker: Arc<Breaker>) -> WriteManagerOptions {
    WriteManagerOptions {
        lanes: QueueLanes::PerResource,
        coalesce_row_batches: true,
        recreate_dropped_tables: false,
        quarantine_other_scopes: false,
        refresh_interval: Duration::from_secs(120),
        retired_snapshot_grace: Some(Duration::from_secs(600)),
        connectivity: Some(breaker),
        mirror: MirrorMode::Lazy(lazy_options()),
        ..WriteManagerOptions::standalone(
            root.to_path_buf(),
            "project".into(),
            "a".repeat(64),
            limits(256 * MIB),
        )
    }
}

struct Device {
    root: tempfile::TempDir,
    breaker: Arc<Breaker>,
    host: Arc<TestHost>,
    writer: Arc<WriteManager>,
    unavailable: Arc<AtomicBool>,
}

impl Device {
    async fn open(cloud: &Cloud) -> Result<Self> {
        Self::with(cloud, |options| options).await
    }

    async fn with(
        cloud: &Cloud,
        configure: impl FnOnce(WriteManagerOptions) -> WriteManagerOptions,
    ) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let breaker = Arc::new(Breaker::default());
        let server = replay_server(cloud.hub.clone(), false, false);
        let unavailable = server.unavailable.clone();
        let host = TestHost::new(cloud.remote.clone(), Some(server));
        *host.objects.lock().unwrap() = Some(cloud.store.clone());
        let writer = WriteManager::open(
            configure(options(root.path(), breaker.clone())),
            host.clone(),
        )
        .await?;
        Ok(Self {
            root,
            breaker,
            host,
            writer,
            unavailable,
        })
    }

    /// The same device after a restart.
    async fn reopen(
        &mut self,
        configure: impl FnOnce(WriteManagerOptions) -> WriteManagerOptions,
    ) -> Result<()> {
        self.writer.close().await?;
        self.writer = WriteManager::open(
            configure(options(self.root.path(), self.breaker.clone())),
            self.host.clone(),
        )
        .await?;
        Ok(())
    }

    async fn add(&self, table: &BufferedTable, setup: TableSetup) -> Result<TableState> {
        self.writer.add_table(table.clone(), setup).await
    }

    fn overlay(&self, table: &BufferedTable) -> Result<Arc<TableOverlay>> {
        self.writer
            .overlay(&resource_key(table))
            .context("table is not registered")
    }

    async fn store(&self, table: &BufferedTable) -> Result<LanceDBVectorStore> {
        let prefix = match table.purpose {
            StoragePurpose::User => "users/auth0|123/apps/project/db",
            _ => "apps/project/storage/db",
        };
        self.writer
            .managed_store(
                &ObjectPath::parse(prefix)?,
                &table.table,
                DatabaseSelector::default(),
            )
            .await?
            .context("managed store")
    }

    async fn ids(&self, table: &BufferedTable, filter: &str) -> Result<Vec<i64>> {
        let mut ids = self
            .store(table)
            .await?
            .filter(filter, Some(vec!["id".into()]), 10_000, 0)
            .await?
            .iter()
            .map(|row| row["id"].as_i64().context("id"))
            .collect::<Result<Vec<_>>>()?;
        ids.sort_unstable();
        Ok(ids)
    }

    async fn insert(&self, table: &BufferedTable, id: i64) -> Result<String> {
        Ok(self
            .overlay(table)?
            .apply(LogicalTableMutation::Insert {
                items: vec![row(id)],
            })
            .await?
            .operation_id)
    }

    fn offline(&self, cloud: &Cloud, offline: bool) {
        self.breaker.offline.store(offline, Ordering::SeqCst);
        cloud.offline(offline);
    }

    fn mirror(&self) -> &Arc<flow_like_storage::databases::vector::offline_mirror::LazyMirror> {
        &self.writer.lazy().expect("lazy mirror").mirror
    }

    fn files(
        &self,
        table: &BufferedTable,
    ) -> Result<Vec<flow_like_storage::databases::vector::offline_mirror::MirrorFile>> {
        let (name, _, _) = self.writer.queue.local_view(&resource_key(table))?;
        Ok(self
            .writer
            .queue
            .snapshot(&name.context("local name")?)?
            .context("recorded snapshot")?
            .files)
    }

    fn data(&self, table: &BufferedTable) -> Result<Vec<String>> {
        Ok(self
            .files(table)?
            .into_iter()
            .filter(|file| file.key.contains("/data/"))
            .map(|file| file.key)
            .collect())
    }

    fn cached(&self, key: &str) -> bool {
        self.mirror().cached(key).is_some()
    }

    async fn state(&self, table: &BufferedTable) -> Result<TableState> {
        self.writer
            .table_states()
            .await?
            .into_iter()
            .find(|state| state.table == *table)
            .context("table state")
    }

    /// Every table file that was read online has been downloaded.
    async fn settle(&self, cloud: &Cloud) -> Result<()> {
        let touched = cloud
            .store
            .gets
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, kind)| {
                *kind != Get::Head
                    && ["/data/", "/_deletions/", "/_indices/"]
                        .iter()
                        .any(|directory| key.contains(directory))
            })
            .map(|(key, _)| key.clone())
            .collect::<HashSet<_>>();
        eventually(|| touched.iter().all(|key| self.cached(key))).await
    }

    fn gets(cloud: &Cloud, key: &str) -> usize {
        cloud.store.count(key, Get::Whole) + cloud.store.count(key, Get::Range)
    }
}

async fn eventually(mut done: impl FnMut() -> bool) -> Result<()> {
    for _ in 0..1000 {
        if done() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("condition not reached within 10 s")
}

async fn complete(device: &Device, table: &BufferedTable) -> Result<()> {
    for _ in 0..1000 {
        if device.state(table).await?.offline_complete {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    anyhow::bail!("the table did not become fully available offline")
}

fn head_request(writer: &WriteManager) -> Result<OfflineReplayRequest> {
    Ok(serde_json::from_value(
        writer.queue.head()?.context("queue is empty")?.payload,
    )?)
}

async fn indexed(cloud: &Cloud, name: &str) -> Result<Table> {
    let table = cloud.seed(name, 3, 100, 16).await?;
    table
        .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
        .execute()
        .await?;
    Ok(table)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lazy_add_table_downloads_no_table_files_and_keeps_only_version_metadata() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    let state = device.add(&table, setup(false)).await?;
    let fetched = cloud
        .store
        .gets
        .lock()
        .unwrap()
        .iter()
        .filter(|(key, kind)| !key.contains("/_versions/") && *kind != Get::Head)
        .count();
    assert_eq!(fetched, 0, "no table file was read");
    assert_eq!(device.mirror().cache_bytes(), 0);
    let (name, _, _) = device.writer.queue.local_view(&resource_key(&table))?;
    let local = device
        .writer
        .root()
        .join("tables")
        .join(format!("{}.lance", name.context("local name")?));
    for entry in std::fs::read_dir(&local)? {
        assert_eq!(entry?.file_name(), "_versions");
    }
    assert_eq!(device.data(&table)?.len(), 3);
    assert!(state.total_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(state.cached_bytes, 0);
    assert!(!state.offline_complete);
    assert_eq!(state.key_indexed, Some(true));
    assert_eq!(state.revision, Some(cloud.revision("measurements").await?));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_cache_touched_files_once_and_uncached_reads_fail_offline_with_e36() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    assert!(device.cached(&data[1]));
    assert!(!device.cached(&data[0]) && !device.cached(&data[2]));
    let requests = cloud.store.requests();
    assert_eq!(device.ids(&table, "id = 160").await?, vec![160]);
    assert_eq!(
        Device::gets(&cloud, &data[1]),
        2,
        "one read and one download; later reads use the cache"
    );
    assert!(cloud.store.requests() >= requests);
    device.offline(&cloud, true);
    assert_eq!(device.ids(&table, "id = 170").await?, vec![170]);
    let error = device.ids(&table, "id = 250").await.unwrap_err();
    assert_eq!(error.to_string(), E36);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn online_misses_read_ranges_at_once_and_download_in_the_background() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 20, 40_000).await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    cloud.store.stall(&data[1]);
    assert_eq!(device.ids(&table, "id = 25").await?, vec![25]);
    assert!(cloud.store.count(&data[1], Get::Range) > 0);
    assert!(
        !device.cached(&data[1]),
        "the read did not wait for the download"
    );
    cloud.store.release();
    eventually(|| device.cached(&data[1])).await?;
    assert_eq!(cloud.store.count(&data[1], Get::Whole), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_downloads_never_evict_recent_files() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 20, 20_000).await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        limits: limits(MIB),
        ..options
    })
    .await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    let sizes = device
        .files(&table)?
        .iter()
        .filter_map(|file| file.bytes.filter(|_| file.key.contains("/data/")))
        .collect::<Vec<_>>();
    assert!(sizes.iter().take(2).sum::<u64>() < 900 * 1024 && sizes.iter().sum::<u64>() > MIB);
    assert_eq!(device.ids(&table, "id = 5").await?, vec![5]);
    eventually(|| data.iter().filter(|key| device.cached(key)).count() == 2).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let cached = data.iter().filter(|key| device.cached(key)).count();
    assert_eq!(
        cached, 2,
        "the third download would have evicted a recent file"
    );
    let downloads = data
        .iter()
        .map(|key| cloud.store.count(key, Get::Whole))
        .sum::<usize>();
    assert_eq!(downloads, 2, "the skipped download never reached the cloud");
    assert!(device.writer.budget.used() <= device.writer.budget.maximum());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_miss_fails_at_once_while_the_breaker_is_open() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    for key in &data {
        cloud.store.stall(key);
    }
    device.breaker.offline.store(true, Ordering::SeqCst);
    let started = Instant::now();
    assert_eq!(
        device
            .ids(&table, "id = 150")
            .await
            .unwrap_err()
            .to_string(),
        E36
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        data.iter().all(|key| cloud.store.count(key, Get::Range)
            + cloud.store.count(key, Get::Whole)
            == 0),
        "the cloud was not asked while the breaker was open"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn freeze_miss_offline_enqueues_nothing_and_returns_e36() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 100, 16).await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    device.offline(&cloud, true);
    assert_eq!(
        device.insert(&table, 1000).await.unwrap_err().to_string(),
        E36
    );
    assert_eq!(device.writer.queue.status()?.pending_count, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn freeze_warm_up_downloads_outside_the_manager_gate() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 100, 16).await?;
    let device = Arc::new(Device::open(&cloud).await?);
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    cloud.store.stall("/data/");
    let writing = {
        let device = device.clone();
        let table = table.clone();
        tokio::spawn(async move { device.insert(&table, 1000).await })
    };
    eventually(|| {
        data.iter()
            .any(|key| cloud.store.count(key, Get::Whole) > 0)
    })
    .await?;
    assert!(
        device.writer.gate.try_lock().is_ok(),
        "the warm-up downloads without holding the manager gate"
    );
    assert!(!writing.is_finished());
    cloud.store.release();
    writing.await??;
    assert_eq!(device.writer.queue.status()?.pending_count, 1);
    assert!(data.iter().all(|key| device.cached(key)));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_reads_keep_files_regardless_of_the_allowance_and_fail_with_e39_when_full()
-> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 20, 20_000).await?;
    let table = selection("measurements");
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        mirror: MirrorMode::Lazy(LazyMirrorOptions {
            max_download_bytes_per_day: 1,
            ..lazy_options()
        }),
        ..options
    })
    .await?;
    device.add(&table, setup(false)).await?;
    device.insert(&table, 1000).await?;
    let data = device.data(&table)?;
    assert!(
        data.iter().all(|key| device.cached(key)),
        "the write kept its files"
    );
    assert!(device.writer.queue.downloaded_today(unix_time()?)? > 1);

    let full = Device::with(&cloud, |options| WriteManagerOptions {
        limits: limits(MIB),
        ..options
    })
    .await?;
    full.add(&table, setup(false)).await?;
    let error = full.insert(&table, 1001).await.unwrap_err().to_string();
    assert!(
        error.starts_with(
            "Offline storage on this device is full: table 'measurements' needs "
        ) && error.ends_with(
            " more for this change. Increase the limit on the Offline access page, or turn off \"Download everything\" for another table."
        ),
        "{error}"
    );
    assert_eq!(full.writer.queue.status()?.pending_count, 0);
    Ok(())
}

/// Enqueues an upsert as a crash between enqueue and its local materialization leaves it.
fn enqueue_unmaterialized(device: &Device, table: &BufferedTable, id: i64) -> Result<String> {
    let key = resource_key(table);
    let request = OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: OfflineResource::Table {
            purpose: table.purpose,
            database: table.database.clone(),
            table: table.table.clone(),
        },
        expected: serde_json::from_value(
            device
                .writer
                .queue
                .resource_revision(&key)?
                .context("revision")?,
        )?,
        mutation: OfflineMutation::TableUpsert {
            id_field: "id".into(),
            rows: vec![row(id)],
        },
    };
    Ok(device
        .writer
        .queue
        .enqueue(&key, serde_json::to_value(request)?, None, unix_time()?)?
        .operation_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovery_miss_does_not_block_the_operation() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 100, 16).await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let id = enqueue_unmaterialized(&device, &table, 150)?;
    device.offline(&cloud, true);
    let overlay = device.overlay(&table)?;
    assert_eq!(overlay.read_table().await.unwrap_err().to_string(), E36);
    let state = device.writer.queue.operation_state(&id)?.context("state")?;
    assert_eq!((state.state.as_str(), state.error), ("pending", None));
    assert!(!device.writer.drain_once().await?);
    assert_eq!(
        device
            .writer
            .queue
            .operation_state(&id)?
            .context("state")?
            .state,
        "pending"
    );
    device.offline(&cloud, false);
    assert!(overlay.read_table().await?.is_some());
    assert!(
        device
            .writer
            .queue
            .operation(&id)?
            .context("operation")?
            .local_version
            .is_some()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drain_recovery_miss_backs_off_only_its_lane() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 100, 16).await?;
    cloud.seed("readings", 1, 100, 16).await?;
    let device = Device::open(&cloud).await?;
    let (missing, healthy) = (selection("measurements"), selection("readings"));
    device.add(&missing, setup(false)).await?;
    device.add(&healthy, setup(false)).await?;
    let stuck = enqueue_unmaterialized(&device, &missing, 150)?;
    device.insert(&healthy, 1000).await?;
    *device.host.objects.lock().unwrap() = None;
    assert!(
        !device.writer.drain_once().await?,
        "the missing lane backs off"
    );
    assert!(device.writer.drain_once().await?, "the other lane replays");
    let replayed = cloud
        .table("readings")
        .await?
        .count_rows(Some("id = 1000".into()))
        .await?;
    assert_eq!(replayed, 1);
    let state = device
        .writer
        .queue
        .operation_state(&stuck)?
        .context("state")?;
    assert_eq!((state.state.as_str(), state.error), ("pending", None));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_fetches_only_the_version_delta_of_a_partly_cached_table() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let before = device.data(&table)?;
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    source.add(batch(300..400, 16)?).execute().await?;
    let RefreshOutcome::Refreshed {
        downloaded_bytes, ..
    } = device.writer.refresh_table(&table).await?
    else {
        anyhow::bail!("the refresh did not switch")
    };
    let after = device.data(&table)?;
    let added = after
        .iter()
        .filter(|key| !before.contains(key))
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    assert!(device.cached(added[0]));
    assert_eq!(
        downloaded_bytes,
        device.mirror().cached(added[0]).context("cached")?
    );
    assert!(!device.cached(&before[0]) && !device.cached(&before[2]));
    assert_eq!(cloud.store.count(&before[0], Get::Whole), 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_over_the_cap_switches_without_fetching_for_lazy_tables() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        mirror: MirrorMode::Lazy(LazyMirrorOptions {
            max_refresh_fetch_bytes: 1,
            ..lazy_options()
        }),
        ..options
    })
    .await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    source.add(batch(300..400, 16)?).execute().await?;
    assert!(matches!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Refreshed {
            downloaded_bytes: 0,
            ..
        }
    ));
    assert_eq!(device.mirror().cache_bytes(), 0);
    assert_eq!(
        device.state(&table).await?.revision,
        Some(cloud.revision("measurements").await?)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_downloads_never_block_the_drain() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    indexed(&cloud, "readings").await?;
    let device = Device::open(&cloud).await?;
    let (table, other) = (selection("measurements"), selection("readings"));
    device.add(&table, setup(false)).await?;
    device.add(&other, setup(false)).await?;
    let before = device.data(&table)?;
    device.writer.spawn_drain();
    source.add(batch(300..400, 16)?).execute().await?;
    cloud.store.stall("/measurements.lance/data/");
    let refresh = {
        let writer = device.writer.clone();
        let table = table.clone();
        tokio::spawn(async move { writer.refresh_table(&table).await })
    };
    let store = cloud.store.clone();
    eventually(move || {
        store.gets.lock().unwrap().iter().any(|(key, kind)| {
            *kind == Get::Whole && key.contains("/data/") && !before.contains(key)
        })
    })
    .await?;
    device.insert(&other, 1000).await?;
    let replayed = cloud.table("readings").await?;
    for _ in 0..1000 {
        replayed.checkout_latest().await?;
        if replayed.count_rows(Some("id = 1000".into())).await? == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(replayed.count_rows(Some("id = 1000".into())).await?, 1);
    assert!(
        !refresh.is_finished(),
        "the other lane replayed while the refresh was downloading"
    );
    cloud.store.release();
    assert!(matches!(refresh.await??, RefreshOutcome::Refreshed { .. }));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acknowledged_local_changes_are_rebased_and_become_evictable() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        retired_snapshot_grace: Some(Duration::ZERO),
        ..options
    })
    .await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    device.insert(&table, 1000).await?;
    let cached = device.mirror().cache_bytes();
    assert!(cached > 0);
    device.writer.evict_mirror(0).await?;
    assert_eq!(
        device.mirror().cache_bytes(),
        cached,
        "a pending lane pins its files"
    );
    let (old, _, _) = device.writer.queue.local_view(&resource_key(&table))?;
    let old = old.context("local name")?;
    assert!(device.writer.drain_once().await?);
    assert!(matches!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Refreshed { .. }
    ));
    let (current, branch, base) = device.writer.queue.local_view(&resource_key(&table))?;
    let current = current.context("local name")?;
    assert_ne!(current, old);
    assert_eq!(branch, "main");
    assert_eq!(
        base,
        Some(
            device
                .writer
                .queue
                .snapshot(&current)?
                .context("recorded")?
                .version
        )
    );
    device.writer.idle_pass().await;
    assert!(
        !device
            .writer
            .root()
            .join("tables")
            .join(format!("{old}.lance"))
            .exists(),
        "the change files left the device with the retired snapshot"
    );
    assert!(device.writer.queue.snapshot(&old)?.is_none());
    device.writer.evict_mirror(0).await?;
    assert_eq!(device.mirror().cache_bytes(), 0);
    assert_eq!(device.ids(&table, "id = 1000").await?, vec![1000]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropped_refresh_retries_soon_and_reuses_its_fetched_files() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Arc::new(Device::open(&cloud).await?);
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let before = device.data(&table)?;
    source.add(batch(300..400, 16)?).execute().await?;
    cloud.store.stall("/data/");
    let refresh = {
        let device = device.clone();
        let table = table.clone();
        tokio::spawn(async move { device.writer.refresh_table(&table).await })
    };
    let store = cloud.store.clone();
    let known = before.clone();
    eventually(move || {
        store.gets.lock().unwrap().iter().any(|(key, kind)| {
            *kind == Get::Whole && key.contains("/data/") && !known.contains(key)
        })
    })
    .await?;
    let started = Instant::now();
    device.insert(&table, 1000).await?;
    cloud.store.release();
    assert_eq!(refresh.await??, RefreshOutcome::Deferred);
    let due = device
        .writer
        .refresh_retry_due(&resource_key(&table))
        .context("retry scheduled")?;
    assert!(due > started && due <= Instant::now() + Duration::from_secs(10));
    let fetched = cloud
        .store
        .gets
        .lock()
        .unwrap()
        .iter()
        .find(|(key, kind)| *kind == Get::Whole && key.contains("/data/") && !before.contains(key))
        .map(|(key, _)| key.clone())
        .context("fetched file")?;
    assert!(device.cached(&fetched), "a dropped refresh keeps its files");
    let head = device.writer.queue.head()?.context("queued write")?;
    device
        .writer
        .queue
        .request_skip(&head.operation_id, "Keep the cloud version", false)?;
    device.writer.drain_once().await?;
    assert_eq!(device.writer.queue.status()?.pending_count, 0);
    assert!(matches!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Refreshed { .. }
    ));
    assert!(device.data(&table)?.contains(&fetched));
    assert_eq!(cloud.store.count(&fetched, Get::Whole), 1);
    Ok(())
}

async fn search_everything(
    device: &Device,
    table: &BufferedTable,
) -> Result<(usize, usize, usize)> {
    let store = device.store(table).await?;
    let fts = store
        .fts_search("needle", None, Some(vec!["id".into()]), None, 1000, 0)
        .await?
        .len();
    let nearest = store
        .vector_search(
            vec![150.0, 0.0, 1.0, 0.5],
            None,
            Some(vec!["id".into()]),
            3,
            0,
        )
        .await?
        .len();
    Ok((store.count(None).await?, fts, nearest))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetch_makes_reads_writes_fts_and_vector_search_work_offline() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    source.delete("id = 5").await?;
    source
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    source
        .create_index(
            &["vec"],
            Index::IvfFlat(IvfFlatIndexBuilder::default().num_partitions(2)),
        )
        .execute()
        .await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(true)).await?;
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    let online = search_everything(&device, &table).await?;
    device.offline(&cloud, true);
    let requests = cloud.store.requests();
    assert_eq!(search_everything(&device, &table).await?, online);
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    device.insert(&table, 1000).await?;
    assert_eq!(device.ids(&table, "id = 1000").await?, vec![1000]);
    assert_eq!(online.0, 299);
    assert_eq!(
        cloud.store.requests(),
        requests,
        "nothing reached the cloud offline"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bitmap_and_ivf_pq_indices_work_on_the_mirror() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = cloud.seed("measurements", 3, 200, 16).await?;
    source
        .create_index(&["value"], Index::Bitmap(BitmapIndexBuilder::default()))
        .execute()
        .await?;
    source
        .create_index(
            &["vec"],
            Index::IvfPq(
                IvfPqIndexBuilder::default()
                    .num_partitions(2)
                    .num_sub_vectors(2),
            ),
        )
        .execute()
        .await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(true)).await?;
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    device.offline(&cloud, true);
    let requests = cloud.store.requests();
    assert_eq!(device.ids(&table, "value = 3").await?.len(), 120);
    let nearest = device
        .store(&table)
        .await?
        .vector_search(
            vec![150.0, 0.0, 1.0, 0.5],
            None,
            Some(vec!["id".into()]),
            3,
            0,
        )
        .await?;
    assert_eq!(nearest.len(), 3);
    assert_eq!(cloud.store.requests(), requests);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_bitmap_and_fts_filters_work_offline_without_index_files_cached() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = cloud.seed("measurements", 2, 100, 16).await?;
    source
        .create_index(&["value"], Index::Bitmap(BitmapIndexBuilder::default()))
        .execute()
        .await?;
    source
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    cloud.legacy_indices("measurements").await?;
    let mut device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let files = device.files(&table)?;
    let index_files = files
        .iter()
        .filter(|file| file.key.contains("/_indices/"))
        .collect::<Vec<_>>();
    assert!(!index_files.is_empty() && index_files.iter().all(|file| file.bytes.is_some()));
    let filters = ["value = 3", "text LIKE 'row 7 %'"];
    let mut online = Vec::new();
    for filter in filters {
        online.push(device.ids(&table, filter).await?);
    }
    assert_eq!(
        online,
        vec![
            (0..200).filter(|id| id % 5 == 3).collect::<Vec<_>>(),
            vec![7]
        ]
    );
    device.settle(&cloud).await?;
    device.reopen(|options| options).await?;
    device.add(&table, setup(false)).await?;
    device.offline(&cloud, true);
    let requests = cloud.store.requests();
    for (filter, expected) in filters.iter().zip(&online) {
        assert_eq!(&device.ids(&table, filter).await?, expected);
    }
    assert_eq!(
        cloud.store.requests(),
        requests,
        "existence probes were answered locally"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetched_refresh_downloads_every_new_file_before_the_checkpoint() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(true)).await?;
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    source.add(batch(300..400, 16)?).execute().await?;
    let RefreshOutcome::Refreshed {
        downloaded_bytes, ..
    } = device.writer.refresh_table(&table).await?
    else {
        anyhow::bail!("the refresh did not switch")
    };
    assert!(downloaded_bytes > 0);
    let state = device.state(&table).await?;
    assert!(
        state.offline_complete,
        "the new version was complete when it became current"
    );
    assert_eq!(state.revision, Some(cloud.revision("measurements").await?));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefetched_refresh_is_deferred_when_it_does_not_fit_and_keeps_the_complete_copy()
-> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(true)).await?;
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    let revision = device.state(&table).await?.revision;
    source.add(batch(300..320, 20_000)?).execute().await?;
    device
        .writer
        .budget
        .set_maximum(device.writer.budget.used() + 64 * 1024);
    assert_eq!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Deferred
    );
    let state = device.state(&table).await?;
    let error = state.mirror_error.clone().context("mirror error")?;
    assert!(
        error.starts_with(
            "Not enough offline storage on this device to download the latest version of 'measurements' ("
        ) && error.ends_with(" needed)."),
        "{error}"
    );
    assert_eq!(state.revision, revision);
    assert!(state.offline_complete);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn activating_a_download_everything_table_switches_metadata_first_and_keeps_downloading()
-> Result<()> {
    let held = TableSetup {
        activation: TableActivation::Held,
        ..setup(true)
    };
    let table = selection("measurements");
    for case in ["downloads", "allowance", "disk"] {
        let cloud = Cloud::project().await?;
        let source = indexed(&cloud, "measurements").await?;
        let device = Device::with(&cloud, |options| match case {
            "allowance" => WriteManagerOptions {
                mirror: MirrorMode::Lazy(LazyMirrorOptions {
                    max_download_bytes_per_day: 1,
                    ..lazy_options()
                }),
                ..options
            },
            _ => options,
        })
        .await?;
        device.add(&table, held).await?;
        source.add(batch(300..400, 16)?).execute().await?;
        if case == "disk" {
            device
                .writer
                .budget
                .set_maximum(device.writer.budget.used() + 64 * 1024);
        }
        let state = device.writer.activate_table(&table).await?;
        assert_eq!(state.activation, Some(TableActivation::Active), "{case}");
        assert_eq!(state.revision, Some(cloud.revision("measurements").await?));
        assert!(
            !state.offline_complete,
            "{case}: activation did not wait for downloads"
        );
        device.writer.spawn_drain();
        match case {
            "downloads" => complete(&device, &table).await?,
            _ => {
                let mut error = None;
                for _ in 0..1000 {
                    error = device.state(&table).await?.mirror_error;
                    if error.is_some() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let error = error.context("mirror error")?;
                if case == "allowance" {
                    assert_eq!(error, ALLOWANCE);
                } else {
                    assert!(error.starts_with("Not enough offline storage"), "{error}");
                }
                assert!(!device.state(&table).await?.offline_complete);
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fast_forward_of_a_download_everything_table_switches_metadata_first() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        fast_forward: Some(FastForward {
            probe_timeout: Duration::from_secs(10),
            min_interval: Duration::ZERO,
        }),
        ..options
    })
    .await?;
    let table = selection("measurements");
    device.add(&table, setup(true)).await?;
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    source
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    cloud.store.stall("/_indices/");
    device.unavailable.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(20), device.insert(&table, 1000)).await??;
    assert_eq!(
        head_request(&device.writer)?.expected,
        cloud.revision("measurements").await?,
        "the write was frozen against the new version"
    );
    assert!(!device.state(&table).await?.offline_complete);
    cloud.store.release();
    device.writer.wake();
    complete(&device, &table).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pending_lanes_pin_listed_keys_without_downloading_large_files_whole() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    device.unavailable.store(true, Ordering::SeqCst);
    device.insert(&table, 1000).await?;
    let cached = device.mirror().cache_bytes();
    device.writer.evict_mirror(0).await?;
    assert_eq!(
        device.mirror().cache_bytes(),
        cached,
        "a pending lane pins its files"
    );
    assert!(device.writer.mirror_usage()?.pinned_bytes >= cached);
    device
        .writer
        .set_limits(
            limits(256 * MIB),
            DESKTOP_OFFLINE_LIMITS,
            Some(LazyMirrorOptions {
                max_lazy_file_bytes: 1,
                ..lazy_options()
            }),
        )
        .await?;
    device
        .overlay(&table)?
        .apply(LogicalTableMutation::Upsert {
            id_field: "id".into(),
            items: vec![row(250)],
        })
        .await?;
    assert!(
        !device.cached(&data[2]),
        "files over the cap are read, not kept"
    );
    assert!(Device::gets(&cloud, &data[2]) > 0);
    assert_eq!(device.writer.queue.status()?.pending_count, 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eviction_takes_garbage_then_lru_and_never_pinned_files() -> Result<()> {
    let cloud = Cloud::project().await?;
    for name in ["pinned", "live", "removed"] {
        indexed(&cloud, name).await?;
    }
    let device = Device::open(&cloud).await?;
    let (pinned, live, removed) = (selection("pinned"), selection("live"), selection("removed"));
    device.add(&pinned, setup(true)).await?;
    device.add(&live, setup(false)).await?;
    device.add(&removed, setup(false)).await?;
    device.writer.spawn_drain();
    complete(&device, &pinned).await?;
    assert_eq!(device.ids(&live, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    assert_eq!(device.ids(&removed, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    let garbage = device.data(&removed)?;
    let live_files = device.data(&live)?;
    device.writer.remove_table(&removed).await?;
    let used = device.writer.budget.used();
    device.writer.evict_mirror(used - 1).await?;
    assert!(
        garbage.iter().any(|key| !device.cached(key)),
        "garbage goes first"
    );
    assert!(live_files.iter().any(|key| device.cached(key)));
    device.writer.evict_mirror(0).await?;
    assert!(garbage.iter().all(|key| !device.cached(key)));
    assert!(live_files.iter().all(|key| !device.cached(key)));
    assert!(
        device.state(&pinned).await?.offline_complete,
        "pinned files stay"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lowering_the_limit_evicts_and_refuses_below_required() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 1, 20, 5_000_000).await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        mirror: MirrorMode::Lazy(LazyMirrorOptions {
            max_lazy_file_bytes: 256 * MIB,
            ..lazy_options()
        }),
        ..options
    })
    .await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    assert_eq!(device.ids(&table, "id = 5").await?, vec![5]);
    device.settle(&cloud).await?;
    let required = device.writer.mirror_usage()?.required;
    assert!(device.writer.budget.used() > required);
    let error = device
        .writer
        .set_limits(limits(required - 1), DESKTOP_OFFLINE_LIMITS, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Tables that download everything and tables with queued changes need at least {} on this device; choose a limit of at least that size.",
            format_limit(required as usize)
        )
    );
    assert!(device.mirror().cache_bytes() > 0);
    device
        .writer
        .set_limits(limits(required), DESKTOP_OFFLINE_LIMITS, None)
        .await?;
    assert_eq!(device.mirror().cache_bytes(), 0);
    let usage = device.writer.mirror_usage()?;
    assert!(usage.used <= usage.maximum && usage.maximum == required);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn vanished_data_deletion_and_index_files_report_e37_and_the_read_is_retried_after_refresh()
-> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    source.delete("id = 5").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let old = device.files(&table)?;
    assert!(old.iter().any(|file| file.key.contains("/_deletions/")));
    cloud.compact_and_prune("measurements").await?;
    assert!(
        old.iter().all(|file| !cloud.file(&file.key).exists()),
        "compaction removed every data, deletion and index file of the snapshot"
    );
    assert_eq!(device.ids(&table, "id = 20").await?, vec![20]);
    assert_eq!(
        device.state(&table).await?.revision,
        Some(cloud.revision("measurements").await?)
    );

    source.add(batch(300..400, 16)?).execute().await?;
    device.writer.refresh_table(&table).await?;
    device.writer.evict_mirror(0).await?;
    device.unavailable.store(true, Ordering::SeqCst);
    device.insert(&table, 1000).await?;
    let gone = device
        .data(&table)?
        .into_iter()
        .find(|key| !device.cached(key))
        .context("an uncached data file")?;
    std::fs::remove_file(cloud.file(&gone))?;
    assert_eq!(
        device.ids(&table, "id >= 0").await.unwrap_err().to_string(),
        E37
    );
    assert_eq!(
        device.state(&table).await?.mirror_error.as_deref(),
        Some(
            "Part of this table was reorganized in the cloud. The copy refreshes once its queued changes are synced."
        )
    );
    let unrelated = device
        .ids(&table, "no_such_column = 1")
        .await
        .unwrap_err()
        .to_string();
    assert!(unrelated != E37 && unrelated != E36, "{unrelated}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrelated_read_errors_are_not_reported_as_e37() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    device.unavailable.store(true, Ordering::SeqCst);
    device.insert(&table, 1000).await?;
    let data = device.data(&table)?;
    std::fs::remove_file(cloud.file(&data[2]))?;
    assert_eq!(
        device
            .ids(&table, "id = 250")
            .await
            .unwrap_err()
            .to_string(),
        E37
    );
    assert!(!device.mirror().stale_keys().is_empty());
    let error = device
        .ids(&table, "no_such_column = 1")
        .await
        .unwrap_err()
        .to_string();
    assert_ne!(error, E37);
    assert!(error.contains("no_such_column"), "{error}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn key_validation_scans_the_cloud_version_without_caching() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 3, 100, 16).await?;
    let dupes = cloud.seed("dupes", 1, 100, 16).await?;
    dupes.add(batch(10..12, 16)?).execute().await?;
    let device = Device::open(&cloud).await?;
    let validated = TableSetup {
        validate_key: true,
        ..setup(false)
    };
    let table = selection("measurements");
    device.add(&table, validated).await?;
    assert_eq!(device.mirror().cache_bytes(), 0);
    let data = device.data(&table)?;
    assert!(
        data.iter().all(|key| Device::gets(&cloud, key) > 0),
        "the key column was read from the cloud"
    );
    let error = device
        .add(&selection("dupes"), validated)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "Column 'id' is not a unique, non-empty key in 'dupes': 4 rows are empty or duplicated."
    );
    assert!(
        !device
            .writer
            .table_is_managed(&ObjectPath::from("apps/project/storage/db"), "dupes")
    );
    assert_eq!(device.writer.table_states().await?.len(), 1);
    assert_eq!(device.mirror().cache_bytes(), 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_states_report_cached_total_offline_complete_and_mirror_errors_per_table()
-> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "complete").await?;
    cloud.seed("partial", 3, 100, 16).await?;
    let device = Device::open(&cloud).await?;
    let (complete_table, partial) = (selection("complete"), selection("partial"));
    device.add(&complete_table, setup(true)).await?;
    device.add(&partial, setup(false)).await?;
    device.writer.spawn_drain();
    complete(&device, &complete_table).await?;
    cloud.offline(true);
    assert!(device.writer.refresh_table(&partial).await.is_err());
    let full = device.state(&complete_table).await?;
    assert!(full.prefetch && full.offline_complete && !full.downloading);
    assert_eq!(Some(full.cached_bytes), full.total_bytes);
    assert_eq!(full.key_indexed, Some(true));
    assert_eq!(full.mirror_error, None);
    let partial = device.state(&partial).await?;
    assert!(!partial.prefetch && !partial.offline_complete);
    assert_eq!(partial.cached_bytes, 0);
    assert!(partial.total_bytes.is_some_and(|total| total > 0));
    assert_eq!(partial.key_indexed, Some(false));
    assert!(
        partial
            .mirror_error
            .as_deref()
            .is_some_and(|error| error.starts_with("Could not refresh from the cloud: "))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn set_prefetch_off_stops_the_download_and_keeps_files_until_evicted() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let files = device.files(&table)?;
    cloud.store.stall(&files[1].key);
    device.writer.spawn_drain();
    assert!(device.writer.set_prefetch(&table, true).await?.prefetch);
    eventually(|| device.cached(&files[0].key)).await?;
    let state = device.writer.set_prefetch(&table, false).await?;
    assert!(!state.prefetch);
    cloud.store.release();
    tokio::time::sleep(Duration::from_millis(300)).await;
    device.writer.wake();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = device.state(&table).await?;
    assert!(
        !state.downloading && !state.offline_complete,
        "the download stopped"
    );
    let cached = device.mirror().cache_bytes();
    assert!(
        cached > 0,
        "nothing is deleted when Download everything turns off"
    );
    device.writer.evict_mirror(0).await?;
    assert_eq!(device.mirror().cache_bytes(), 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_keys_of_a_clone_source_refresh_the_tables_that_list_them() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "source").await?;
    cloud.shallow_clone("source", "measurements").await?;
    let device = Device::open(&cloud).await?;
    let (source, clone) = (selection("source"), selection("measurements"));
    device.add(&source, setup(false)).await?;
    device.add(&clone, setup(false)).await?;
    let shared = device.data(&clone)?;
    assert!(shared.iter().all(|key| key.contains("/source.lance/")));
    device.writer.spawn_drain();
    device.unavailable.store(true, Ordering::SeqCst);
    device.insert(&clone, 1000).await?;
    let before = device.state(&source).await?.revision;
    cloud.compact_and_prune("source").await?;
    assert_eq!(
        device
            .ids(&clone, "id = 250")
            .await
            .unwrap_err()
            .to_string(),
        E37
    );
    device.writer.idle_pass().await;
    for _ in 0..1000 {
        if device.state(&source).await?.revision != before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        device.state(&source).await?.revision,
        Some(cloud.revision("source").await?),
        "the source table that lists the stale key refreshed at once"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_download_everything_works_for_an_escaped_user_subject() -> Result<()> {
    let cloud = Cloud::user().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = user_selection();
    device.add(&table, setup(true)).await?;
    let files = device.files(&table)?;
    assert!(files.iter().all(|file| {
        file.key
            .starts_with("users/auth0|123/apps/project/db/measurements.lance/")
    }));
    device.writer.spawn_drain();
    complete(&device, &table).await?;
    device.offline(&cloud, true);
    let requests = cloud.store.requests();
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    assert_eq!(device.ids(&table, "id >= 0").await?.len(), 300);
    assert_eq!(
        cloud.store.requests(),
        requests,
        "every key Lance requested was listed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn materialize_mode_creates_no_mirror_directory() -> Result<()> {
    let cloud = Cloud::project().await?;
    cloud.seed("measurements", 1, 10, 16).await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        mirror: MirrorMode::Materialize,
        ..options
    })
    .await?;
    let table = selection("measurements");
    let state = device.add(&table, setup(false)).await?;
    assert!(!device.writer.root().join("mirror").exists());
    assert!(device.writer.lazy().is_none());
    assert_eq!(
        (
            state.prefetch,
            state.cached_bytes,
            state.total_bytes,
            state.offline_complete,
            state.downloading,
            state.key_indexed
        ),
        (false, 0, None, true, false, None)
    );
    assert_eq!(
        device
            .writer
            .set_prefetch(&table, true)
            .await
            .unwrap_err()
            .to_string(),
        "Download everything needs a lazy offline mirror"
    );
    assert_eq!(device.writer.evict_mirror(0).await?, 0);
    assert_eq!(device.writer.mirror_usage()?.cache_bytes, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn daily_download_allowance_defers_refreshes_and_prefetch_but_not_online_reads_or_writes()
-> Result<()> {
    let cloud = Cloud::project().await?;
    let everything = indexed(&cloud, "everything").await?;
    let lazy = indexed(&cloud, "measurements").await?;
    let device = Device::with(&cloud, |options| WriteManagerOptions {
        mirror: MirrorMode::Lazy(LazyMirrorOptions {
            max_download_bytes_per_day: 1,
            ..lazy_options()
        }),
        ..options
    })
    .await?;
    let (prefetched, table) = (selection("everything"), selection("measurements"));
    device.add(&prefetched, setup(true)).await?;
    device.add(&table, setup(false)).await?;
    device.writer.spawn_drain();
    let mut error = None;
    for _ in 0..1000 {
        error = device.state(&prefetched).await?.mirror_error;
        if error.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(error.as_deref(), Some(ALLOWANCE));
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        device.state(&table).await?.cached_bytes,
        0,
        "reads stay ranged"
    );
    device
        .overlay(&table)?
        .apply(LogicalTableMutation::Update {
            filter: "id = 250".into(),
            updates: vec![("value".into(), "1".into())],
        })
        .await?;
    assert!(
        device.cached(&device.data(&table)?[2]),
        "writes still download"
    );
    let queue = &device.writer.queue;
    eventually(|| queue.status().is_ok_and(|status| status.pending_count == 0)).await?;
    everything.add(batch(300..400, 16)?).execute().await?;
    lazy.add(batch(400..500, 16)?).execute().await?;
    assert_eq!(
        device.writer.refresh_table(&prefetched).await?,
        RefreshOutcome::Deferred
    );
    assert_eq!(
        device.state(&prefetched).await?.mirror_error.as_deref(),
        Some(ALLOWANCE)
    );
    let outcome = device.writer.refresh_table(&table).await?;
    assert!(
        matches!(
            outcome,
            RefreshOutcome::Refreshed {
                downloaded_bytes: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_allowance_charges_transferred_bytes_once() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let today = || {
        device
            .writer
            .queue
            .downloaded_today(unix_time().unwrap())
            .unwrap()
    };
    assert_eq!(today(), 0);
    assert_eq!(device.ids(&table, "id = 150").await?, vec![150]);
    device.settle(&cloud).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(today(), device.mirror().cache_bytes());
    assert_eq!(device.ids(&table, "id = 160").await?, vec![160]);
    assert_eq!(today(), device.mirror().cache_bytes());
    assert_eq!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Unchanged
    );
    assert_eq!(
        today(),
        device.mirror().cache_bytes(),
        "refreshes charge nothing"
    );
    source.add(batch(300..400, 16)?).execute().await?;
    let charged = today();
    let RefreshOutcome::Refreshed {
        downloaded_bytes, ..
    } = device.writer.refresh_table(&table).await?
    else {
        anyhow::bail!("the refresh did not switch")
    };
    assert_eq!(today(), charged + downloaded_bytes);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_table_keeps_the_snapshot_row_of_another_tables_refresh() -> Result<()> {
    let cloud = Cloud::project().await?;
    let source = indexed(&cloud, "measurements").await?;
    indexed(&cloud, "readings").await?;
    let device = Device::open(&cloud).await?;
    let (table, other) = (selection("measurements"), selection("readings"));
    device.add(&table, setup(false)).await?;
    device.add(&other, setup(false)).await?;
    let before = device.data(&table)?;
    source.add(batch(300..400, 16)?).execute().await?;
    cloud.store.stall("/measurements.lance/data/");
    let refresh = {
        let writer = device.writer.clone();
        let table = table.clone();
        tokio::spawn(async move { writer.refresh_table(&table).await })
    };
    let store = cloud.store.clone();
    eventually(move || {
        store.gets.lock().unwrap().iter().any(|(key, kind)| {
            *kind == Get::Whole && key.contains("/data/") && !before.contains(key)
        })
    })
    .await?;
    device.writer.remove_table(&other).await?;
    cloud.store.release();
    assert!(matches!(refresh.await??, RefreshOutcome::Refreshed { .. }));
    let (name, _, _) = device.writer.queue.local_view(&resource_key(&table))?;
    assert!(
        device
            .writer
            .queue
            .snapshot(&name.context("local name")?)?
            .is_some(),
        "the refreshed snapshot kept its recorded cloud location"
    );
    assert_eq!(device.ids(&table, "id = 350").await?, vec![350]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refresh_waiters_are_answered_when_the_mirror_task_ends() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    indexed(&cloud, "readings").await?;
    let device = Device::open(&cloud).await?;
    let (table, other) = (selection("measurements"), selection("readings"));
    device.add(&table, setup(false)).await?;
    device.add(&other, setup(false)).await?;
    device.writer.spawn_mirror();
    let held = device.host.hold.lock().await;
    let refresh = |table: &BufferedTable| {
        let writer = device.writer.clone();
        let table = table.clone();
        tokio::spawn(async move { writer.refresh_table(&table).await })
    };
    let (running, queued) = (refresh(&table), refresh(&other));
    for _ in 0..2 {
        eventually(|| device.writer.queued_jobs() == 1).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        device.writer.queued_jobs(),
        1,
        "one refresh runs and one waits"
    );
    device.host.panic_remote.store(true, Ordering::SeqCst);
    drop(held);
    for waiter in [running, queued] {
        assert!(
            tokio::time::timeout(Duration::from_secs(10), waiter)
                .await??
                .is_err()
        );
    }
    assert_eq!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Unchanged,
        "later refreshes run inline"
    );
    let overlay = device.overlay(&table)?;
    device.writer.close().await?;
    let closed = tokio::time::timeout(
        Duration::from_secs(10),
        device.writer.refresh_lazy(&overlay),
    )
    .await?
    .unwrap_err();
    assert_eq!(
        closed.to_string(),
        "Offline changes for this project were removed from this device"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_everything_is_recorded_with_the_resource_row() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let mut device = Device::open(&cloud).await?;
    let table = selection("measurements");
    assert!(device.add(&table, setup(true)).await?.prefetch);
    device.reopen(|options| options).await?;
    assert!(device.add(&table, setup(false)).await?.prefetch);
    let (prefetched, plain) = (
        resource_key(&selection("prefetched")),
        resource_key(&selection("plain")),
    );
    let queue = &device.writer.queue;
    queue.initialize_table(
        &prefetched,
        &json!({"version": 1}),
        Some(1),
        Some("a"),
        true,
    )?;
    queue.initialize_resource(&plain, &json!({"version": 1}), Some(1))?;
    assert!(queue.prefetch(&prefetched)? && !queue.prefetch(&plain)?);
    Ok(())
}

async fn read_stale(device: &Device, table: &BufferedTable, key: &str) -> Result<()> {
    assert_eq!(
        device.ids(table, "id = 250").await.unwrap_err().to_string(),
        E37
    );
    assert_eq!(device.mirror().stale_keys(), vec![key.to_string()]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_keys_clear_when_the_refresh_finds_no_change_or_the_table_is_removed() -> Result<()> {
    let cloud = Cloud::project().await?;
    indexed(&cloud, "measurements").await?;
    let device = Device::open(&cloud).await?;
    let table = selection("measurements");
    device.add(&table, setup(false)).await?;
    let data = device.data(&table)?;
    std::fs::remove_file(cloud.file(&data[2]))?;
    read_stale(&device, &table, &data[2]).await?;
    assert_eq!(
        device.writer.refresh_table(&table).await?,
        RefreshOutcome::Unchanged
    );
    assert!(device.mirror().stale_keys().is_empty());
    device.writer.idle_pass().await;
    assert_eq!(
        device.writer.queued_jobs(),
        0,
        "no stale refresh is queued again"
    );
    read_stale(&device, &table, &data[2]).await?;
    device.writer.remove_table(&table).await?;
    assert!(
        device.mirror().stale_keys().is_empty(),
        "no current snapshot lists the key"
    );
    Ok(())
}
