//! Persistent ranges of immutable Lance files. Mutable manifests and listings always
//! reach storage, so this cache does not choose a table's current version.

use crate::credentials::RenewableCredentials;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use object_store::{
    Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
    path::Path,
};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex, Weak},
};
use tokio::sync::OnceCell;

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;
const MAX_METADATA: usize = 4096;
const CHECKSUM_BYTES: u64 = 32;
static CACHES: LazyLock<Mutex<HashMap<PathBuf, Weak<LanceRangeCache>>>> =
    LazyLock::new(Mutex::default);

#[derive(Debug)]
struct DiskEntry {
    bytes: u64,
    used: u64,
}

#[derive(Debug, Default)]
struct DiskState {
    entries: HashMap<String, DiskEntry>,
    bytes: u64,
    clock: u64,
}

/// A shared disk budget across account and project namespaces. The host must create
/// a private directory before opening it. Cache I/O failure never prevents a query.
#[derive(Debug)]
pub struct LanceRangeCache {
    directory: PathBuf,
    limit: u64,
    state: Mutex<DiskState>,
}

impl LanceRangeCache {
    pub fn open(directory: PathBuf) -> std::io::Result<Arc<Self>> {
        let mut caches = CACHES.lock().unwrap();
        if let Some(cache) = caches.get(&directory).and_then(Weak::upgrade) {
            return Ok(cache);
        }
        caches.retain(|_, cache| cache.strong_count() > 0);
        let cache = Self::open_with_limit(directory.clone(), MAX_BYTES)?;
        caches.insert(directory, Arc::downgrade(&cache));
        Ok(cache)
    }

    fn open_with_limit(directory: PathBuf, limit: u64) -> std::io::Result<Arc<Self>> {
        let mut state = DiskState::default();
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !cache_filename(&name) {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path())?;
            if !meta.is_file() || meta.len() > MAX_RANGE_BYTES + CHECKSUM_BYTES {
                continue;
            }
            entries.push((meta.modified().ok(), name, meta.len()));
        }
        entries.sort_unstable();
        for (_, name, bytes) in entries {
            state.clock += 1;
            state.entries.insert(
                name,
                DiskEntry {
                    bytes,
                    used: state.clock,
                },
            );
            state.bytes += bytes;
        }
        let cache = Arc::new(Self {
            directory,
            limit,
            state: Mutex::new(state),
        });
        cache.make_room(&mut cache.state.lock().unwrap(), 0);
        Ok(cache)
    }

    fn make_room(&self, state: &mut DiskState, bytes: u64) {
        while state.bytes.saturating_add(bytes) > self.limit || state.entries.len() >= MAX_ENTRIES {
            let Some(name) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(name, _)| name.clone())
            else {
                break;
            };
            if let Some(entry) = state.entries.remove(&name) {
                state.bytes = state.bytes.saturating_sub(entry.bytes);
                let _ = std::fs::remove_file(self.directory.join(name));
            }
        }
    }

    async fn get(self: &Arc<Self>, key: String) -> Option<Bytes> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = cache.state.lock().unwrap();
            let entry = state.entries.get(&key)?;
            let path = cache.directory.join(&key);
            let valid = || -> Option<Bytes> {
                let meta = std::fs::symlink_metadata(&path).ok()?;
                if !meta.is_file() || meta.len() != entry.bytes {
                    return None;
                }
                let mut file = std::fs::File::open(&path).ok()?;
                let mut bytes = Vec::with_capacity(entry.bytes as usize);
                file.read_to_end(&mut bytes).ok()?;
                if bytes.len() < CHECKSUM_BYTES as usize
                    || blake3::hash(&bytes[32..]).as_bytes() != &bytes[..32]
                {
                    return None;
                }
                Some(Bytes::from(bytes).slice(32..))
            }();
            if valid.is_some() {
                state.clock += 1;
                let used = state.clock;
                state.entries.get_mut(&key).unwrap().used = used;
            } else if let Some(entry) = state.entries.remove(&key) {
                state.bytes = state.bytes.saturating_sub(entry.bytes);
                let _ = std::fs::remove_file(path);
            }
            valid
        })
        .await
        .ok()
        .flatten()
    }

    async fn put(self: &Arc<Self>, key: String, bytes: Bytes) {
        if bytes.len() as u64 > MAX_RANGE_BYTES || bytes.len() as u64 + CHECKSUM_BYTES > self.limit
        {
            return;
        }
        let cache = self.clone();
        let _ = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut state = cache.state.lock().unwrap();
            if state.entries.contains_key(&key) {
                return Ok(());
            }
            let length = bytes.len() as u64 + CHECKSUM_BYTES;
            cache.make_room(&mut state, length);
            let mut file = tempfile::NamedTempFile::new_in(&cache.directory)?;
            file.write_all(blake3::hash(&bytes).as_bytes())?;
            file.write_all(&bytes)?;
            file.persist(cache.directory.join(&key))
                .map_err(|error| error.error)?;
            state.clock += 1;
            let used = state.clock;
            state.entries.insert(
                key,
                DiskEntry {
                    bytes: length,
                    used,
                },
            );
            state.bytes += length;
            Ok(())
        })
        .await;
    }
}

