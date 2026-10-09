//! Persistent blocks of immutable Lance files. Mutable manifests and listings always
//! reach storage, so this cache does not choose a table's current version.

use crate::credentials::RenewableCredentials;
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures::{StreamExt, stream::BoxStream};
use object_store::{
    Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
    path::Path,
};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{OnceCell, Semaphore};

const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
// Complete reads fetch less than two blocks beyond their requested range. Groups
// coalesce misses without fetching cached blocks or unrequested interior data.
const BLOCK_BYTES: u64 = 256 * 1024;
const GROUP_BYTES: u64 = 4 * 1024 * 1024;
const READ_AHEAD_GROUPS: usize = 2;
const MAX_CLOUD_REQUESTS: usize = 8;
const MAX_ENTRIES: usize = 16384;
const MAX_METADATA: usize = 4096;
const CHECKSUM_BYTES: u64 = 32;
const TEMP_PREFIX: &str = ".lance-range-";
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
    disabled: bool,
}

/// Cumulative counters shared by namespaces using the same disk cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LanceCacheStats {
    pub block_hits: u64,
    pub block_misses: u64,
    pub bypassed_requests: u64,
    /// Bytes read from disk, including alignment and read-ahead bytes.
    pub disk_bytes: u64,
    /// Bytes received while filling blocks, including alignment and read-ahead.
    pub downloaded_bytes: u64,
    /// Requested bytes emitted by cache-backed streams, including cold misses.
    pub served_bytes: u64,
}

#[derive(Debug, Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    bypasses: AtomicU64,
    disk: AtomicU64,
    downloaded: AtomicU64,
    served: AtomicU64,
}

