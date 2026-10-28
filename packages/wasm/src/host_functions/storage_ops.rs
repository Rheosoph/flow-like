//! Typed, bounded storage operations shared by both WASM ABIs.

use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{
    self, path::Path, CopyMode, CopyOptions, GetOptions, GetRange, MultipartUpload, ObjectMeta,
    ObjectStore, ObjectStoreExt, PutMode, PutOptions, PutPayload, UpdateVersion,
};
use futures::{stream::BoxStream, StreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

use super::{
    storage::{include_listed_file, StorageFlowPath, MAX_STORAGE_FILE_SIZE},
    StorageContext,
};

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl StorageError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message)
    }

    pub fn resource_limit(message: impl Into<String>) -> Self {
        Self::new("resource_limit", message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("unsupported", message)
    }

    pub fn invalid_cursor(message: impl Into<String>) -> Self {
        Self::new("invalid_cursor", message)
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StorageError {}

impl From<flow_like_storage::object_store::Error> for StorageError {
    fn from(error: flow_like_storage::object_store::Error) -> Self {
        use flow_like_storage::object_store::Error;
        let code = match &error {
            Error::NotFound { .. } => "not_found",
            Error::PermissionDenied { .. } | Error::Unauthenticated { .. } => "permission_denied",
            Error::AlreadyExists { .. }
            | Error::Precondition { .. }
            | Error::NotModified { .. } => "conflict",
            Error::InvalidPath { .. } | Error::UnknownConfigurationKey { .. } => "invalid_argument",
            Error::NotSupported { .. } | Error::NotImplemented { .. } => "unsupported",
            _ => "io",
        };
        // Generic backend failures can include permanent errors. Do not promise
        // that replaying a mutation is safe without a known transient cause.
        Self::new(code, error.to_string())
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let code = match error.kind() {
            ErrorKind::NotFound => "not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::AlreadyExists => "conflict",
            ErrorKind::InvalidInput | ErrorKind::InvalidData => "invalid_argument",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Interrupted => "cancelled",
            _ => "io",
        };
        let retryable = matches!(
            error.kind(),
            ErrorKind::WouldBlock
                | ErrorKind::TimedOut
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
        );
        Self::new(code, error.to_string()).with_retryable(retryable)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStat {
    #[serde(flatten)]
    pub flow_path: StorageFlowPath,
    pub size: u64,
    pub e_tag: Option<String>,
    pub version: Option<String>,
    pub last_modified: String,
}

impl FileStat {
    pub fn from_metadata(path: &StorageFlowPath, metadata: ObjectMeta) -> Self {
        Self {
            flow_path: StorageFlowPath {
                path: metadata.location.to_string(),
                ..path.clone()
            },
            size: metadata.size,
            e_tag: metadata.e_tag,
            version: metadata.version,
            last_modified: metadata.last_modified.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriteOptions {
    pub if_match: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub create_only: bool,
}

fn resolve_store(
    ctx: &StorageContext,
    reference: &str,
    path: &Path,
) -> StorageResult<FlowLikeStore> {
    ctx.resolve_store(reference, path).ok_or_else(|| {
        StorageError::new(
            "permission_denied",
            "The store reference does not grant access to this path",
        )
    })
}

fn resolve_cache(
    ctx: &StorageContext,
    flow_path: &StorageFlowPath,
) -> StorageResult<Option<FlowLikeStore>> {
    match flow_path.cache_store_ref.as_deref() {
        Some(reference) if reference != flow_path.store_ref => {
            resolve_store(ctx, reference, &flow_path.object_path()).map(Some)
        }
        _ => Ok(None),
    }
}

pub async fn stat_file(
    ctx: &StorageContext,
    flow_path: &StorageFlowPath,
) -> StorageResult<FileStat> {
    let path = flow_path.object_path();
    let store = resolve_store(ctx, &flow_path.store_ref, &path)?;
    Ok(FileStat::from_metadata(
        flow_path,
        store.as_generic().head(&path).await?,
    ))
}

/// Return at most `length` bytes. At EOF the result is empty; an offset beyond
/// EOF is invalid. The caller can pin every chunk to one ETag with `if_match`.
pub async fn read_file_range(
    ctx: &StorageContext,
    flow_path: &StorageFlowPath,
    offset: u64,
    length: u64,
    if_match: Option<&str>,
) -> StorageResult<Vec<u8>> {
    if length > MAX_STORAGE_FILE_SIZE as u64 {
        return Err(StorageError::resource_limit(
            "A read range may contain at most 10 MiB",
        ));
    }
    let end = offset
        .checked_add(length)
        .ok_or_else(|| StorageError::invalid_argument("The read range overflows u64"))?;
    let path = flow_path.object_path();
    let store = resolve_store(ctx, &flow_path.store_ref, &path)?.as_generic();
    let options = GetOptions {
        if_match: if_match.map(str::to_owned),
        ..Default::default()
    };
    if length == 0 {
        let result = store
            .get_opts(
                &path,
                GetOptions {
                    head: true,
                    ..options
                },
            )
            .await?;
        if offset > result.meta.size {
            return Err(StorageError::invalid_argument(
                "The read offset is beyond EOF",
            ));
        }
        return Ok(Vec::new());
    }
    let result = match store
        .get_opts(
            &path,
            GetOptions {
                range: Some(GetRange::Bounded(offset..end)),
                ..options.clone()
            },
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            // Backends encode unsatisfiable ranges differently. A metadata-only
            // request distinguishes EOF from transport failures on this rare path.
            if let Ok(metadata) = store
                .get_opts(
                    &path,
                    GetOptions {
                        head: true,
                        ..options
                    },
                )
                .await
            {
                if offset == metadata.meta.size {
                    return Ok(Vec::new());
                }
                if offset > metadata.meta.size {
                    return Err(StorageError::invalid_argument(
                        "The read offset is beyond EOF",
                    ));
                }
            }
            return Err(error.into());
        }
    };
    if result.range.start != offset || result.range.end != end.min(result.meta.size) {
        return Err(StorageError::new(
            "io",
            "The backend returned a different byte range",
        ));
    }
    let expected = (result.range.end - result.range.start) as usize;
    let mut bytes = Vec::with_capacity(expected);
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.len() > expected.saturating_sub(bytes.len()) {
            return Err(StorageError::new(
                "io",
                "The backend returned more bytes than requested",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != expected {
        return Err(StorageError::new(
            "io",
            "The backend ended the read before the requested range was complete",
        ));
    }
    Ok(bytes)
}

fn put_mode(options: WriteOptions) -> StorageResult<PutMode> {
    if options.create_only && (options.if_match.is_some() || options.version.is_some()) {
        return Err(StorageError::invalid_argument(
            "create_only cannot be combined with if_match or version",
        ));
    }
    if options.if_match.as_deref() == Some("") || options.version.as_deref() == Some("") {
        return Err(StorageError::invalid_argument(
            "Write preconditions cannot be empty",
        ));
    }
    Ok(if options.create_only {
        PutMode::Create
    } else if options.if_match.is_some() || options.version.is_some() {
        PutMode::Update(UpdateVersion {
            e_tag: options.if_match,
            version: options.version,
        })
    } else {
        PutMode::Overwrite
    })
}

/// Preconditions are enforced by the backend's atomic PUT. Unsupported
/// preconditions fail without falling back to an unconditional write.
pub async fn write_file(
    ctx: &StorageContext,
    flow_path: &StorageFlowPath,
    data: Vec<u8>,
    options: WriteOptions,
) -> StorageResult<FileStat> {
    if data.len() > MAX_STORAGE_FILE_SIZE {
        return Err(StorageError::resource_limit(
            "A write may contain at most 10 MiB; use streaming writes for larger files",
        ));
    }
    let mode = put_mode(options)?;
    let path = flow_path.object_path();
    let store = resolve_store(ctx, &flow_path.store_ref, &path)?.as_generic();
    let cache = resolve_cache(ctx, flow_path)?;
    let payload = PutPayload::from_bytes(data.into());
    let committed = store
        .put_opts(
            &path,
            payload.clone(),
            PutOptions {
                mode,
                ..Default::default()
            },
        )
        .await?;
    if let Some(cache) = cache {
        let cache = cache.as_generic();
        if !Arc::ptr_eq(&store, &cache) {
            if let Err(error) = cache.put(&path, payload).await {
                invalidate_cache(&cache, &path, "write", &error).await;
            }
        }
    }
    let metadata = store
        .get_opts(
            &path,
            GetOptions {
                head: true,
                if_match: committed.e_tag,
                version: committed.version,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            committed_error(
                "The write committed, but reading its metadata failed",
                error,
            )
        })?;
    Ok(FileStat::from_metadata(flow_path, metadata.meta))
}

fn committed_error(message: &str, error: object_store::Error) -> StorageError {
    StorageError::new("committed", format!("{message}: {error}"))
}

async fn invalidate_cache(
    cache: &Arc<dyn ObjectStore>,
    path: &Path,
    operation: &str,
    error: &object_store::Error,
) {
    tracing::warn!(%error, %path, operation, "Storage primary operation succeeded but cache update failed");
    if let Err(invalidation) = cache.delete(path).await {
        tracing::warn!(%invalidation, %path, "Storage cache invalidation failed");
    }
}

/// Delete one object. Deleting an absent object follows the backend's idempotent
/// delete behavior. Cache references are authorized before the primary delete.
pub async fn delete_file(ctx: &StorageContext, flow_path: &StorageFlowPath) -> StorageResult<()> {
    let path = flow_path.object_path();
    let store = resolve_store(ctx, &flow_path.store_ref, &path)?.as_generic();
    let cache = resolve_cache(ctx, flow_path)?;
    store.delete(&path).await?;
    if let Some(cache) = cache {
        if let Err(error) = cache.as_generic().delete(&path).await {
            tracing::warn!(%error, %path, "Storage primary delete succeeded but cache delete failed");
        }
    }
    Ok(())
}

/// Keep the upload handle available until completion or abort, including when
/// the guest invocation drops the in-flight copy future.
struct CopyUpload(Option<Box<dyn MultipartUpload>>);

impl CopyUpload {
    async fn abort(&mut self) {
        if let Some(upload) = self.0.as_mut() {
            if let Err(error) = upload.abort().await {
                tracing::warn!(%error, "Storage copy multipart abort failed");
            }
        }
        self.0.take();
    }
}

impl Drop for CopyUpload {
    fn drop(&mut self) {
        let Some(mut upload) = self.0.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = upload.abort().await {
                    tracing::warn!(%error, "Cancelled storage copy multipart abort failed");
                }
            });
        } else {
            tracing::warn!("Storage copy multipart cleanup requires a running Tokio runtime");
        }
    }
}

pub(super) async fn copy_between_stores(
    source: &Arc<dyn ObjectStore>,
    source_path: &Path,
    destination: &Arc<dyn ObjectStore>,
    destination_path: &Path,
    overwrite: bool,
) -> StorageResult<()> {
    if Arc::ptr_eq(source, destination) {
        if source_path == destination_path {
            source.head(source_path).await?;
            return Ok(());
        }
        destination
            .copy_opts(
                source_path,
                destination_path,
                CopyOptions::new().with_mode(if overwrite {
                    CopyMode::Overwrite
                } else {
                    CopyMode::Create
                }),
            )
            .await?;
        return Ok(());
    }
    let result = source.get(source_path).await?;
    if result.meta.size <= MAX_STORAGE_FILE_SIZE as u64 {
        let size = result.meta.size as usize;
        let mut bytes = Vec::with_capacity(size);
        let mut stream = result.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.len() > size.saturating_sub(bytes.len()) {
                return Err(StorageError::new(
                    "io",
                    "The source returned more bytes than its metadata declared",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != size {
            return Err(StorageError::new(
                "io",
                "The source ended before its declared length",
            ));
        }
        destination
            .put_opts(
                destination_path,
                PutPayload::from_bytes(bytes.into()),
                PutOptions {
                    mode: if overwrite {
                        PutMode::Overwrite
                    } else {
                        PutMode::Create
                    },
                    ..Default::default()
                },
            )
            .await?;
        return Ok(());
    }
    if !overwrite {
        return Err(StorageError::unsupported(
            "Atomic create-only copies across stores are limited to 10 MiB; multipart uploads cannot enforce a create precondition",
        ));
    }
    let expected = result.meta.size;
    let mut transferred = 0_u64;
    let mut upload = CopyUpload(Some(destination.put_multipart(destination_path).await?));
    let mut buffer = Vec::with_capacity(MAX_STORAGE_FILE_SIZE);
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                upload.abort().await;
                return Err(error.into());
            }
        };
        if chunk.len() as u64 > expected.saturating_sub(transferred) {
            upload.abort().await;
            return Err(StorageError::new(
                "io",
                "The source returned more bytes than its metadata declared",
            ));
        }
        transferred += chunk.len() as u64;
        let mut remaining = chunk.as_ref();
        while !remaining.is_empty() {
            let count = remaining.len().min(MAX_STORAGE_FILE_SIZE - buffer.len());
            buffer.extend_from_slice(&remaining[..count]);
            remaining = &remaining[count..];
            if buffer.len() == MAX_STORAGE_FILE_SIZE {
                let payload = PutPayload::from_bytes(std::mem::take(&mut buffer).into());
                if let Err(error) = upload
                    .0
                    .as_mut()
                    .expect("active upload")
                    .put_part(payload)
                    .await
                {
                    upload.abort().await;
                    return Err(error.into());
                }
                buffer = Vec::with_capacity(MAX_STORAGE_FILE_SIZE);
            }
        }
    }
    if transferred != expected {
        upload.abort().await;
        return Err(StorageError::new(
            "io",
            "The source ended before its declared length",
        ));
    }
    if !buffer.is_empty() {
        if let Err(error) = upload
            .0
            .as_mut()
            .expect("active upload")
            .put_part(PutPayload::from_bytes(buffer.into()))
            .await
        {
            upload.abort().await;
            return Err(error.into());
        }
    }
    if let Err(error) = upload.0.as_mut().expect("active upload").complete().await {
        upload.abort().await;
        return Err(error.into());
    }
    upload.0.take();
    Ok(())
}

struct TransferStores {
    source: Arc<dyn ObjectStore>,
    destination: Arc<dyn ObjectStore>,
    source_cache: Option<Arc<dyn ObjectStore>>,
    destination_cache: Option<Arc<dyn ObjectStore>>,
}

fn resolve_transfer(
    ctx: &StorageContext,
    source: &StorageFlowPath,
    destination: &StorageFlowPath,
) -> StorageResult<TransferStores> {
    // Check every supplied reference before the first copy or delete.
    let primary_source = resolve_store(ctx, &source.store_ref, &source.object_path())?.as_generic();
    let primary_destination =
        resolve_store(ctx, &destination.store_ref, &destination.object_path())?.as_generic();
    let source_cache = resolve_cache(ctx, source)?
        .map(|cache| cache.as_generic())
        .filter(|cache| !Arc::ptr_eq(cache, &primary_source));
    let destination_cache = resolve_cache(ctx, destination)?
        .map(|cache| cache.as_generic())
        .filter(|cache| !Arc::ptr_eq(cache, &primary_destination));
    Ok(TransferStores {
        source: primary_source,
        destination: primary_destination,
        source_cache,
        destination_cache,
    })
}

async fn transfer(
    stores: &TransferStores,
    source: &Path,
    destination: &Path,
    overwrite: bool,
) -> StorageResult<()> {
    copy_between_stores(
        &stores.source,
        source,
        &stores.destination,
        destination,
        overwrite,
    )
    .await?;
    if let Some(cache) = &stores.destination_cache {
        if let Err(error) =
            copy_between_stores(&stores.destination, destination, cache, destination, true).await
        {
            tracing::warn!(%error, %destination, "Storage primary copy succeeded but cache copy failed");
            if let Err(error) = cache.delete(destination).await {
                tracing::warn!(%error, %destination, "Storage cache invalidation failed");
            }
        }
    }
    Ok(())
}

pub async fn copy_file(
    ctx: &StorageContext,
    source: &StorageFlowPath,
    destination: &StorageFlowPath,
    overwrite: bool,
) -> StorageResult<()> {
    let stores = resolve_transfer(ctx, source, destination)?;
    transfer(
        &stores,
        &source.object_path(),
        &destination.object_path(),
        overwrite,
    )
    .await
}

/// A cross-store move copies before deleting. If the delete fails, both copies
/// remain and the error explicitly reports that the destination was written.
pub async fn move_file(
    ctx: &StorageContext,
    source: &StorageFlowPath,
    destination: &StorageFlowPath,
    overwrite: bool,
) -> StorageResult<()> {
    let stores = resolve_transfer(ctx, source, destination)?;
    let source_path = source.object_path();
    let destination_path = destination.object_path();
    if Arc::ptr_eq(&stores.source, &stores.destination) && source_path == destination_path {
        stores.source.head(&source_path).await?;
        return Ok(());
    }
    transfer(&stores, &source_path, &destination_path, overwrite).await?;
    stores.source.delete(&source_path).await.map_err(|error| {
        committed_error(
            "The destination was copied, but deleting the source failed; the move is incomplete",
            error,
        )
    })?;
    if let Some(cache) = &stores.source_cache {
        // A source cache may also be the destination's primary store.
        let is_destination = Arc::ptr_eq(cache, &stores.destination)
            || stores
                .destination_cache
                .as_ref()
                .is_some_and(|destination_cache| Arc::ptr_eq(cache, destination_cache));
        if !(is_destination && source_path == destination_path) {
            if let Err(error) = cache.delete(&source_path).await {
                tracing::warn!(%error, %source_path, "Storage move succeeded but source cache delete failed");
            }
        }
    }
    Ok(())
}

pub async fn file_url(
    ctx: &StorageContext,
    flow_path: &StorageFlowPath,
) -> StorageResult<Option<String>> {
    let path = flow_path.object_path();
    let store = resolve_store(ctx, &flow_path.store_ref, &path)?;
    // Asset URLs cannot retain the opened directory handles across a later
    // browser fetch. The resource bridge uses the protected storage read path.
    if matches!(store, FlowLikeStore::Local(_)) {
        return Ok(None);
    }
    store
        .sign_reference("GET", &path, Duration::from_secs(3600))
        .await
        .map(|url| url.map(|url| url.to_string()))
        .map_err(|error| StorageError::new("io", error.to_string()))
}

const MAX_LIST_SESSIONS: usize = 32;
const MAX_LIST_PAGE_SIZE: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPageRequest {
    pub path: StorageFlowPath,
    #[serde(default = "default_page_limit")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub exclude_directories: Vec<String>,
    #[serde(default = "default_recursive")]
    pub recursive: bool,
}

fn default_page_limit() -> usize {
    100
}
fn default_recursive() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPage {
    pub entries: Vec<FileStat>,
    pub cursor: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ListBinding {
    prefix: Path,
    store_ref: String,
    cache_store_ref: Option<String>,
    exclude_directories: Vec<String>,
    recursive: bool,
}

impl ListBinding {
    fn from_request(request: &ListPageRequest) -> Self {
        let mut excluded = request.exclude_directories.clone();
        excluded.sort();
        excluded.dedup();
        Self {
            prefix: request.path.object_path(),
            store_ref: request.path.store_ref.clone(),
            cache_store_ref: request.path.cache_store_ref.clone(),
            exclude_directories: excluded,
            recursive: request.recursive,
        }
    }

    fn includes(&self, metadata: &ObjectMeta) -> bool {
        let Some(parts) = metadata.location.prefix_match(&self.prefix) else {
            return false;
        };
        let depth = parts.count();
        depth > 0
            && (self.recursive || depth == 1)
            && include_listed_file(&metadata.location, &self.prefix, &self.exclude_directories)
    }
}

struct ListStream {
    stream: BoxStream<'static, object_store::Result<ObjectMeta>>,
    lookahead: Option<ObjectMeta>,
}

struct ListSession {
    binding: ListBinding,
    stream: Mutex<ListStream>,
    closed: AtomicBool,
}

/// Cursors own their backend stream and live only for one host invocation.
/// Closing a cursor or dropping the invocation releases its listing state.
#[derive(Default)]
pub struct ListSessions {
    sessions: RwLock<HashMap<String, Arc<ListSession>>>,
}

impl ListSessions {
    pub fn clear(&self) {
        for session in self.sessions.write().drain().map(|(_, session)| session) {
            session.closed.store(true, Ordering::Release);
        }
    }
}

impl std::fmt::Debug for ListSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ListSessions")
            .field("active", &self.sessions.read().len())
            .finish()
    }
}

fn new_cursor() -> StorageResult<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        StorageError::new("io", format!("Could not create a listing cursor: {error}"))
    })?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

async fn next_matching(
    stream: &mut ListStream,
    binding: &ListBinding,
) -> StorageResult<Option<ObjectMeta>> {
    if let Some(metadata) = stream.lookahead.take() {
        return Ok(Some(metadata));
    }
    while let Some(metadata) = stream.stream.next().await {
        let metadata = metadata?;
        if binding.includes(&metadata) {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

pub fn close_list(sessions: &ListSessions, cursor: &str) -> StorageResult<()> {
    let session = sessions.sessions.write().remove(cursor).ok_or_else(|| {
        StorageError::invalid_cursor("The listing cursor is unknown or has already closed")
    })?;
    session.closed.store(true, Ordering::Release);
    Ok(())
}

/// A cancelled page may have consumed entries that never reached the guest.
/// Close its cursor so a retry cannot silently skip those entries.
struct ListPageGuard<'a> {
    sessions: &'a ListSessions,
    cursor: &'a str,
    session: &'a ListSession,
    armed: bool,
}

impl Drop for ListPageGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.closed.store(true, Ordering::Release);
            self.sessions.sessions.write().remove(self.cursor);
        }
    }
}

pub async fn list_files_page(
    ctx: &StorageContext,
    sessions: &ListSessions,
    request: &ListPageRequest,
) -> StorageResult<ListPage> {
    if request.limit == 0 || request.limit > MAX_LIST_PAGE_SIZE {
        return Err(StorageError::invalid_argument(
            "A listing page must contain between 1 and 1000 entries",
        ));
    }
    let binding = ListBinding::from_request(request);
    let store = resolve_store(ctx, &request.path.store_ref, &binding.prefix)?.as_generic();
    let (cursor, session) = if let Some(cursor) = &request.cursor {
        let session = sessions
            .sessions
            .read()
            .get(cursor)
            .cloned()
            .ok_or_else(|| {
                StorageError::invalid_cursor(
                    "The listing cursor is unknown or belongs to another invocation",
                )
            })?;
        if session.binding != binding {
            return Err(StorageError::invalid_cursor(
                "The listing cursor belongs to a different path or listing options",
            ));
        }
        (cursor.clone(), session)
    } else {
        let mut entries = sessions.sessions.write();
        if entries.len() >= MAX_LIST_SESSIONS {
            return Err(StorageError::resource_limit(
                "At most 32 listings may be open; finish or close an existing cursor",
            ));
        }
        let cursor = loop {
            let cursor = new_cursor()?;
            if !entries.contains_key(&cursor) {
                break cursor;
            }
        };
        let stream = super::storage::list_files_excluding(
            store,
            binding.prefix.clone(),
            binding.exclude_directories.clone(),
        );
        let session = Arc::new(ListSession {
            binding,
            stream: Mutex::new(ListStream {
                stream,
                lookahead: None,
            }),
            closed: AtomicBool::new(false),
        });
        entries.insert(cursor.clone(), session.clone());
        (cursor, session)
    };
    let mut guard = ListPageGuard {
        sessions,
        cursor: &cursor,
        session: &session,
        armed: true,
    };
    let mut stream = session.stream.lock().await;
    if session.closed.load(Ordering::Acquire) {
        return Err(StorageError::invalid_cursor(
            "The listing cursor has closed",
        ));
    }
    let result: StorageResult<ListPage> = async {
        let mut entries = Vec::with_capacity(request.limit);
        while entries.len() < request.limit {
            match next_matching(&mut stream, &session.binding).await? {
                Some(metadata) => entries.push(FileStat::from_metadata(&request.path, metadata)),
                None => {
                    return Ok(ListPage {
                        entries,
                        cursor: None,
                    });
                }
            }
        }
        stream.lookahead = next_matching(&mut stream, &session.binding).await?;
        Ok(ListPage {
            entries,
            cursor: stream.lookahead.as_ref().map(|_| cursor.clone()),
        })
    }
    .await;
    if session.closed.load(Ordering::Acquire) {
        return Err(StorageError::new(
            "cancelled",
            "The listing closed while its page was being read",
        ));
    }
    if result.as_ref().map_or(true, |page| page.cursor.is_none()) {
        session.closed.store(true, Ordering::Release);
        sessions.sessions.write().remove(&cursor);
    }
    guard.armed = false;
    result
}

#[cfg(test)]
mod tests {
    use super::super::storage::StorageStore;
    use super::*;
    use flow_like::state::FlowLikeStores;
    use flow_like_storage::files::store::local_store::LocalObjectStore;
    use flow_like_storage::object_store::{
        memory::InMemory, GetResult, ListResult, MultipartUpload, PutMultipartOptions, PutResult,
    };

    fn context() -> StorageContext {
        StorageContext {
            stores: FlowLikeStores::default(),
            store_cache: RwLock::new(HashMap::new()),
            credentials_store: None,
            app_id: "test".into(),
            board_dir: Path::from("apps/test"),
            board_id: "board".into(),
            node_id: "node".into(),
            sub: "user".into(),
        }
    }

    fn grant(ctx: &StorageContext, reference: &str, store: FlowLikeStore, root: &str) {
        ctx.store_cache.write().insert(
            reference.into(),
            StorageStore {
                store,
                roots: vec![Path::from(root)],
            },
        );
    }

    fn path(reference: &str, path: &str) -> StorageFlowPath {
        StorageFlowPath {
            path: path.into(),
            store_ref: reference.into(),
            cache_store_ref: None,
        }
    }

    fn memory_context() -> (StorageContext, FlowLikeStore, StorageFlowPath) {
        let ctx = context();
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        grant(&ctx, "primary", store.clone(), "project");
        (ctx, store, path("primary", "project/file"))
    }

    async fn bytes(store: &FlowLikeStore, path: &StorageFlowPath) -> Vec<u8> {
        store
            .as_generic()
            .get(&path.object_path())
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn conditional_writes_preserve_newer_content_and_create_only_conflicts() {
        let (ctx, store, path) = memory_context();
        let original = write_file(&ctx, &path, b"old".to_vec(), WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(original.size, 3);
        assert!(original.e_tag.is_some());
        assert!(!original.last_modified.is_empty());
        let latest = write_file(
            &ctx,
            &path,
            b"latest".to_vec(),
            WriteOptions {
                if_match: original.e_tag.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_ne!(original.e_tag, latest.e_tag);
        let error = write_file(
            &ctx,
            &path,
            b"stale".to_vec(),
            WriteOptions {
                if_match: original.e_tag,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(bytes(&store, &path).await, b"latest");
        let error = write_file(
            &ctx,
            &path,
            b"overwrite".to_vec(),
            WriteOptions {
                create_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(bytes(&store, &path).await, b"latest");
        let error = write_file(
            &ctx,
            &path,
            b"invalid".to_vec(),
            WriteOptions {
                create_only: true,
                if_match: Some("etag".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "invalid_argument");
    }

    #[tokio::test]
    async fn range_reads_bound_memory_and_validate_offsets_and_etags() {
        let (ctx, store, path) = memory_context();
        let metadata = write_file(&ctx, &path, b"0123456789".to_vec(), WriteOptions::default())
            .await
            .unwrap();
        assert_eq!(
            read_file_range(&ctx, &path, 2, 4, metadata.e_tag.as_deref())
                .await
                .unwrap(),
            b"2345"
        );
        assert_eq!(
            read_file_range(&ctx, &path, 8, 4, None).await.unwrap(),
            b"89"
        );
        assert!(read_file_range(&ctx, &path, 10, 4, None)
            .await
            .unwrap()
            .is_empty());
        assert!(read_file_range(&ctx, &path, 10, 0, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            read_file_range(&ctx, &path, 11, 1, None)
                .await
                .unwrap_err()
                .code,
            "invalid_argument"
        );
        assert_eq!(
            read_file_range(&ctx, &path, 11, 0, None)
                .await
                .unwrap_err()
                .code,
            "invalid_argument"
        );
        assert_eq!(
            read_file_range(&ctx, &path, u64::MAX, 1, None)
                .await
                .unwrap_err()
                .code,
            "invalid_argument"
        );
        assert_eq!(
            read_file_range(&ctx, &path, 0, MAX_STORAGE_FILE_SIZE as u64 + 1, None)
                .await
                .unwrap_err()
                .code,
            "resource_limit"
        );
        store
            .as_generic()
            .put(&path.object_path(), PutPayload::from_static(b"changed"))
            .await
            .unwrap();
        assert_eq!(
            read_file_range(&ctx, &path, 0, 1, metadata.e_tag.as_deref())
                .await
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(
            read_file_range(&ctx, &path, 7, 1, metadata.e_tag.as_deref())
                .await
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[tokio::test]
    async fn local_urls_never_expose_ambient_paths_to_guests() {
        let root = tempfile::tempdir().unwrap();
        let store =
            FlowLikeStore::Local(Arc::new(LocalObjectStore::new(root.path().into()).unwrap()));
        let ctx = context();
        grant(&ctx, "local", store, "project");
        let file = path("local", "project/file.bin");
        assert_eq!(file_url(&ctx, &file).await.unwrap(), None);
        assert_eq!(
            crate::host_functions::storage::flow_path_url(&ctx, &file).await,
            None
        );
    }

    #[tokio::test]
    async fn local_files_support_ranges_and_cannot_escape_the_granted_folder() {
        let root = tempfile::tempdir().unwrap();
        let store =
            FlowLikeStore::Local(Arc::new(LocalObjectStore::new(root.path().into()).unwrap()));
        let ctx = context();
        grant(&ctx, "local", store.clone(), "project");
        let allowed = path("local", "project/large.bin");
        store
            .as_generic()
            .put(&allowed.object_path(), PutPayload::from_static(b"abcdef"))
            .await
            .unwrap();
        assert_eq!(
            read_file_range(&ctx, &allowed, 1, 3, None).await.unwrap(),
            b"bcd"
        );
        assert_eq!(stat_file(&ctx, &allowed).await.unwrap().size, 6);
        let outside = path("local", "project-other/file");
        assert_eq!(
            stat_file(&ctx, &outside).await.unwrap_err().code,
            "permission_denied"
        );
        assert_eq!(
            write_file(&ctx, &outside, vec![1], WriteOptions::default())
                .await
                .unwrap_err()
                .code,
            "permission_denied"
        );
        assert!(!root.path().join("project-other/file").exists());
        let denied_update = write_file(
            &ctx,
            &allowed,
            b"unsafe".to_vec(),
            WriteOptions {
                if_match: Some("stale".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            denied_update.code.as_str(),
            "unsupported" | "conflict"
        ));
        assert_eq!(bytes(&store, &allowed).await, b"abcdef");
    }

    #[tokio::test]
    async fn invalid_cache_grants_reject_before_mutation() {
        let (ctx, store, mut source) = memory_context();
        store
            .as_generic()
            .put(&source.object_path(), PutPayload::from_static(b"source"))
            .await
            .unwrap();
        source.cache_store_ref = Some("unresolved".into());
        assert_eq!(
            delete_file(&ctx, &source).await.unwrap_err().code,
            "permission_denied"
        );
        assert_eq!(
            write_file(&ctx, &source, b"bad".to_vec(), WriteOptions::default())
                .await
                .unwrap_err()
                .code,
            "permission_denied"
        );
        let destination = path("primary", "project/destination");
        assert_eq!(
            move_file(&ctx, &source, &destination, true)
                .await
                .unwrap_err()
                .code,
            "permission_denied"
        );
        assert_eq!(bytes(&store, &source).await, b"source");
        assert!(store
            .as_generic()
            .head(&destination.object_path())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn copy_and_move_do_not_clobber_existing_destinations() {
        let (ctx, store, source) = memory_context();
        let destination = path("primary", "project/destination");
        store
            .as_generic()
            .put(&source.object_path(), PutPayload::from_static(b"source"))
            .await
            .unwrap();
        store
            .as_generic()
            .put(
                &destination.object_path(),
                PutPayload::from_static(b"destination"),
            )
            .await
            .unwrap();
        assert_eq!(
            move_file(&ctx, &source, &destination, false)
                .await
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(bytes(&store, &source).await, b"source");
        assert_eq!(bytes(&store, &destination).await, b"destination");
        copy_file(&ctx, &source, &destination, true).await.unwrap();
        assert_eq!(bytes(&store, &source).await, b"source");
        assert_eq!(bytes(&store, &destination).await, b"source");
        move_file(&ctx, &source, &destination, true).await.unwrap();
        assert_eq!(
            stat_file(&ctx, &source).await.unwrap_err().code,
            "not_found"
        );
        delete_file(&ctx, &destination).await.unwrap();
        assert_eq!(
            stat_file(&ctx, &destination).await.unwrap_err().code,
            "not_found"
        );
    }

    #[tokio::test]
    async fn failed_cross_store_copy_preserves_source_and_failed_delete_reports_partial_move() {
        let (ctx, store, source) = memory_context();
        let destination_store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        grant(
            &ctx,
            "destination",
            destination_store.read_only(),
            "project",
        );
        let destination = path("destination", "project/destination");
        store
            .as_generic()
            .put(&source.object_path(), PutPayload::from_static(b"source"))
            .await
            .unwrap();
        assert_eq!(
            move_file(&ctx, &source, &destination, true)
                .await
                .unwrap_err()
                .code,
            "unsupported"
        );
        assert_eq!(bytes(&store, &source).await, b"source");
        assert!(destination_store
            .as_generic()
            .head(&destination.object_path())
            .await
            .is_err());

        grant(&ctx, "primary", store.read_only(), "project");
        grant(&ctx, "destination", destination_store.clone(), "project");
        let error = move_file(&ctx, &source, &destination, true)
            .await
            .unwrap_err();
        assert_eq!(error.code, "committed");
        assert!(error.message.contains("destination was copied"));
        assert!(!error.retryable);
        assert_eq!(bytes(&store, &source).await, b"source");
        assert_eq!(bytes(&destination_store, &destination).await, b"source");
    }

    #[tokio::test]
    async fn large_cross_store_copy_streams_all_bytes() {
        let (ctx, source_store, source) = memory_context();
        let destination_store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        grant(&ctx, "destination", destination_store.clone(), "project");
        let destination = path("destination", "project/destination");
        let content = vec![42_u8; MAX_STORAGE_FILE_SIZE * 2 + 19];
        source_store
            .as_generic()
            .put(
                &source.object_path(),
                PutPayload::from_bytes(content.clone().into()),
            )
            .await
            .unwrap();
        copy_file(&ctx, &source, &destination, true).await.unwrap();
        assert_eq!(bytes(&destination_store, &destination).await, content);
        assert_eq!(
            stat_file(&ctx, &source).await.unwrap().size,
            content.len() as u64
        );
    }

    #[derive(Debug)]
    struct ScriptedListing {
        inner: InMemory,
        entries: Vec<Option<String>>,
        polls: Arc<std::sync::atomic::AtomicUsize>,
        upload: Option<Arc<UploadObserver>>,
        fail_reads: bool,
    }

    impl std::fmt::Display for ScriptedListing {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "ScriptedListing")
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for ScriptedListing {
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
            let upload = self.inner.put_multipart_opts(location, options).await?;
            Ok(match &self.upload {
                Some(observer) => Box::new(ObservedUpload {
                    inner: upload,
                    observer: observer.clone(),
                }),
                None => upload,
            })
        }
        async fn get_opts(
            &self,
            location: &Path,
            options: GetOptions,
        ) -> object_store::Result<GetResult> {
            let fail = self.fail_reads && !options.head;
            let mut result = self.inner.get_opts(location, options).await?;
            if fail {
                result.payload = object_store::GetResultPayload::Stream(
                    futures::stream::once(async { Err(injected_error("read")) }).boxed(),
                );
            }
            Ok(result)
        }
        fn delete_stream(
            &self,
            locations: BoxStream<'static, object_store::Result<Path>>,
        ) -> BoxStream<'static, object_store::Result<Path>> {
            self.inner.delete_stream(locations)
        }
        fn list(&self, _: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            let polls = self.polls.clone();
            futures::stream::iter(self.entries.clone())
                .map(move |entry| {
                    polls.fetch_add(1, Ordering::Relaxed);
                    entry
                        .map(|path| ObjectMeta {
                            location: Path::from(path),
                            last_modified: Default::default(),
                            size: 1,
                            e_tag: None,
                            version: None,
                        })
                        .ok_or_else(|| object_store::Error::Generic {
                            store: "test",
                            source: "listing failed".into(),
                        })
                })
                .boxed()
        }
        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            if self.entries.is_empty() {
                return self.inner.list_with_delimiter(prefix).await;
            }
            let listing = InMemory::new();
            for entry in &self.entries {
                let location = Path::from(entry.as_deref().ok_or_else(|| injected_error("list"))?);
                listing
                    .put(&location, PutPayload::from_static(b"x"))
                    .await?;
            }
            listing.list_with_delimiter(prefix).await
        }
        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            options: CopyOptions,
        ) -> object_store::Result<()> {
            self.inner.copy_opts(from, to, options).await
        }
    }

    fn scripted_listing(
        entries: Vec<Option<&str>>,
    ) -> (
        StorageContext,
        ListPageRequest,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let ctx = context();
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        grant(
            &ctx,
            "list",
            FlowLikeStore::Other(Arc::new(ScriptedListing {
                inner: InMemory::new(),
                entries: entries
                    .into_iter()
                    .map(|entry| entry.map(str::to_owned))
                    .collect(),
                polls: polls.clone(),
                upload: None,
                fail_reads: false,
            })),
            "project",
        );
        let request = ListPageRequest {
            path: path("list", "project"),
            limit: 2,
            cursor: None,
            exclude_directories: vec![],
            recursive: true,
        };
        (ctx, request, polls)
    }

    #[tokio::test]
    async fn pagination_preserves_backend_order_and_reads_one_lookahead() {
        let (ctx, mut request, polls) = scripted_listing(vec![
            Some("project/z"),
            Some("project/a"),
            Some("project/t"),
            Some("project/b"),
            Some("project/c"),
        ]);
        let sessions = ListSessions::default();
        let first = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(polls.load(Ordering::Relaxed), 3);
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.flow_path.path.as_str())
                .collect::<Vec<_>>(),
            vec!["project/z", "project/a"]
        );
        request.cursor = first.cursor;
        let second = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(polls.load(Ordering::Relaxed), 5);
        assert_eq!(
            second
                .entries
                .iter()
                .map(|entry| entry.flow_path.path.as_str())
                .collect::<Vec<_>>(),
            vec!["project/t", "project/b"]
        );
        request.cursor = second.cursor;
        let third = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(third.entries[0].flow_path.path, "project/c");
        assert!(third.cursor.is_none());
        assert!(sessions.sessions.read().is_empty());
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
    }

    #[tokio::test]
    async fn listing_errors_are_returned_and_the_cursor_is_closed() {
        let (ctx, request, _) = scripted_listing(vec![
            Some("project/a"),
            Some("project/b"),
            None,
            Some("project/c"),
        ]);
        let sessions = ListSessions::default();
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "io"
        );
        assert!(sessions.sessions.read().is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_page_invalidates_its_partially_consumed_cursor() {
        let (ctx, mut request, _) = scripted_listing(vec![
            Some("project/a"),
            Some("project/b"),
            Some("project/c"),
        ]);
        let ctx = Arc::new(ctx);
        let sessions = Arc::new(ListSessions::default());
        request.cursor = list_files_page(&ctx, &sessions, &request)
            .await
            .unwrap()
            .cursor;
        let session = sessions
            .sessions
            .read()
            .get(request.cursor.as_ref().unwrap())
            .unwrap()
            .clone();
        let pending = Arc::new(tokio::sync::Notify::new());
        let notify = pending.clone();
        session.stream.lock().await.stream = futures::stream::once(async move {
            notify.notify_one();
            futures::future::pending().await
        })
        .boxed();
        let task_context = ctx.clone();
        let task_sessions = sessions.clone();
        let task_request = request.clone();
        let task = tokio::spawn(async move {
            list_files_page(&task_context, &task_sessions, &task_request).await
        });
        tokio::time::timeout(Duration::from_secs(2), pending.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(sessions.sessions.read().is_empty());
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
    }

    #[tokio::test]
    async fn cursors_bind_to_options_and_can_be_closed_explicitly() {
        let (ctx, mut request, _) = scripted_listing(vec![
            Some("project/a"),
            Some("project/b"),
            Some("project/c"),
        ]);
        let sessions = ListSessions::default();
        request.cursor = list_files_page(&ctx, &sessions, &request)
            .await
            .unwrap()
            .cursor;
        let mut altered = request.clone();
        altered.recursive = false;
        assert_eq!(
            list_files_page(&ctx, &sessions, &altered)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        altered = request.clone();
        altered.path.path = "project/sub".into();
        assert_eq!(
            list_files_page(&ctx, &sessions, &altered)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        assert_eq!(
            list_files_page(&ctx, &ListSessions::default(), &request)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        close_list(&sessions, request.cursor.as_deref().unwrap()).unwrap();
        assert!(sessions.sessions.read().is_empty());
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
        request.cursor = None;
        request.cursor = list_files_page(&ctx, &sessions, &request)
            .await
            .unwrap()
            .cursor;
        sessions.clear();
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "invalid_cursor"
        );
    }

    #[tokio::test]
    async fn listings_filter_directory_segments_and_never_escape_the_prefix() {
        let (ctx, mut request, _) = scripted_listing(vec![
            Some("project/z"),
            Some("project/node_modules/a"),
            Some("project/src/a"),
            Some("project/src/node_modules"),
            Some("project-other/secret"),
        ]);
        request.limit = 10;
        let sessions = ListSessions::default();
        // The recursive backend includes an out-of-scope entry deliberately.
        let page = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(page.entries.len(), 4);
        assert!(page
            .entries
            .iter()
            .all(|entry| entry.flow_path.path.starts_with("project/")));
        request.exclude_directories = vec!["node_modules".into()];
        let page = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.flow_path.path.as_str())
                .collect::<Vec<_>>(),
            vec!["project/z", "project/src/a", "project/src/node_modules"]
        );
        request.recursive = false;
        let page = list_files_page(&ctx, &sessions, &request).await.unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].flow_path.path, "project/z");
    }

    #[tokio::test]
    async fn listing_limits_bound_live_sessions() {
        let (ctx, request, _) = scripted_listing(vec![
            Some("project/a"),
            Some("project/b"),
            Some("project/c"),
        ]);
        let sessions = ListSessions::default();
        for _ in 0..MAX_LIST_SESSIONS {
            assert!(list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap()
                .cursor
                .is_some());
        }
        assert_eq!(
            list_files_page(&ctx, &sessions, &request)
                .await
                .unwrap_err()
                .code,
            "resource_limit"
        );
        let mut invalid = request.clone();
        invalid.limit = MAX_LIST_PAGE_SIZE + 1;
        assert_eq!(
            list_files_page(&ctx, &sessions, &invalid)
                .await
                .unwrap_err()
                .code,
            "invalid_argument"
        );
    }

    #[tokio::test]
    async fn memory_url_does_not_inline_file_contents() {
        let (ctx, _, path) = memory_context();
        assert!(file_url(&ctx, &path).await.unwrap().is_none());
    }

    #[derive(Debug, Clone, Copy)]
    enum UploadFailure {
        Part,
        Complete,
        PendingPart,
    }

    #[derive(Debug)]
    struct UploadObserver {
        failure: UploadFailure,
        largest_part: std::sync::atomic::AtomicUsize,
        aborts: std::sync::atomic::AtomicUsize,
        part_started: tokio::sync::Notify,
        aborted: tokio::sync::Notify,
    }

    impl UploadObserver {
        fn new(failure: UploadFailure) -> Arc<Self> {
            Arc::new(Self {
                failure,
                largest_part: Default::default(),
                aborts: Default::default(),
                part_started: Default::default(),
                aborted: Default::default(),
            })
        }
    }

    #[derive(Debug)]
    struct ObservedUpload {
        inner: Box<dyn MultipartUpload>,
        observer: Arc<UploadObserver>,
    }

    fn injected_error(stage: &str) -> object_store::Error {
        object_store::Error::Generic {
            store: "test",
            source: format!("injected {stage} failure").into(),
        }
    }

    #[async_trait::async_trait]
    impl MultipartUpload for ObservedUpload {
        fn put_part(&mut self, data: PutPayload) -> object_store::UploadPart {
            self.observer
                .largest_part
                .fetch_max(data.content_length(), Ordering::Relaxed);
            self.observer.part_started.notify_one();
            match self.observer.failure {
                UploadFailure::Part => Box::pin(async { Err(injected_error("part")) }),
                UploadFailure::PendingPart => Box::pin(futures::future::pending()),
                UploadFailure::Complete => self.inner.put_part(data),
            }
        }

        async fn complete(&mut self) -> object_store::Result<PutResult> {
            Err(injected_error("complete"))
        }

        async fn abort(&mut self) -> object_store::Result<()> {
            self.observer.aborts.fetch_add(1, Ordering::Relaxed);
            self.observer.aborted.notify_one();
            self.inner.abort().await
        }
    }

    async fn copy_failure_fixture(
        failure: UploadFailure,
        fail_reads: bool,
    ) -> (
        Arc<StorageContext>,
        StorageFlowPath,
        StorageFlowPath,
        Arc<UploadObserver>,
    ) {
        let ctx = Arc::new(context());
        let source = FlowLikeStore::Other(Arc::new(ScriptedListing {
            inner: InMemory::new(),
            entries: vec![],
            polls: Default::default(),
            upload: None,
            fail_reads,
        }));
        let observer = UploadObserver::new(failure);
        let destination = FlowLikeStore::Other(Arc::new(ScriptedListing {
            inner: InMemory::new(),
            entries: vec![],
            polls: Default::default(),
            upload: Some(observer.clone()),
            fail_reads: false,
        }));
        grant(&ctx, "source", source.clone(), "project");
        grant(&ctx, "destination", destination.clone(), "project");
        let source_path = path("source", "project/file");
        let destination_path = path("destination", "project/file");
        source
            .as_generic()
            .put(
                &source_path.object_path(),
                PutPayload::from_bytes(vec![7_u8; MAX_STORAGE_FILE_SIZE + 3].into()),
            )
            .await
            .unwrap();
        destination
            .as_generic()
            .put(
                &destination_path.object_path(),
                PutPayload::from_static(b"original"),
            )
            .await
            .unwrap();
        (ctx, source_path, destination_path, observer)
    }

    #[tokio::test]
    async fn multipart_copy_aborts_on_read_part_and_completion_failures() {
        for (failure, fail_reads) in [
            (UploadFailure::Part, false),
            (UploadFailure::Complete, false),
            (UploadFailure::Complete, true),
        ] {
            let (ctx, source, destination, observer) =
                copy_failure_fixture(failure, fail_reads).await;
            assert_eq!(
                move_file(&ctx, &source, &destination, true)
                    .await
                    .unwrap_err()
                    .code,
                "io"
            );
            assert_eq!(observer.aborts.load(Ordering::Relaxed), 1);
            assert!(observer.largest_part.load(Ordering::Relaxed) <= MAX_STORAGE_FILE_SIZE);
            assert_eq!(
                stat_file(&ctx, &source).await.unwrap().size,
                MAX_STORAGE_FILE_SIZE as u64 + 3
            );
            assert_eq!(
                read_file_range(&ctx, &destination, 0, 8, None)
                    .await
                    .unwrap(),
                b"original"
            );
        }
    }

    #[tokio::test]
    async fn dropping_an_in_flight_copy_aborts_the_backend_upload() {
        let (ctx, source, destination, observer) =
            copy_failure_fixture(UploadFailure::PendingPart, false).await;
        let task_context = ctx.clone();
        let task_source = source.clone();
        let task_destination = destination.clone();
        let task = tokio::spawn(async move {
            copy_file(&task_context, &task_source, &task_destination, true).await
        });
        tokio::time::timeout(Duration::from_secs(2), observer.part_started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), observer.aborted.notified())
            .await
            .unwrap();
        assert_eq!(observer.aborts.load(Ordering::Relaxed), 1);
        assert_eq!(
            stat_file(&ctx, &source).await.unwrap().size,
            MAX_STORAGE_FILE_SIZE as u64 + 3
        );
        assert_eq!(
            read_file_range(&ctx, &destination, 0, 8, None)
                .await
                .unwrap(),
            b"original"
        );
    }

    #[tokio::test]
    async fn large_create_only_copy_never_falls_back_to_overwrite() {
        let (ctx, source, destination, observer) =
            copy_failure_fixture(UploadFailure::Part, false).await;
        assert_eq!(
            copy_file(&ctx, &source, &destination, false)
                .await
                .unwrap_err()
                .code,
            "unsupported"
        );
        assert_eq!(observer.largest_part.load(Ordering::Relaxed), 0);
        assert_eq!(observer.aborts.load(Ordering::Relaxed), 0);
        assert_eq!(
            read_file_range(&ctx, &destination, 0, 8, None)
                .await
                .unwrap(),
            b"original"
        );
    }

    #[tokio::test]
    async fn moving_between_stores_keeps_a_shared_destination_cache() {
        let (ctx, primary, mut source) = memory_context();
        let destination_store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let shared_cache = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        grant(&ctx, "destination", destination_store.clone(), "project");
        grant(&ctx, "source_cache", shared_cache.clone(), "project");
        grant(&ctx, "destination_cache", shared_cache.clone(), "project");
        source.cache_store_ref = Some("source_cache".into());
        let mut destination = path("destination", "project/file");
        destination.cache_store_ref = Some("destination_cache".into());
        primary
            .as_generic()
            .put(&source.object_path(), PutPayload::from_static(b"source"))
            .await
            .unwrap();
        move_file(&ctx, &source, &destination, true).await.unwrap();
        assert_eq!(bytes(&destination_store, &destination).await, b"source");
        assert_eq!(bytes(&shared_cache, &destination).await, b"source");
        assert_eq!(
            stat_file(&ctx, &source).await.unwrap_err().code,
            "not_found"
        );
    }
}