fn cache_filename(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn uuid_name(name: &str) -> bool {
    name.len() == 36
        && name.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn data_name(name: &str) -> bool {
    let Some(name) = name.strip_suffix(".lance") else {
        return false;
    };
    uuid_name(name)
        || (name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
        // Lance's current filename encodes a random UUID as 24 binary + 26 hex digits.
        || (name.is_ascii() && name.len() == 50 && name[..24].bytes().all(|byte| matches!(byte, b'0' | b'1'))
            && name[24..].bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn immutable_file(path: &Path) -> bool {
    let parts: Vec<_> = path.as_ref().split('/').collect();
    parts.windows(2).enumerate().any(|(index, pair)| {
        if !pair[0].ends_with(".lance") {
            return false;
        }
        let tail = &parts[index + 1..];
        match tail {
            ["data", name] => data_name(name),
            ["_indices", uuid, file] => uuid_name(uuid) && file.ends_with(".lance"),
            _ => false,
        }
    })
}

#[derive(Clone, Debug)]
struct ReadMetadata {
    meta: ObjectMeta,
    attributes: Attributes,
}

#[derive(Debug, Default)]
struct ReadState {
    metadata: HashMap<Path, Arc<OnceCell<ReadMetadata>>>,
    // Mutation targets stay uncached for this store's lifetime, including while a
    // multipart upload or an earlier read is in flight.
    written: HashSet<Path>,
    disabled: bool,
}

/// Only UUID-addressed Lance data/index files use this cache. Their first access
/// in this credential lease validates an ETag with HEAD; subsequent ranges reuse
/// it because committed Lance files are immutable. A new lease validates again.
/// Manual object replacement outside Lance is outside this immutability contract.
#[derive(Clone, Debug)]
pub struct CachedLanceStore {
    inner: Arc<dyn ObjectStore>,
    cache: Arc<LanceRangeCache>,
    namespace: String,
    credentials: RenewableCredentials,
    reads: Arc<Mutex<ReadState>>,
}

impl CachedLanceStore {
    pub fn new(
        inner: Arc<dyn ObjectStore>,
        cache: Arc<LanceRangeCache>,
        namespace: String,
        credentials: RenewableCredentials,
    ) -> Self {
        Self {
            inner,
            cache,
            namespace,
            credentials,
            reads: Arc::default(),
        }
    }

    async fn authorize(&self) -> object_store::Result<()> {
        self.credentials
            .credential()
            .await
            .map(|_| ())
            .map_err(|error| object_store::Error::Generic {
                store: "LanceReadCache",
                source: Box::new(error),
            })
    }

    fn writing(&self, path: &Path) {
        let mut reads = self.reads.lock().unwrap();
        reads.metadata.remove(path);
        if reads.written.len() >= MAX_METADATA {
            reads.disabled = true;
            reads.metadata.clear();
        } else {
            reads.written.insert(path.clone());
        }
    }

    fn metadata_cell(&self, path: &Path) -> Option<Arc<OnceCell<ReadMetadata>>> {
        let mut reads = self.reads.lock().unwrap();
        if reads.disabled || reads.written.contains(path) {
            return None;
        }
        if reads.metadata.len() >= MAX_METADATA {
            reads.metadata.clear();
        }
        Some(reads.metadata.entry(path.clone()).or_default().clone())
    }

    fn can_cache(&self, path: &Path) -> bool {
        let reads = self.reads.lock().unwrap();
        !reads.disabled && !reads.written.contains(path)
    }
}

impl std::fmt::Display for CachedLanceStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "CachedLanceStore({})", self.inner)
    }
}

#[async_trait]
impl ObjectStore for CachedLanceStore {
    async fn get_opts(&self, path: &Path, options: GetOptions) -> object_store::Result<GetResult> {
        if !immutable_file(path)
            || options.version.is_some()
            || options.if_match.is_some()
            || options.if_none_match.is_some()
            || options.if_modified_since.is_some()
            || options.if_unmodified_since.is_some()
            || !options.extensions.is_empty()
        {
            return self.inner.get_opts(path, options).await;
        }
        let Some(cell) = self.metadata_cell(path) else {
            return self.inner.get_opts(path, options).await;
        };
        self.authorize().await?;
        let metadata = cell
            .get_or_try_init(|| async {
                let result = self
                    .inner
                    .get_opts(
                        path,
                        GetOptions {
                            head: true,
                            ..Default::default()
                        },
                    )
                    .await;
                let result = match result {
                    Ok(result) => result,
                    Err(object_store::Error::Precondition { .. }) => {
                        self.reads.lock().unwrap().metadata.remove(path);
                        return self.inner.get_opts(path, original_options).await;
                    }
                    Err(error) => return Err(error),
                };
                Ok::<_, object_store::Error>(ReadMetadata {
                    meta: result.meta,
                    attributes: result.attributes,
                })
            })
            .await?;
        if !self.can_cache(path) {
            return self.inner.get_opts(path, options).await;
        }
        let Some(etag) = metadata
            .meta
            .e_tag
            .as_deref()
            .filter(|etag| !etag.is_empty())
        else {
            return self.inner.get_opts(path, options).await;
        };
        let range = match &options.range {
            Some(range) => match range.as_range(metadata.meta.size) {
                Ok(range) => range,
                Err(_) => return self.inner.get_opts(path, options).await,
            },
            None => 0..metadata.meta.size,
        };
        if options.head {
            self.authorize().await?;
            return Ok(GetResult {
                payload: GetResultPayload::Stream(futures::stream::empty().boxed()),
                meta: metadata.meta.clone(),
                range: 0..0,
                attributes: metadata.attributes.clone(),
            });
        }
        if range.end - range.start > MAX_RANGE_BYTES {
            return self.inner.get_opts(path, options).await;
        }
        let mut hash = blake3::Hasher::new();
        for value in [self.namespace.as_str(), path.as_ref(), etag] {
            hash.update(&(value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
        hash.update(&metadata.meta.size.to_le_bytes());
        hash.update(&range.start.to_le_bytes());
        hash.update(&range.end.to_le_bytes());
        let key = hash.finalize().to_hex().to_string();
        let original_options = options.clone();
        let bytes = match self.cache.get(key.clone()).await {
            Some(bytes) if bytes.len() as u64 == range.end - range.start => bytes,
            _ => {
                // The conditional GET cannot populate this ETag's entry with bytes
                // from an object replaced between HEAD and GET.
                let result = self
                    .inner
                    .get_opts(
                        path,
                        GetOptions {
                            if_match: Some(etag.to_owned()),
                            ..options
                        },
                    )
                    .await?;
                let matches = result.meta.e_tag == metadata.meta.e_tag && result.range == range;
                if !matches {
                    return Ok(result);
                }
                let bytes = result.bytes().await?;
                if self.can_cache(path) && bytes.len() as u64 == range.end - range.start {
                    self.cache.put(key, bytes.clone()).await;
                }
                bytes
            }
        };
        if !self.can_cache(path) {
            return self.inner.get_opts(path, original_options).await;
        }
        self.authorize().await?;
        Ok(GetResult {
            payload: GetResultPayload::Stream(
                futures::stream::once(async move { Ok(bytes) }).boxed(),
            ),
            meta: metadata.meta.clone(),
            range,
            attributes: metadata.attributes.clone(),
        })
    }

    async fn put_opts(
        &self,
        path: &Path,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.writing(path);
        self.inner.put_opts(path, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        path: &Path,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.writing(path);
        self.inner.put_multipart_opts(path, options).await
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        let this = self.clone();
        self.inner.delete_stream(
            paths
                .map(move |path| {
                    if let Ok(path) = &path {
                        this.writing(path);
                    }
                    path
                })
                .boxed(),
        )
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }
    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list_with_offset(prefix, offset)
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
        self.writing(to);
        self.inner.copy_opts(from, to, options).await
    }
    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.writing(from);
        self.writing(to);
        self.inner.rename_opts(from, to, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{
        StorageCredential, StorageCredentialLease, StorageCredentialProvider,
        StorageCredentialScope,
    };
    use flow_like_types_contracts::authorization::AuthorizationError;
    use object_store::{ObjectStoreExt, memory::InMemory};
    use std::{
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::{Duration, SystemTime},
    };

    const FILE: &str = "apps/project/storage/db/orders.lance/data/110111111101011011100010b888614a979b814629954cf60b.lance";

    struct Provider {
        scope: StorageCredentialScope,
        denied: AtomicBool,
    }

    #[async_trait]
    impl StorageCredentialProvider for Provider {
        fn scope(&self) -> &StorageCredentialScope {
            &self.scope
        }
        async fn credential(&self) -> Result<StorageCredentialLease, AuthorizationError> {
            if self.denied.load(Ordering::SeqCst) {
                return Err(AuthorizationError::Denied);
            }
            Ok(StorageCredentialLease {
                scope: self.scope.clone(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                credential: StorageCredential::GcpBearer("test".into()),
            })
        }
    }

    #[derive(Debug, Default)]
    struct Cloud {
        store: InMemory,
        heads: AtomicUsize,
        gets: AtomicUsize,
        pause: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
    }

    impl std::fmt::Display for Cloud {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("test cloud")
        }
    }

    #[async_trait]
    impl ObjectStore for Cloud {
        async fn get_opts(&self, path: &Path, opts: GetOptions) -> object_store::Result<GetResult> {
            let pause = if opts.head {
                self.heads.fetch_add(1, Ordering::SeqCst);
                None
            } else {
                self.gets.fetch_add(1, Ordering::SeqCst);
                self.pause.lock().unwrap().take()
            };
            let result = self.store.get_opts(path, opts).await;
            if let Some((entered, resume)) = pause {
                entered.notify_one();
                resume.notified().await;
            }
            result
        }
        async fn put_opts(
            &self,
            path: &Path,
            value: PutPayload,
            opts: PutOptions,
        ) -> object_store::Result<PutResult> {
            self.store.put_opts(path, value, opts).await
        }
        async fn put_multipart_opts(
            &self,
            path: &Path,
            opts: PutMultipartOptions,
        ) -> object_store::Result<Box<dyn MultipartUpload>> {
            self.store.put_multipart_opts(path, opts).await
        }
        fn delete_stream(
            &self,
            paths: BoxStream<'static, object_store::Result<Path>>,
        ) -> BoxStream<'static, object_store::Result<Path>> {
            self.store.delete_stream(paths)
        }
        fn list(
            &self,
            prefix: Option<&Path>,
        ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            self.store.list(prefix)
        }
        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            self.store.list_with_delimiter(prefix).await
        }
        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            opts: CopyOptions,
        ) -> object_store::Result<()> {
            self.store.copy_opts(from, to, opts).await
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        Arc<Cloud>,
        Arc<Provider>,
        CachedLanceStore,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let provider = Arc::new(Provider {
            scope: StorageCredentialScope {
                instance_id: "i".into(),
                project_id: "p".into(),
                placement_id: "desktop".into(),
                grant_id: "g".into(),
                resource: "content".into(),
            },
            denied: AtomicBool::new(false),
        });
        let store = CachedLanceStore::new(
            cloud.clone(),
            LanceRangeCache::open(directory.path().into()).unwrap(),
            "account/project/bucket".into(),
            RenewableCredentials::new(provider.clone()).unwrap(),
        );
        (directory, cloud, provider, store)
    }

    #[test]
    fn only_immutable_lance_layouts_are_cacheable() {
        for path in [
            FILE,
            "db/t.lance/data/05fab9e5-2a0b-427f-a8cb-ac02a7d5589e.lance",
            "db/t.lance/_indices/05fab9e5-2a0b-427f-a8cb-ac02a7d5589e/page_data.lance",
        ] {
            assert!(immutable_file(&Path::from(path)), "{path}");
        }
        for path in [
            "db/t.lance/_latest.manifest",
            "db/t.lance/_versions/1.manifest",
            "db/t.lance/data/current.lance",
            "db/t.lance/_indices/current/page_data.lance",
            "db/t.lance/_indices/05fab9e5-2a0b-427f-a8cb-ac02a7d5589e/nested/file.lance",
            "db/t.lance/data/ééééééééééééééééééééééééé.lance",
        ] {
            assert!(!immutable_file(&Path::from(path)), "{path}");
        }
    }

    #[tokio::test]
    async fn repeated_ranges_avoid_cloud_and_survive_reopening_with_etag_validation() {
        let (directory, cloud, provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        for _ in 0..2 {
            assert_eq!(store.get_range(&path, 1..4).await.unwrap(), "bcd");
        }
        assert_eq!(cloud.heads.load(Ordering::SeqCst), 1);
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        drop(store);
        let reopened = CachedLanceStore::new(
            cloud.clone(),
            LanceRangeCache::open(directory.path().into()).unwrap(),
            "account/project/bucket".into(),
            RenewableCredentials::new(provider).unwrap(),
        );
        assert_eq!(reopened.get_range(&path, 1..4).await.unwrap(), "bcd");
        assert_eq!(cloud.heads.load(Ordering::SeqCst), 2);
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        cloud.put(&path, "uvwxyz".into()).await.unwrap();
        let newer = CachedLanceStore::new(
            cloud.clone(),
            reopened.cache.clone(),
            reopened.namespace.clone(),
            reopened.credentials.clone(),
        );
        assert_eq!(newer.get_range(&path, 1..4).await.unwrap(), "vwx");
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn account_namespace_and_ranges_do_not_alias() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "abc");
        assert_eq!(store.get_range(&path, 3..6).await.unwrap(), "def");
        let other = CachedLanceStore::new(
            cloud.clone(),
            store.cache.clone(),
            "other-account".into(),
            store.credentials.clone(),
        );
        assert_eq!(other.get_range(&path, 0..3).await.unwrap(), "abc");
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn mutable_manifests_and_conditional_requests_always_reach_storage() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from("db/t.lance/_versions/1.manifest");
        cloud.put(&path, "first".into()).await.unwrap();
        assert_eq!(
            store.get(&path).await.unwrap().bytes().await.unwrap(),
            "first"
        );
        cloud.put(&path, "second".into()).await.unwrap();
        assert_eq!(
            store.get(&path).await.unwrap().bytes().await.unwrap(),
            "second"
        );
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        store.get_range(&path, 0..3).await.unwrap();
        assert!(
            store
                .get_opts(
                    &path,
                    GetOptions {
                        if_match: Some("wrong".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn cached_bytes_do_not_survive_revocation_or_local_mutation() {
        let (_directory, cloud, provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        store.get_range(&path, 0..3).await.unwrap();
        provider.denied.store(true, Ordering::SeqCst);
        assert!(store.get_range(&path, 0..3).await.is_err());
        provider.denied.store(false, Ordering::SeqCst);
        store.put(&path, "uvwxyz".into()).await.unwrap();
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "uvw");
        store.delete(&path).await.unwrap();
        assert!(store.get_range(&path, 0..3).await.is_err());
    }

    #[tokio::test]
    async fn mutation_during_a_read_cannot_publish_or_return_old_cached_bytes() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        *cloud.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
        let reading = store.clone();
        let read_path = path.clone();
        let read = tokio::spawn(async move { reading.get_range(&read_path, 0..3).await });
        entered.notified().await;
        store.put(&path, "uvwxyz".into()).await.unwrap();
        resume.notify_one();
        assert_eq!(read.await.unwrap().unwrap(), "uvw");
        assert!(store.cache.state.lock().unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn corrupt_or_unavailable_disk_cache_falls_back_to_cloud() {
        let (directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        store.get_range(&path, 0..3).await.unwrap();
        let entry = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(entry, [0_u8; 35]).unwrap();
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "abc");
        std::fs::remove_dir_all(directory.path()).unwrap();
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "abc");
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn disk_budget_evicts_oldest_ranges_and_remains_bounded_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let cache = LanceRangeCache::open_with_limit(directory.path().into(), 70).unwrap();
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        cache.put(a.clone(), Bytes::from_static(b"abc")).await;
        cache.put(b.clone(), Bytes::from_static(b"def")).await;
        assert!(cache.get(a.clone()).await.is_some());
        cache.put(c.clone(), Bytes::from_static(b"ghi")).await;
        assert!(cache.get(b).await.is_none());
        assert!(cache.get(a).await.is_some());
        assert!(cache.get(c).await.is_some());
        assert_eq!(cache.state.lock().unwrap().bytes, 70);
        let smaller = LanceRangeCache::open_with_limit(directory.path().into(), 35).unwrap();
        assert_eq!(smaller.state.lock().unwrap().entries.len(), 1);
        assert_eq!(smaller.state.lock().unwrap().bytes, 35);
    }
}