/// A shared disk budget across account and project namespaces. The host must create
/// a private directory before opening it. Cache I/O failure never prevents a query.
#[derive(Debug)]
pub struct LanceRangeCache {
    directory: PathBuf,
    limit: u64,
    state: Mutex<DiskState>,
    fills: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    downloads: Semaphore,
    counters: Counters,
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
            if name.starts_with(TEMP_PREFIX) {
                std::fs::remove_file(entry.path())?;
                continue;
            }
            if !cache_filename(&name) {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path())?;
            if !meta.is_file() || meta.len() > BLOCK_BYTES + CHECKSUM_BYTES {
                std::fs::remove_file(entry.path())?;
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
            fills: Mutex::default(),
            downloads: Semaphore::new(MAX_CLOUD_REQUESTS),
            counters: Counters::default(),
        });
        cache.make_room(&mut cache.state.lock().unwrap(), 0)?;
        Ok(cache)
    }

    fn make_room(&self, state: &mut DiskState, bytes: u64) -> std::io::Result<()> {
        while state.bytes.saturating_add(bytes) > self.limit || state.entries.len() >= MAX_ENTRIES {
            let Some(name) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(name, _)| name.clone())
            else {
                break;
            };
            match std::fs::remove_file(self.directory.join(&name)) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            if let Some(entry) = state.entries.remove(&name) {
                state.bytes = state.bytes.saturating_sub(entry.bytes);
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> LanceCacheStats {
        LanceCacheStats {
            block_hits: self.counters.hits.load(Ordering::Relaxed),
            block_misses: self.counters.misses.load(Ordering::Relaxed),
            bypassed_requests: self.counters.bypasses.load(Ordering::Relaxed),
            disk_bytes: self.counters.disk.load(Ordering::Relaxed),
            downloaded_bytes: self.counters.downloaded.load(Ordering::Relaxed),
            served_bytes: self.counters.served.load(Ordering::Relaxed),
        }
    }

    fn fill_lock(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut fills = self.fills.lock().unwrap();
        if let Some(lock) = fills.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        if fills.len() >= MAX_METADATA {
            fills.retain(|_, lock| lock.strong_count() > 0);
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        fills.insert(key, Arc::downgrade(&lock));
        lock
    }

    fn read_block(&self, state: &mut DiskState, key: &str) -> Option<Bytes> {
        if state.disabled {
            return None;
        }
        let entry = state.entries.get(key)?;
        let path = self.directory.join(key);
        let valid = || -> Option<Bytes> {
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() != entry.bytes {
                return None;
            }
            let file = std::fs::File::open(&path).ok()?;
            let mut bytes = Vec::with_capacity(entry.bytes as usize);
            file.take(entry.bytes + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() as u64 != entry.bytes
                || bytes.len() < CHECKSUM_BYTES as usize
                || blake3::hash(&bytes[32..]).as_bytes() != &bytes[..32]
            {
                return None;
            }
            Some(Bytes::from(bytes).slice(32..))
        }();
        if valid.is_some() {
            state.clock += 1;
            let used = state.clock;
            state.entries.get_mut(key).unwrap().used = used;
        } else if let Some(entry) = state.entries.remove(key) {
            state.bytes = state.bytes.saturating_sub(entry.bytes);
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                state.disabled = true;
            }
        }
        valid
    }

    async fn get_many(self: &Arc<Self>, keys: Vec<String>) -> Vec<Option<Bytes>> {
        let cache = self.clone();
        let count = keys.len();
        let blocks = tokio::task::spawn_blocking(move || {
            let mut state = cache.state.lock().unwrap();
            keys.iter()
                .map(|key| cache.read_block(&mut state, key))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_else(|_| vec![None; count]);
        let hits = blocks.iter().filter(|block| block.is_some()).count() as u64;
        self.counters.hits.fetch_add(hits, Ordering::Relaxed);
        self.counters
            .misses
            .fetch_add(count as u64 - hits, Ordering::Relaxed);
        self.counters.disk.fetch_add(
            blocks
                .iter()
                .flatten()
                .map(|block| block.len() as u64)
                .sum::<u64>(),
            Ordering::Relaxed,
        );
        blocks
    }

    async fn put_many(self: &Arc<Self>, blocks: Vec<(String, Bytes)>) {
        let cache = self.clone();
        let _ = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut state = cache.state.lock().unwrap();
            for (key, bytes) in blocks {
                if state.disabled || state.entries.contains_key(&key) {
                    continue;
                }
                let length = bytes.len() as u64 + CHECKSUM_BYTES;
                if bytes.len() as u64 > BLOCK_BYTES || length > cache.limit {
                    continue;
                }
                cache.make_room(&mut state, length)?;
                let mut file = tempfile::Builder::new()
                    .prefix(TEMP_PREFIX)
                    .tempfile_in(&cache.directory)?;
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
            }
            Ok(())
        })
        .await;
    }

    #[cfg(test)]
    async fn get(self: &Arc<Self>, key: String) -> Option<Bytes> {
        self.get_many(vec![key]).await.pop().flatten()
    }

    #[cfg(test)]
    async fn put(self: &Arc<Self>, key: String, bytes: Bytes) {
        self.put_many(vec![(key, bytes)]).await;
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
/// through this scoped store validates an ETag with HEAD; subsequent ranges reuse
/// it because committed Lance files are immutable. Desktop retains this store
/// across runs on one renewable authority. Credential refresh keeps that store;
/// constructing a new store validates again, including after a process restart.
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

    async fn uncached(&self, path: &Path, options: GetOptions) -> object_store::Result<GetResult> {
        self.cache.counters.bypasses.fetch_add(1, Ordering::Relaxed);
        self.inner.get_opts(path, options).await
    }

    fn changed(&self, path: &Path) -> object_store::Error {
        self.reads.lock().unwrap().metadata.remove(path);
        object_store::Error::Precondition {
            path: path.to_string(),
            source: "The Lance object changed while its blocks were being read".into(),
        }
    }

    async fn check_read(&self, path: &Path) -> object_store::Result<()> {
        self.authorize().await?;
        if !self.can_cache(path) {
            return Err(self.changed(path));
        }
        Ok(())
    }

    fn block_key(&self, path: &Path, metadata: &ReadMetadata, range: &Range<u64>) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(b"lance-block-v1");
        for value in [
            self.namespace.as_str(),
            path.as_ref(),
            metadata.meta.e_tag.as_deref().unwrap(),
        ] {
            hash.update(&(value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
        hash.update(&metadata.meta.size.to_le_bytes());
        hash.update(&range.start.to_le_bytes());
        hash.update(&range.end.to_le_bytes());
        hash.finalize().to_hex().to_string()
    }

    /// A group covers at most 4 MiB and shares a fill lock with overlapping reads.
    /// Only requested blocks are downloaded; consecutive misses share one GET.
    async fn read_group(
        &self,
        path: &Path,
        metadata: &ReadMetadata,
        requested: Range<u64>,
    ) -> object_store::Result<Bytes> {
        self.check_read(path).await?;
        if requested.is_empty() {
            return Ok(Bytes::new());
        }
        let size = metadata.meta.size;
        let group_start = requested.start / GROUP_BYTES * GROUP_BYTES;
        let group = group_start..group_start.saturating_add(GROUP_BYTES).min(size);
        let lock = self.cache.fill_lock(self.block_key(path, metadata, &group));
        let _guard = lock.lock().await;
        self.check_read(path).await?;
        let start = requested.start / BLOCK_BYTES * BLOCK_BYTES;
        let end = requested
            .end
            .saturating_add((BLOCK_BYTES - requested.end % BLOCK_BYTES) % BLOCK_BYTES)
            .min(size);
        let ranges: Vec<_> = (start..end)
            .step_by(BLOCK_BYTES as usize)
            .map(|start| start..start.saturating_add(BLOCK_BYTES).min(size))
            .collect();
        let keys: Vec<_> = ranges
            .iter()
            .map(|range| self.block_key(path, metadata, range))
            .collect();
        let mut blocks = self.cache.get_many(keys.clone()).await;
        for (block, range) in blocks.iter_mut().zip(&ranges) {
            if block
                .as_ref()
                .is_some_and(|bytes| bytes.len() as u64 != range.end - range.start)
            {
                *block = None;
            }
        }
        let mut index = 0;
        while index < ranges.len() {
            if blocks[index].is_some() {
                index += 1;
                continue;
            }
            let first = index;
            while index < ranges.len() && blocks[index].is_none() {
                index += 1;
            }
            let missing = ranges[first].start..ranges[index - 1].end;
            let _permit = self.cache.downloads.acquire().await.map_err(|error| {
                object_store::Error::Generic {
                    store: "LanceReadCache",
                    source: Box::new(error),
                }
            })?;
            self.check_read(path).await?;
            let result = self
                .inner
                .get_opts(
                    path,
                    GetOptions {
                        range: Some(missing.clone().into()),
                        if_match: metadata.meta.e_tag.clone(),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| {
                    if matches!(error, object_store::Error::Precondition { .. }) {
                        self.reads.lock().unwrap().metadata.remove(path);
                    }
                    error
                })?;
            if result.meta.e_tag != metadata.meta.e_tag || result.meta.size != size {
                return Err(self.changed(path));
            }
            if result.range != missing {
                return Err(invalid_response(
                    path,
                    "Cloud storage returned an unexpected byte range",
                ));
            }
            let expected = (missing.end - missing.start) as usize;
            let mut buffer = BytesMut::with_capacity(expected);
            let mut payload = result.into_stream();
            while let Some(chunk) = payload.next().await {
                let chunk = chunk?;
                self.cache
                    .counters
                    .downloaded
                    .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                if chunk.len() > expected - buffer.len() {
                    return Err(invalid_response(
                        path,
                        "Cloud storage returned too many bytes",
                    ));
                }
                buffer.extend_from_slice(&chunk);
            }
            if buffer.len() != expected {
                return Err(invalid_response(
                    path,
                    "Cloud storage returned an incomplete byte range",
                ));
            }
            self.check_read(path).await?;
            let bytes = buffer.freeze();
            let mut downloaded = Vec::with_capacity(index - first);
            for block in first..index {
                let range = &ranges[block];
                let bytes = bytes.slice(
                    (range.start - missing.start) as usize..(range.end - missing.start) as usize,
                );
                blocks[block] = Some(bytes.clone());
                downloaded.push((keys[block].clone(), bytes));
            }
            // Keep the fill slot until disk persistence finishes, so a slow disk
            // cannot accumulate an unbounded queue of downloaded groups.
            self.cache.put_many(downloaded).await;
        }
        self.check_read(path).await?;
        if blocks.len() == 1 {
            return Ok(blocks
                .pop()
                .unwrap()
                .unwrap()
                .slice((requested.start - start) as usize..(requested.end - start) as usize));
        }
        let mut result = BytesMut::with_capacity((requested.end - requested.start) as usize);
        for (block, range) in blocks.into_iter().zip(ranges) {
            let from = requested.start.max(range.start) - range.start;
            let to = requested.end.min(range.end) - range.start;
            result.extend_from_slice(&block.unwrap()[from as usize..to as usize]);
        }
        Ok(result.freeze())
    }
}

fn invalid_response(path: &Path, message: &str) -> object_store::Error {
    object_store::Error::Generic {
        store: "LanceReadCache",
        source: format!("{message}: {path}").into(),
    }
}

fn group_end(start: u64, end: u64) -> u64 {
    start
        .saturating_add(GROUP_BYTES - start % GROUP_BYTES)
        .min(end)
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
            return self.uncached(path, options).await;
        }
        let Some(cell) = self.metadata_cell(path) else {
            return self.uncached(path, options).await;
        };
        self.authorize().await?;
        let metadata = cell
            .get_or_try_init(|| async {
                let _permit = self.cache.downloads.acquire().await.map_err(|error| {
                    object_store::Error::Generic {
                        store: "LanceReadCache",
                        source: Box::new(error),
                    }
                })?;
                self.authorize().await?;
                let result = self
                    .inner
                    .get_opts(
                        path,
                        GetOptions {
                            head: true,
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok::<_, object_store::Error>(ReadMetadata {
                    meta: result.meta,
                    attributes: result.attributes,
                })
            })
            .await?
            .clone();
        if !self.can_cache(path) {
            return self.uncached(path, options).await;
        }
        if !metadata
            .meta
            .e_tag
            .as_deref()
            .is_some_and(|etag| !etag.is_empty() && !etag.starts_with("W/"))
        {
            return self.uncached(path, options).await;
        }
        let range = match &options.range {
            Some(range) => match range.as_range(metadata.meta.size) {
                Ok(range) => range,
                Err(_) => return self.uncached(path, options).await,
            },
            None => 0..metadata.meta.size,
        };
        if options.head {
            self.authorize().await?;
            if !self.can_cache(path) {
                return self.uncached(path, options).await;
            }
            return Ok(GetResult {
                payload: GetResultPayload::Stream(futures::stream::empty().boxed()),
                meta: metadata.meta,
                range: 0..0,
                attributes: metadata.attributes,
            });
        }
        // Resolve the first group before returning metadata. A changed ETag can still
        // fall back to the original request here. Later changes fail the stream;
        // joining blocks from different object revisions would corrupt the result.
        let first_end = group_end(range.start, range.end);
        let first = match self
            .read_group(path, &metadata, range.start..first_end)
            .await
        {
            Ok(bytes) => bytes,
            Err(object_store::Error::Precondition { .. }) => {
                return self.uncached(path, options).await;
            }
            Err(error) => return Err(error),
        };
        self.authorize().await?;
        if !self.can_cache(path) {
            return self.uncached(path, options).await;
        }
        let authority = self.clone();
        let first_path = path.clone();
        let first = futures::stream::once(async move {
            authority.check_read(&first_path).await?;
            authority
                .cache
                .counters
                .served
                .fetch_add(first.len() as u64, Ordering::Relaxed);
            Ok(first)
        });
        let end = range.end;
        let groups = futures::stream::unfold(first_end, move |start| async move {
            (start < end).then(|| {
                let next = group_end(start, end);
                (start..next, next)
            })
        });
        let store = self.clone();
        let object = path.clone();
        let group_metadata = metadata.clone();
        // Two groups may read ahead. Metadata probes and block fills share eight
        // slots across this disk cache; bypassed reads keep backend scheduling.
        let remaining = groups
            .map(move |requested| {
                let store = store.clone();
                let path = object.clone();
                let metadata = group_metadata.clone();
                async move { store.read_group(&path, &metadata, requested).await }
            })
            .buffered(READ_AHEAD_GROUPS);
        let authority = self.clone();
        let object = path.clone();
        let remaining = remaining.then(move |bytes| {
            let authority = authority.clone();
            let path = object.clone();
            async move {
                let bytes = bytes?;
                authority.check_read(&path).await?;
                authority
                    .cache
                    .counters
                    .served
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                Ok(bytes)
            }
        });
        let stream =
            futures::stream::unfold(Some(first.chain(remaining).boxed()), |stream| async move {
                let mut stream = stream?;
                let result = stream.next().await?;
                // Drop any read-ahead futures immediately after an error.
                let stream = if result.is_err() { None } else { Some(stream) };
                Some((result, stream))
            })
            .boxed();
        Ok(GetResult {
            payload: GetResultPayload::Stream(stream),
            meta: metadata.meta,
            range,
            attributes: metadata.attributes,
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
        ranges: Mutex<Vec<Range<u64>>>,
        delay: AtomicBool,
        active: AtomicUsize,
        max_active: AtomicUsize,
        corrupt_payload: AtomicUsize,
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
            let head = opts.head;
            let pause = if head {
                self.heads.fetch_add(1, Ordering::SeqCst);
                None
            } else {
                self.gets.fetch_add(1, Ordering::SeqCst);
                self.pause.lock().unwrap().take()
            };
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _active = (!head).then(|| {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                Active(&self.active)
            });
            if !head && self.delay.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let mut result = self.store.get_opts(path, opts).await;
            if !head && let Ok(response) = &result {
                self.ranges.lock().unwrap().push(response.range.clone());
            }
            let corrupt = self.corrupt_payload.load(Ordering::SeqCst);
            if !head
                && corrupt > 0
                && let Ok(response) = result
            {
                let meta = response.meta.clone();
                let range = response.range.clone();
                let attributes = response.attributes.clone();
                let bytes = response.bytes().await?;
                let bytes = if corrupt == 1 {
                    bytes.slice(..bytes.len() - 1)
                } else {
                    let mut oversized = BytesMut::from(bytes.as_ref());
                    oversized.extend_from_slice(b"x");
                    oversized.freeze()
                };
                result = Ok(GetResult {
                    meta,
                    range,
                    attributes,
                    payload: GetResultPayload::Stream(
                        futures::stream::once(async move { Ok(bytes) }).boxed(),
                    ),
                });
            }
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
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 2);
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
        let pending = store
            .get_opts(
                &path,
                GetOptions {
                    range: Some((0..3).into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        provider.denied.store(true, Ordering::SeqCst);
        assert!(pending.bytes().await.is_err());
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
    async fn replacement_between_head_and_get_never_uses_the_old_etag_key() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        store.head(&path).await.unwrap();
        cloud.put(&path, "uvwxyz".into()).await.unwrap();
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "uvw");
        assert!(store.cache.state.lock().unwrap().entries.is_empty());
        assert_eq!(store.get_range(&path, 0..3).await.unwrap(), "uvw");
        assert_eq!(cloud.heads.load(Ordering::SeqCst), 2);
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 3);
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

    #[test]
    fn reopening_removes_interrupted_downloads_and_oversized_owned_entries() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = directory.path().join(format!("{TEMP_PREFIX}interrupted"));
        std::fs::write(&temporary, b"partial download").unwrap();
        let oversized = directory.path().join("d".repeat(64));
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(BLOCK_BYTES + CHECKSUM_BYTES + 1)
            .unwrap();
        LanceRangeCache::open(directory.path().into()).unwrap();
        assert!(!temporary.exists());
        assert!(!oversized.exists());
    }

    fn patterned(length: usize) -> Bytes {
        (0..length)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>()
            .into()
    }

    #[tokio::test]
    async fn overlapping_ranges_reuse_blocks_and_measure_alignment_cost() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        let data = patterned((3 * BLOCK_BYTES + 17) as usize);
        cloud.put(&path, data.clone().into()).await.unwrap();
        let requested = [
            BLOCK_BYTES - 3..BLOCK_BYTES + 5,
            BLOCK_BYTES + 2..2 * BLOCK_BYTES - 5,
            2 * BLOCK_BYTES - 2..2 * BLOCK_BYTES + 4,
        ];
        let mut served = 0;
        for range in requested {
            let actual = store
                .get_opts(
                    &path,
                    GetOptions {
                        range: Some(range.clone().into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(actual.range, range);
            assert_eq!(
                actual.bytes().await.unwrap(),
                data.slice(range.start as usize..range.end as usize)
            );
            served += range.end - range.start;
        }
        assert_eq!(
            *cloud.ranges.lock().unwrap(),
            vec![0..2 * BLOCK_BYTES, 2 * BLOCK_BYTES..3 * BLOCK_BYTES]
        );
        let stats = store.cache.stats();
        assert_eq!(stats.block_hits, 2);
        assert_eq!(stats.block_misses, 3);
        assert_eq!(stats.downloaded_bytes, 3 * BLOCK_BYTES);
        assert_eq!(stats.disk_bytes, 2 * BLOCK_BYTES);
        assert_eq!(stats.served_bytes, served);
    }

    #[tokio::test]
    async fn large_ranges_stream_coalesced_groups_with_bounded_read_ahead() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        let data = patterned((3 * GROUP_BYTES + 137) as usize);
        cloud.put(&path, data.clone().into()).await.unwrap();
        cloud.delay.store(true, Ordering::SeqCst);
        let range = 17..data.len() as u64 - 9;
        let opts = GetOptions {
            range: Some(range.clone().into()),
            ..Default::default()
        };
        let result = store.get_opts(&path, opts.clone()).await.unwrap();
        assert_eq!(result.range, range);
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        let mut stream = result.into_stream();
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            data.slice(17..GROUP_BYTES as usize)
        );
        drop(stream);
        assert_eq!(
            cloud.gets.load(Ordering::SeqCst),
            1,
            "dropping after the first chunk must not download the rest"
        );
        let result = store.get_opts(&path, opts.clone()).await.unwrap();
        assert_eq!(
            result.bytes().await.unwrap(),
            data.slice(range.start as usize..range.end as usize)
        );
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 4);
        assert!(
            cloud.max_active.load(Ordering::SeqCst) >= 2,
            "large cold reads overlap group downloads"
        );
        assert!(
            cloud
                .ranges
                .lock()
                .unwrap()
                .iter()
                .all(|range| range.end - range.start <= GROUP_BYTES)
        );
        assert_eq!(store.cache.stats().downloaded_bytes, data.len() as u64);
        assert_eq!(
            store
                .get_opts(&path, opts)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            data.slice(range.start as usize..range.end as usize)
        );
        assert_eq!(
            cloud.gets.load(Ordering::SeqCst),
            4,
            "a large warm read uses the same blocks"
        );
    }

    #[tokio::test]
    async fn ranges_and_metadata_match_storage_at_block_boundaries_and_eof() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        let data = patterned((GROUP_BYTES + BLOCK_BYTES + 31) as usize);
        let size = data.len() as u64;
        let mut attributes = Attributes::new();
        attributes.insert(
            object_store::Attribute::ContentType,
            "application/octet-stream".into(),
        );
        cloud
            .put_opts(
                &path,
                data.clone().into(),
                PutOptions {
                    attributes,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let ranges = [
            object_store::GetRange::Bounded(0..1),
            object_store::GetRange::Bounded(BLOCK_BYTES - 1..BLOCK_BYTES + 1),
            object_store::GetRange::Bounded(GROUP_BYTES - 3..GROUP_BYTES + BLOCK_BYTES + 4),
            object_store::GetRange::Bounded(size - 3..size + 99),
            object_store::GetRange::Offset(size - 17),
            object_store::GetRange::Suffix(21),
            object_store::GetRange::Suffix(size + 99),
            object_store::GetRange::Suffix(0),
        ];
        for range in ranges {
            let options = GetOptions {
                range: Some(range),
                ..Default::default()
            };
            let expected = cloud.store.get_opts(&path, options.clone()).await.unwrap();
            let actual = store.get_opts(&path, options).await.unwrap();
            assert_eq!(actual.meta, expected.meta);
            assert_eq!(actual.range, expected.range);
            assert_eq!(actual.attributes, expected.attributes);
            assert_eq!(
                actual.bytes().await.unwrap(),
                expected.bytes().await.unwrap()
            );
        }
        for range in [
            object_store::GetRange::Bounded(0..0),
            object_store::GetRange::Bounded(11..10),
            object_store::GetRange::Bounded(size..size + 1),
            object_store::GetRange::Offset(size),
            object_store::GetRange::Offset(size + 1),
        ] {
            let options = GetOptions {
                range: Some(range),
                ..Default::default()
            };
            assert!(cloud.store.get_opts(&path, options.clone()).await.is_err());
            assert!(store.get_opts(&path, options).await.is_err());
        }
        assert!(store.cache.stats().bypassed_requests >= 5);
    }

    #[tokio::test]
    async fn empty_objects_and_zero_byte_suffixes_do_not_fill_blocks() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, Bytes::new().into()).await.unwrap();
        for range in [None, Some(object_store::GetRange::Suffix(0))] {
            let result = store
                .get_opts(
                    &path,
                    GetOptions {
                        range,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.range, 0..0);
            assert!(result.bytes().await.unwrap().is_empty());
        }
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 0);
        assert_eq!(store.cache.stats().block_misses, 0);
    }

    #[tokio::test]
    async fn concurrent_overlapping_ranges_share_one_cloud_fill() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        let data = patterned(BLOCK_BYTES as usize);
        cloud.put(&path, data.clone().into()).await.unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        *cloud.pause.lock().unwrap() = Some((entered.clone(), resume.clone()));
        let first_store = store.clone();
        let first_path = path.clone();
        let first =
            tokio::spawn(async move { first_store.get_range(&first_path, 10..20).await.unwrap() });
        entered.notified().await;
        let next_store = store.clone();
        let next_path = path.clone();
        let second =
            tokio::spawn(async move { next_store.get_range(&next_path, 15..25).await.unwrap() });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        resume.notify_one();
        assert_eq!(first.await.unwrap(), data.slice(10..20));
        assert_eq!(second.await.unwrap(), data.slice(15..25));
        assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        assert_eq!(store.cache.stats().block_hits, 1);
    }

    #[tokio::test]
    async fn independent_queries_share_the_cloud_concurrency_limit() {
        let (_directory, cloud, _provider, store) = setup();
        cloud.delay.store(true, Ordering::SeqCst);
        let mut paths = Vec::new();
        for index in 0..16 {
            let path = Path::from(format!(
                "db/t.lance/data/00000000-0000-4000-8000-{index:012x}.lance"
            ));
            cloud
                .put(&path, Bytes::from(vec![42; BLOCK_BYTES as usize]).into())
                .await
                .unwrap();
            paths.push(path);
        }
        futures::future::join_all(paths.into_iter().map(|path| {
            let store = store.clone();
            async move {
                assert_eq!(store.get_range(&path, 0..1).await.unwrap().as_ref(), &[42]);
            }
        }))
        .await;
        let concurrency = cloud.max_active.load(Ordering::SeqCst);
        assert!(
            concurrency > 1 && concurrency <= MAX_CLOUD_REQUESTS,
            "active cloud GETs: {concurrency}"
        );
    }

    #[tokio::test]
    async fn later_stream_groups_cannot_mix_object_revisions() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        let size = (2 * GROUP_BYTES + 7) as usize;
        cloud
            .put(&path, Bytes::from(vec![42; size]).into())
            .await
            .unwrap();
        let mut stream = store.get(&path).await.unwrap().into_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.len(), GROUP_BYTES as usize);
        assert!(first.iter().all(|byte| *byte == 42));
        cloud
            .put(&path, Bytes::from(vec![84; size]).into())
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(object_store::Error::Precondition { .. })
        ));
        assert!(stream.next().await.is_none());
        assert_eq!(
            store
                .get_range(&path, size as u64 - 1..size as u64)
                .await
                .unwrap()
                .as_ref(),
            &[84]
        );
    }

    #[tokio::test]
    async fn later_stream_groups_check_revocation_and_local_writes() {
        for revoke in [true, false] {
            let (_directory, cloud, provider, store) = setup();
            let path = Path::from(FILE);
            let size = (GROUP_BYTES + 17) as usize;
            cloud
                .put(&path, Bytes::from(vec![42; size]).into())
                .await
                .unwrap();
            let mut stream = store.get(&path).await.unwrap().into_stream();
            assert!(stream.next().await.unwrap().is_ok());
            if revoke {
                provider.denied.store(true, Ordering::SeqCst);
            } else {
                store
                    .put(&path, Bytes::from(vec![84; size]).into())
                    .await
                    .unwrap();
            }
            assert!(stream.next().await.unwrap().is_err());
            assert!(stream.next().await.is_none());
            assert_eq!(cloud.gets.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn short_and_overlong_cloud_payloads_never_populate_blocks() {
        let (_directory, cloud, _provider, store) = setup();
        let path = Path::from(FILE);
        cloud.put(&path, "abcdef".into()).await.unwrap();
        for corrupt in [1, 2] {
            cloud.corrupt_payload.store(corrupt, Ordering::SeqCst);
            assert!(store.get_range(&path, 1..4).await.is_err());
            assert!(store.cache.state.lock().unwrap().entries.is_empty());
        }
        cloud.corrupt_payload.store(0, Ordering::SeqCst);
        assert_eq!(store.get_range(&path, 1..4).await.unwrap(), "bcd");
    }
}
