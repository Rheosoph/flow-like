//! File stores of online-app runs (design §4.7): the local cache view, the cloud view with
//! its connectivity breaker, the buffered file store and the Lance store of unavailable
//! databases.

use super::{
    object_index::{ContentRoots, KeyClass, Known, MAX_LISTING_BYTES, ObjectIndex},
    texts,
};
use flow_like::flow_like_storage::{
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
    lance::Result as LanceResult,
    lance_io::object_store::{
        ObjectStore as LanceObjectStore, ObjectStoreParams, ObjectStoreProvider,
    },
    object_store::{
        self, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload,
        ObjectMeta, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload,
        PutResult, RenameOptions, UploadPart, memory::InMemory, path::Path as ObjectPath,
        signer::Signer,
    },
};
use flow_like_device_protocol::StoragePurpose;
use flow_like_offline_writes::{
    Connectivity, FileBuffering, FileOverlay, FileOverlayOptions, FileRoute, Observation,
    PendingFile, fs::is_offline_error,
};
use flow_like_types::{
    Bytes,
    async_stream::try_stream,
    authorization::AuthorizationError,
    reqwest::{Method, Url},
};
use futures::{StreamExt, stream::BoxStream};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(crate) const UNAVAILABLE_SCHEME: &str = "flow-like-offline";
const STORE: &str = "DesktopOfflineFiles";
const CACHE_STORE: &str = "DesktopOfflineCache";
const UNAVAILABLE_STORE: &str = "FlowLikeOffline";
const READ_PROBE: Duration = Duration::from_secs(5);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BREAKER_BACKOFF: Duration = Duration::from_secs(30);
const DIRECT_WRITE_BASE: Duration = Duration::from_secs(10);
const DIRECT_WRITE_MIN_BYTES_PER_SECOND: u64 = 64 * 1024;
const PENDING_SETTLE: Duration = Duration::from_secs(10);
const INLINE_LINK_BYTES: u64 = 8 * 1024 * 1024;
const ABSENCE_PROBE_ENTRIES: usize = 64;

/// E17, carried inside the cache view's errors so the file overlay classifies it offline.
#[derive(Debug)]
pub(crate) struct CacheMiss(pub(crate) String);

impl fmt::Display for CacheMiss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CacheMiss {}

fn generic(text: impl Into<String>) -> object_store::Error {
    object_store::Error::Generic {
        store: STORE,
        source: text.into().into(),
    }
}

fn miss(path: &ObjectPath) -> object_store::Error {
    object_store::Error::Generic {
        store: CACHE_STORE,
        source: Box::new(CacheMiss(texts::not_cached(path.as_ref()))),
    }
}

fn denied(path: &ObjectPath) -> object_store::Error {
    object_store::Error::PermissionDenied {
        path: path.to_string(),
        source: texts::other_scope(path.as_ref()).into(),
    }
}

fn not_found(path: &ObjectPath, reason: &str) -> object_store::Error {
    object_store::Error::NotFound {
        path: path.to_string(),
        source: reason.to_owned().into(),
    }
}

fn failing_stream<T: Send + 'static>(
    error: object_store::Error,
) -> BoxStream<'static, object_store::Result<T>> {
    futures::stream::once(async move { Err(error) }).boxed()
}

/// How a cloud call failed, for the breaker and the cache fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Connect,
    Timeout,
    Server,
    Other,
}

pub(crate) fn failure(error: &object_store::Error) -> Failure {
    use object_store::client::{HttpError, HttpErrorKind};
    if !matches!(error, object_store::Error::Generic { .. }) {
        return Failure::Other;
    }
    let mut server = false;
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.is::<CacheMiss>() {
            return Failure::Connect;
        }
        if let Some(error) = current.downcast_ref::<AuthorizationError>() {
            return match error {
                AuthorizationError::Unavailable => Failure::Connect,
                _ => Failure::Other,
            };
        }
        if let Some(error) = current.downcast_ref::<HttpError>() {
            match error.kind() {
                HttpErrorKind::Connect | HttpErrorKind::Interrupted => return Failure::Connect,
                HttpErrorKind::Timeout => return Failure::Timeout,
                _ => {}
            }
        }
        if let Some(error) = current.downcast_ref::<flow_like_types::reqwest::Error>() {
            if error.is_timeout() {
                return Failure::Timeout;
            }
            if error.is_connect() {
                return Failure::Connect;
            }
            server |= error
                .status()
                .is_some_and(|status| status.is_server_error());
        }
        if let Some(error) = current.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind;
            match error.kind() {
                ErrorKind::TimedOut => return Failure::Timeout,
                ErrorKind::NotConnected
                | ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NetworkUnreachable
                | ErrorKind::HostUnreachable => return Failure::Connect,
                _ => {}
            }
        }
        server |= current.to_string().contains("non-2xx status code: 5");
        source = current.source();
    }
    if server {
        Failure::Server
    } else {
        Failure::Other
    }
}

/// The file overlay's `offline_error`: offline-class errors (cache misses included) as
/// connect failures or timeouts. An expired lease is not one: the hub answered 401 or
/// returned credentials that had already expired.
pub(crate) fn offline_observation(error: &object_store::Error) -> Option<Observation> {
    if lease_expired(error) || !is_offline_error(error, &|source| source.is::<CacheMiss>()) {
        return None;
    }
    Some(if failure(error) == Failure::Timeout {
        Observation::TimedOut
    } else {
        Observation::ConnectFailed
    })
}

fn lease_expired(error: &object_store::Error) -> bool {
    std::iter::successors(
        Some(error as &(dyn std::error::Error + 'static)),
        |&current| current.source(),
    )
    .find_map(|current| current.downcast_ref::<AuthorizationError>())
    .is_some_and(|error| matches!(error, AuthorizationError::Expired))
}

type Reconnected = Arc<dyn Fn() + Send + Sync>;

/// The scope's connectivity breaker. It follows the hub: only a failed reachability check
/// opens it, and the check keeps running with backoff until the hub answers again.
pub(crate) struct ScopeConnectivity {
    this: Weak<Self>,
    health: String,
    client: flow_like_types::reqwest::Client,
    offline: AtomicBool,
    checking: AtomicBool,
    reconnected: Mutex<Option<Reconnected>>,
}

impl fmt::Debug for ScopeConnectivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeConnectivity")
            .field("offline", &self.is_offline())
            .finish()
    }
}

impl ScopeConnectivity {
    pub(crate) fn new(hub: &str) -> Arc<Self> {
        let health = format!("{}/health", super::api_base(hub));
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            health,
            client: flow_like_types::reqwest::Client::builder()
                .redirect(flow_like_types::reqwest::redirect::Policy::none())
                .timeout(HEALTH_TIMEOUT)
                .build()
                .unwrap_or_default(),
            offline: AtomicBool::new(false),
            checking: AtomicBool::new(false),
            reconnected: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_offline(&self, offline: bool) {
        if offline {
            self.offline.store(true, Ordering::Release);
        } else {
            self.close();
        }
    }

    pub(crate) fn on_reconnect(&self, callback: Reconnected) {
        *self
            .reconnected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(callback);
    }

    fn close(&self) {
        if self.offline.swap(false, Ordering::AcqRel) {
            let callback = self
                .reconnected
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(callback) = callback {
                callback();
            }
        }
    }

    async fn reachable(&self) -> bool {
        self.client.get(&self.health).send().await.is_ok()
    }

    fn start_check(&self) {
        if self.checking.swap(true, Ordering::AcqRel) {
            return;
        }
        let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() else {
            self.checking.store(false, Ordering::Release);
            return;
        };
        let this = self.this.clone();
        runtime.spawn(async move {
            let mut delay = Duration::from_secs(1);
            loop {
                let Some(breaker) = this.upgrade() else {
                    return;
                };
                if breaker.reachable().await {
                    breaker.close();
                    breaker.checking.store(false, Ordering::Release);
                    return;
                }
                breaker.offline.store(true, Ordering::Release);
                drop(breaker);
                flow_like_types::tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_BREAKER_BACKOFF);
            }
        });
    }
}

impl Connectivity for ScopeConnectivity {
    fn is_offline(&self) -> bool {
        self.offline.load(Ordering::Acquire)
    }

    fn observe(&self, observation: Observation) {
        match observation {
            Observation::Succeeded => self.close(),
            Observation::ConnectFailed | Observation::TimedOut => self.start_check(),
        }
    }
}

/// The view of Disconnected runs: every buffered write is queued; outcomes still reach the breaker.
pub(crate) struct DisconnectedConnectivity(pub(crate) Arc<ScopeConnectivity>);

impl Connectivity for DisconnectedConnectivity {
    fn is_offline(&self) -> bool {
        true
    }

    fn observe(&self, observation: Observation) {
        self.0.observe(observation);
    }
}

/// Cached copies of this device: FlowPath's local cache files plus the object index.
#[derive(Clone)]
pub(crate) struct LocalCacheView {
    pub(crate) index: Option<Arc<ObjectIndex>>,
    pub(crate) project: Arc<LocalObjectStore>,
    pub(crate) user: Arc<LocalObjectStore>,
    pub(crate) roots: ContentRoots,
}

impl fmt::Debug for LocalCacheView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopOfflineCache")
    }
}

impl fmt::Display for LocalCacheView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopOfflineCache")
    }
}

fn head_result(meta: ObjectMeta) -> GetResult {
    GetResult {
        payload: GetResultPayload::Stream(futures::stream::empty().boxed()),
        meta,
        range: 0..0,
        attributes: Default::default(),
    }
}

impl LocalCacheView {
    pub(crate) fn local_for(&self, path: &ObjectPath) -> &Arc<LocalObjectStore> {
        if self.roots.is_user(path.as_ref()) {
            &self.user
        } else {
            &self.project
        }
    }

    fn lookup(&self, path: &ObjectPath) -> object_store::Result<Known> {
        match &self.index {
            Some(index) => index
                .lookup(path)
                .map_err(|error| generic(error.to_string())),
            None => Ok(Known::Unknown),
        }
    }

    async fn local_meta(&self, path: &ObjectPath) -> Option<ObjectMeta> {
        self.local_for(path).head(path).await.ok()
    }

    async fn serve(
        &self,
        path: &ObjectPath,
        options: GetOptions,
        meta: ObjectMeta,
    ) -> object_store::Result<GetResult> {
        options.check_preconditions(&meta)?;
        if options
            .version
            .as_ref()
            .is_some_and(|version| meta.version.as_ref() != Some(version))
        {
            return Err(miss(path));
        }
        if options.head {
            return Ok(head_result(meta));
        }
        let local = GetOptions {
            range: options.range.clone(),
            ..GetOptions::default()
        };
        let mut result = self.local_for(path).get_opts(path, local).await?;
        result.meta = meta;
        Ok(result)
    }

    async fn cached_get(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        match self.lookup(path)? {
            Known::Absent => Err(not_found(path, "The file is absent in the cloud")),
            Known::Present(meta) => match self.local_meta(path).await {
                _ if options.head => self.serve(path, options, meta).await,
                Some(local) if local.size == meta.size => self.serve(path, options, meta).await,
                _ => Err(miss(path)),
            },
            Known::Unknown => match self.local_meta(path).await {
                Some(local) => {
                    let meta = ObjectMeta {
                        e_tag: None,
                        version: None,
                        ..local
                    };
                    self.serve(path, options, meta).await
                }
                None => Err(miss(path)),
            },
        }
    }

    fn listed(&self, prefix: &ObjectPath) -> object_store::Result<Vec<ObjectMeta>> {
        let listing = match &self.index {
            Some(index) => index
                .listing(prefix)
                .map_err(|error| generic(error.to_string()))?,
            None => None,
        };
        listing.ok_or_else(|| generic(texts::listing_unavailable(prefix.as_ref())))
    }

    fn write_refused(&self, path: &ObjectPath) -> object_store::Error {
        match self.roots.classify(path.as_ref()) {
            KeyClass::Foreign | KeyClass::Outside => denied(path),
            _ => generic(texts::write_needs_hub(path.as_ref())),
        }
    }
}

#[flow_like_types::async_trait]
impl ObjectStore for LocalCacheView {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        _payload: PutPayload,
        _options: PutOptions,
    ) -> object_store::Result<PutResult> {
        Err(self.write_refused(path))
    }

    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        _options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        Err(self.write_refused(path))
    }

    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        match self.roots.classify(path.as_ref()) {
            KeyClass::Content => self.cached_get(path, options).await,
            KeyClass::Metadata => self.project.get_opts(path, options).await,
            KeyClass::Database { user } => Err(generic(texts::database_unavailable(user))),
            KeyClass::Foreign | KeyClass::Outside => Err(denied(path)),
        }
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let this = self.clone();
        paths
            .map(move |path| Err(this.write_refused(&path?)))
            .boxed()
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let prefix = prefix.cloned().unwrap_or_default();
        match self.roots.classify(prefix.as_ref()) {
            KeyClass::Content => match self.listed(&prefix) {
                Ok(entries) => futures::stream::iter(entries.into_iter().map(Ok)).boxed(),
                Err(error) => failing_stream(error),
            },
            KeyClass::Metadata => self.project.list(Some(&prefix)),
            KeyClass::Database { user } => {
                failing_stream(generic(texts::database_unavailable(user)))
            }
            KeyClass::Foreign | KeyClass::Outside => failing_stream(denied(&prefix)),
        }
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        let prefix = prefix.cloned().unwrap_or_default();
        match self.roots.classify(prefix.as_ref()) {
            KeyClass::Content => Ok(one_level(&prefix, self.listed(&prefix)?)),
            KeyClass::Metadata => self.project.list_with_delimiter(Some(&prefix)).await,
            KeyClass::Database { user } => Err(generic(texts::database_unavailable(user))),
            KeyClass::Foreign | KeyClass::Outside => Err(denied(&prefix)),
        }
    }

    async fn copy_opts(
        &self,
        _from: &ObjectPath,
        to: &ObjectPath,
        _options: CopyOptions,
    ) -> object_store::Result<()> {
        Err(self.write_refused(to))
    }

    async fn rename_opts(
        &self,
        _from: &ObjectPath,
        to: &ObjectPath,
        _options: RenameOptions,
    ) -> object_store::Result<()> {
        Err(self.write_refused(to))
    }
}

/// One level of a recursive listing below `prefix`.
fn one_level(prefix: &ObjectPath, entries: Vec<ObjectMeta>) -> ListResult {
    let root = if prefix.as_ref().is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    let mut prefixes = BTreeSet::new();
    let mut objects = Vec::new();
    for meta in entries {
        let Some(relative) = meta.location.as_ref().strip_prefix(&root) else {
            continue;
        };
        match relative.split_once('/') {
            Some((directory, _)) => {
                if let Ok(path) = ObjectPath::parse(format!("{root}{directory}")) {
                    prefixes.insert(path);
                }
            }
            None => objects.push(meta),
        }
    }
    ListResult {
        common_prefixes: prefixes.into_iter().collect(),
        objects,
    }
}

/// The cloud content store, recording what it proves into the index and falling back to the
/// cache when the cloud is unreachable. Without a cloud (Disconnected runs) it is the cache.
#[derive(Clone)]
pub(crate) struct DesktopCloudView {
    pub(crate) cloud: Option<Arc<dyn ObjectStore>>,
    pub(crate) cache: LocalCacheView,
    pub(crate) connectivity: Arc<dyn Connectivity>,
    pub(crate) probe: Duration,
}

impl fmt::Debug for DesktopCloudView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopCloudView")
    }
}

impl fmt::Display for DesktopCloudView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopCloudView")
    }
}

impl DesktopCloudView {
    pub(crate) fn new(
        cloud: Option<Arc<dyn ObjectStore>>,
        cache: LocalCacheView,
        connectivity: Arc<dyn Connectivity>,
    ) -> Self {
        Self {
            cloud,
            cache,
            connectivity,
            probe: READ_PROBE,
        }
    }

    fn cached_content(&self, path: &ObjectPath) -> bool {
        self.cache.roots.is_content(path.as_ref())
    }

    fn record(&self, record: impl FnOnce(&ObjectIndex) -> anyhow::Result<()>) {
        if let Some(index) = &self.cache.index
            && let Err(error) = record(index)
        {
            tracing::debug!(%error, "Could not record a cloud object revision");
        }
    }

    fn record_put(&self, path: &ObjectPath, size: u64, result: &PutResult) {
        if !self.cached_content(path) {
            return;
        }
        let meta = ObjectMeta {
            location: path.clone(),
            last_modified: chrono::Utc::now(),
            size,
            e_tag: result.e_tag.clone(),
            version: result.version.clone(),
        };
        self.record(|index| index.observe_present(&meta));
    }

    /// A 403 for a key the invoke policies cannot read is confirmed by a listing that starts
    /// just before the key: cloud listings are ordered, so passing the key proves it absent.
    async fn confirmed_absent(&self, cloud: &Arc<dyn ObjectStore>, path: &ObjectPath) -> bool {
        let parts: Vec<_> = path.parts().collect();
        let Some((name, directory)) = parts.split_last() else {
            return false;
        };
        let parent = ObjectPath::from_iter(directory.iter().cloned());
        let name = name.as_ref();
        let stem = &name[..name.char_indices().last().map_or(0, |(index, _)| index)];
        let offset = if stem.is_empty() {
            Ok(parent.clone())
        } else if parent.as_ref().is_empty() {
            ObjectPath::parse(stem)
        } else {
            ObjectPath::parse(format!("{parent}/{stem}"))
        };
        let Ok(offset) = offset else {
            return false;
        };
        let mut listed = cloud
            .list_with_offset(Some(&parent), &offset)
            .take(ABSENCE_PROBE_ENTRIES);
        let mut seen = 0;
        while let Some(entry) = listed.next().await {
            let Ok(meta) = entry else {
                return false;
            };
            if meta.location == *path {
                return false;
            }
            if meta.location.as_ref() > path.as_ref() {
                return true;
            }
            seen += 1;
        }
        seen < ABSENCE_PROBE_ENTRIES
    }

    async fn settle_get(
        &self,
        cloud: &Arc<dyn ObjectStore>,
        path: &ObjectPath,
        options: GetOptions,
        result: object_store::Result<GetResult>,
    ) -> object_store::Result<GetResult> {
        match result {
            Ok(result) => {
                self.connectivity.observe(Observation::Succeeded);
                if options.version.is_none() {
                    self.record(|index| index.observe_present(&result.meta));
                }
                Ok(result)
            }
            Err(error @ object_store::Error::NotFound { .. }) => {
                self.connectivity.observe(Observation::Succeeded);
                if options.version.is_none() {
                    self.record(|index| index.observe_absent(path));
                }
                Err(error)
            }
            Err(error @ object_store::Error::PermissionDenied { .. }) => {
                if self.confirmed_absent(cloud, path).await {
                    self.record(|index| index.observe_absent(path));
                    return Err(not_found(path, "The file is absent in the cloud"));
                }
                Err(error)
            }
            Err(error) => match failure(&error) {
                Failure::Connect => {
                    self.connectivity.observe(Observation::ConnectFailed);
                    self.cache.get_opts(path, options).await
                }
                Failure::Timeout => {
                    self.connectivity.observe(Observation::TimedOut);
                    self.cache.get_opts(path, options).await.map_err(|_| error)
                }
                Failure::Server => self.cache.get_opts(path, options).await.map_err(|_| error),
                Failure::Other => Err(error),
            },
        }
    }

    async fn delete_one(&self, path: &ObjectPath) -> object_store::Result<()> {
        let Some(cloud) = &self.cloud else {
            return Err(self.cache.write_refused(path));
        };
        cloud.delete(path).await?;
        if self.cached_content(path) {
            self.record(|index| index.observe_absent(path));
        }
        Ok(())
    }

    fn cloud_list(
        &self,
        cloud: Arc<dyn ObjectStore>,
        prefix: ObjectPath,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let this = self.clone();
        Box::pin(try_stream! {
            let mut stream = cloud.list(Some(&prefix));
            let first = stream.next().await;
            let first = match first {
                Some(Err(error)) if failure(&error) == Failure::Connect => {
                    this.connectivity.observe(Observation::ConnectFailed);
                    let mut cached = this.cache.list(Some(&prefix));
                    while let Some(meta) = cached.next().await {
                        yield meta?;
                    }
                    return;
                }
                other => other,
            };
            let mut entries = Vec::new();
            let mut bytes = 0usize;
            let mut recordable = true;
            let mut next = first;
            while let Some(item) = next {
                let meta = item?;
                if recordable {
                    bytes = bytes.saturating_add(meta.location.as_ref().len() + 96);
                    recordable = bytes <= MAX_LISTING_BYTES;
                    if recordable {
                        entries.push(meta.clone());
                    } else {
                        entries.clear();
                    }
                }
                yield meta;
                next = stream.next().await;
            }
            this.connectivity.observe(Observation::Succeeded);
            if recordable {
                this.record(|index| index.record_listing(&prefix, &entries));
            }
        })
    }
}

#[flow_like_types::async_trait]
impl ObjectStore for DesktopCloudView {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        let Some(cloud) = &self.cloud else {
            return Err(self.cache.write_refused(path));
        };
        let size = payload.content_length() as u64;
        let result = cloud.put_opts(path, payload, options).await?;
        self.record_put(path, size, &result);
        Ok(result)
    }

    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        let Some(cloud) = &self.cloud else {
            return Err(self.cache.write_refused(path));
        };
        let upload = cloud.put_multipart_opts(path, options).await?;
        if !self.cached_content(path) {
            return Ok(upload);
        }
        Ok(Box::new(RecordedUpload {
            upload,
            view: self.clone(),
            path: path.clone(),
            size: 0,
        }))
    }

    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        let Some(cloud) = &self.cloud else {
            return self.cache.get_opts(path, options).await;
        };
        if !self.cached_content(path) {
            return cloud.get_opts(path, options).await;
        }
        if self.connectivity.is_offline() {
            return self.cache.get_opts(path, options).await;
        }
        let call = cloud.get_opts(path, options.clone());
        futures::pin_mut!(call);
        let result = match flow_like_types::tokio::time::timeout(self.probe, &mut call).await {
            Ok(result) => result,
            Err(_) => {
                self.connectivity.observe(Observation::TimedOut);
                if let Ok(hit) = self.cache.get_opts(path, options.clone()).await {
                    return Ok(hit);
                }
                call.await
            }
        };
        self.settle_get(cloud, path, options, result).await
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let this = self.clone();
        paths
            .then(move |path| {
                let this = this.clone();
                async move {
                    let path = path?;
                    this.delete_one(&path).await?;
                    Ok(path)
                }
            })
            .boxed()
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let Some(cloud) = &self.cloud else {
            return self.cache.list(prefix);
        };
        let prefix = prefix.cloned().unwrap_or_default();
        if !self.cached_content(&prefix) {
            return cloud.list(Some(&prefix));
        }
        if self.connectivity.is_offline() {
            return self.cache.list(Some(&prefix));
        }
        self.cloud_list(cloud.clone(), prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        let Some(cloud) = &self.cloud else {
            return self.cache.list_with_delimiter(prefix).await;
        };
        let prefix = prefix.cloned().unwrap_or_default();
        if !self.cached_content(&prefix) {
            return cloud.list_with_delimiter(Some(&prefix)).await;
        }
        if self.connectivity.is_offline() {
            return self.cache.list_with_delimiter(Some(&prefix)).await;
        }
        match cloud.list_with_delimiter(Some(&prefix)).await {
            Ok(listed) => {
                self.connectivity.observe(Observation::Succeeded);
                self.record(|index| index.observe_present_all(&listed.objects));
                Ok(listed)
            }
            Err(error) if failure(&error) == Failure::Connect => {
                self.connectivity.observe(Observation::ConnectFailed);
                self.cache.list_with_delimiter(Some(&prefix)).await
            }
            Err(error) => Err(error),
        }
    }

    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        let Some(cloud) = &self.cloud else {
            return Err(self.cache.write_refused(to));
        };
        cloud.copy_opts(from, to, options).await?;
        if self.cached_content(to) {
            self.record(|index| index.forget(to));
        }
        Ok(())
    }

    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        let Some(cloud) = &self.cloud else {
            return Err(self.cache.write_refused(to));
        };
        cloud.rename_opts(from, to, options).await?;
        if self.cached_content(to) {
            self.record(|index| index.forget(to));
        }
        if self.cached_content(from) {
            self.record(|index| index.observe_absent(from));
        }
        Ok(())
    }
}

/// A cloud multipart upload of content that records the object once it completes.
struct RecordedUpload {
    upload: Box<dyn MultipartUpload>,
    view: DesktopCloudView,
    path: ObjectPath,
    size: u64,
}

impl fmt::Debug for RecordedUpload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedUpload")
            .field("path", &self.path)
            .field("upload", &self.upload)
            .finish()
    }
}

#[flow_like_types::async_trait]
impl MultipartUpload for RecordedUpload {
    fn put_part(&mut self, data: PutPayload) -> UploadPart {
        self.size = self.size.saturating_add(data.content_length() as u64);
        self.upload.put_part(data)
    }

    async fn complete(&mut self) -> object_store::Result<PutResult> {
        let result = self.upload.complete().await?;
        self.view.record_put(&self.path, self.size, &result);
        Ok(result)
    }

    async fn abort(&mut self) -> object_store::Result<()> {
        self.upload.abort().await
    }
}

/// Why a Disconnected run's content writes cannot be queued.
#[derive(Clone, Debug)]
pub(crate) enum Refusal {
    /// E6: a PAT run whose account was never confirmed on this device.
    Unattributed,
    /// E7: the scope's offline changes cannot open.
    Unavailable(String),
    /// E26: the hub does not accept offline changes, or that was never checked.
    HubUnsupported,
}

impl Refusal {
    fn error(&self, path: &ObjectPath) -> object_store::Error {
        generic(match self {
            Self::Unattributed => texts::UNATTRIBUTED.to_owned(),
            Self::Unavailable(error) => texts::unavailable(error),
            Self::HubUnsupported => texts::hub_rejects_files(path.as_ref()),
        })
    }
}

#[derive(Clone)]
pub(crate) enum FileLayer {
    Overlay(Arc<FileOverlay>),
    Refused(Refusal),
}

/// The content store of an online-app run: the engine's file overlay over the cloud view,
/// with signing and FlowPath cache bookkeeping.
#[derive(Clone)]
pub(crate) struct DesktopFileStore {
    pub(crate) layer: FileLayer,
    pub(crate) cloud: Option<FlowLikeStore>,
    pub(crate) cache: LocalCacheView,
}

impl fmt::Debug for DesktopFileStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopFileStore")
    }
}

impl fmt::Display for DesktopFileStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DesktopFileStore")
    }
}

impl DesktopFileStore {
    fn refused(&self, path: &ObjectPath) -> Option<object_store::Error> {
        match &self.layer {
            FileLayer::Refused(refusal) if self.cache.roots.is_content(path.as_ref()) => {
                Some(refusal.error(path))
            }
            FileLayer::Refused(_) => Some(self.cache.write_refused(path)),
            FileLayer::Overlay(_) => None,
        }
    }

    fn reader(&self) -> &dyn ObjectStore {
        match &self.layer {
            FileLayer::Overlay(overlay) => overlay.as_ref(),
            FileLayer::Refused(_) => &self.cache,
        }
    }

    async fn inline_link(path: &ObjectPath, bytes: Bytes) -> object_store::Result<Url> {
        let memory = Arc::new(InMemory::new());
        memory.put(path, bytes.into()).await?;
        FlowLikeStore::Memory(memory)
            .sign("GET", path, Duration::from_secs(60))
            .await
            .map_err(|error| generic(error.to_string()))
    }
}

#[flow_like_types::async_trait]
impl ObjectStore for DesktopFileStore {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        let FileLayer::Overlay(overlay) = &self.layer else {
            return Err(self
                .refused(path)
                .unwrap_or_else(|| self.cache.write_refused(path)));
        };
        let result = overlay.put_opts(path, payload, options).await?;
        if let (Some(index), Some(operation)) = (
            &self.cache.index,
            result
                .e_tag
                .as_deref()
                .and_then(|tag| tag.strip_prefix("offline-")),
        ) && let Err(error) = index.mark_queued(path, operation)
        {
            tracing::debug!(%error, "Could not mark a queued file in the object index");
        }
        Ok(result)
    }

    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        match &self.layer {
            FileLayer::Overlay(overlay) => overlay.put_multipart_opts(path, options).await,
            FileLayer::Refused(_) => Err(self
                .refused(path)
                .unwrap_or_else(|| self.cache.write_refused(path))),
        }
    }

    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.reader().get_opts(path, options).await
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        match &self.layer {
            FileLayer::Overlay(overlay) => overlay.delete_stream(paths),
            FileLayer::Refused(_) => {
                let this = self.clone();
                paths
                    .map(move |path| {
                        let path = path?;
                        Err(this
                            .refused(&path)
                            .unwrap_or_else(|| this.cache.write_refused(&path)))
                    })
                    .boxed()
            }
        }
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.reader().list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        self.reader().list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        match &self.layer {
            FileLayer::Overlay(overlay) => overlay.copy_opts(from, to, options).await,
            FileLayer::Refused(_) => Err(self
                .refused(to)
                .unwrap_or_else(|| self.cache.write_refused(to))),
        }
    }

    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        match &self.layer {
            FileLayer::Overlay(overlay) => overlay.rename_opts(from, to, options).await,
            FileLayer::Refused(_) => Err(self
                .refused(to)
                .unwrap_or_else(|| self.cache.write_refused(to))),
        }
    }
}

#[flow_like_types::async_trait]
impl Signer for DesktopFileStore {
    async fn signed_url(
        &self,
        method: Method,
        path: &ObjectPath,
        expires_in: Duration,
    ) -> object_store::Result<Url> {
        if matches!(
            self.cache.roots.classify(path.as_ref()),
            KeyClass::Foreign | KeyClass::Outside
        ) {
            return Err(denied(path));
        }
        if let FileLayer::Overlay(overlay) = &self.layer {
            match overlay.pending(path)? {
                PendingFile::Put { .. } if method == Method::GET => {
                    let bytes = overlay.pending_bytes(path)?.ok_or_else(|| miss(path))?;
                    return Self::inline_link(path, bytes).await;
                }
                PendingFile::Put { .. } => {
                    return Err(generic(texts::waiting_to_upload(path.as_ref())));
                }
                PendingFile::Deleted => {
                    return Err(not_found(path, "The file is deleted on this device"));
                }
                PendingFile::None => {}
            }
        }
        if let Some(cloud) = &self.cloud {
            return cloud
                .sign(method.as_str(), path, expires_in)
                .await
                .map_err(|error| generic(error.to_string()));
        }
        if method != Method::GET {
            return Err(generic(texts::waiting_to_upload(path.as_ref())));
        }
        let meta = self.cache.head(path).await?;
        if self
            .cache
            .local_meta(path)
            .await
            .is_none_or(|local| local.size != meta.size)
        {
            return Err(miss(path));
        }
        if meta.size <= INLINE_LINK_BYTES {
            let bytes = self.cache.get(path).await?.bytes().await?;
            return Self::inline_link(path, bytes).await;
        }
        FlowLikeStore::Local(self.cache.local_for(path).clone())
            .sign("GET", path, expires_in)
            .await
            .map_err(|error| generic(error.to_string()))
    }
}

/// Buffered file roots of a scope (§1.1): the project upload and storage folders and the
/// account's user folder, each as a whole.
pub(crate) fn file_options(
    prefixes: &[(StoragePurpose, String)],
    scheme: &str,
    connectivity: Arc<dyn Connectivity>,
    max_file_bytes: usize,
) -> FileOverlayOptions {
    FileOverlayOptions {
        routes: prefixes
            .iter()
            .map(|(purpose, root)| FileRoute {
                purpose: *purpose,
                root: root.clone(),
                prefix: String::new(),
                scheme: scheme.to_owned(),
            })
            .collect(),
        buffering: FileBuffering::WhenOffline {
            connectivity,
            direct_write_base: DIRECT_WRITE_BASE,
            direct_write_min_bytes_per_second: DIRECT_WRITE_MIN_BYTES_PER_SECOND,
            pending_settle_timeout: PENDING_SETTLE,
        },
        max_file_bytes,
        offline_error: Arc::new(offline_observation),
    }
}

/// Configured tables that are not registered with the engine, per database, at run start.
#[derive(Clone, Debug, Default)]
pub(crate) struct UnreadyTables {
    pub(crate) project: HashSet<String>,
    pub(crate) user: HashSet<String>,
}

impl UnreadyTables {
    /// E3 for configured tables that are not set up yet, E1 for other tables, E2 otherwise.
    pub(crate) fn text(&self, key: &str) -> String {
        let user = key.starts_with("users/");
        let tables = if user { &self.user } else { &self.project };
        match key
            .split('/')
            .find_map(|segment| segment.strip_suffix(".lance"))
        {
            Some(name) if tables.contains(name) => texts::table_not_ready(name),
            Some(name) => texts::table_unavailable(name),
            None => texts::database_unavailable(user),
        }
    }
}

/// Databases of a Disconnected run that are not available offline.
#[derive(Clone, Debug)]
pub(crate) struct UnavailableStore {
    tables: Arc<UnreadyTables>,
}

impl UnavailableStore {
    pub(crate) fn new(tables: Arc<UnreadyTables>) -> Self {
        Self { tables }
    }

    fn error(&self, key: &str) -> object_store::Error {
        object_store::Error::Generic {
            store: UNAVAILABLE_STORE,
            source: self.tables.text(key).into(),
        }
    }
}

impl fmt::Display for UnavailableStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(UNAVAILABLE_STORE)
    }
}

#[flow_like_types::async_trait]
impl ObjectStore for UnavailableStore {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        _payload: PutPayload,
        _options: PutOptions,
    ) -> object_store::Result<PutResult> {
        Err(self.error(path.as_ref()))
    }

    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        _options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        Err(self.error(path.as_ref()))
    }

    async fn get_opts(
        &self,
        path: &ObjectPath,
        _options: GetOptions,
    ) -> object_store::Result<GetResult> {
        Err(self.error(path.as_ref()))
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let this = self.clone();
        paths
            .map(move |path| Err(this.error(path?.as_ref())))
            .boxed()
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        failing_stream(self.error(prefix.map(ObjectPath::as_ref).unwrap_or_default()))
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        Err(self.error(prefix.map(ObjectPath::as_ref).unwrap_or_default()))
    }

    async fn copy_opts(
        &self,
        _from: &ObjectPath,
        to: &ObjectPath,
        _options: CopyOptions,
    ) -> object_store::Result<()> {
        Err(self.error(to.as_ref()))
    }
}

/// Lance provider of the `flow-like-offline` scheme; opening a store does no I/O.
#[derive(Debug)]
pub(crate) struct UnavailableProvider {
    tables: Arc<UnreadyTables>,
}

impl UnavailableProvider {
    pub(crate) fn new(tables: UnreadyTables) -> Self {
        Self {
            tables: Arc::new(tables),
        }
    }
}

#[flow_like_types::async_trait]
impl ObjectStoreProvider for UnavailableProvider {
    async fn new_store(
        &self,
        base_path: Url,
        params: &ObjectStoreParams,
    ) -> LanceResult<LanceObjectStore> {
        Ok(LanceObjectStore::new(
            Arc::new(UnavailableStore::new(self.tables.clone())),
            base_path,
            params.block_size,
            None,
            false,
            true,
            8,
            0,
            None,
        ))
    }

    fn calculate_object_store_prefix(
        &self,
        url: &Url,
        _options: Option<&HashMap<String, String>>,
    ) -> LanceResult<String> {
        Ok(format!("{}${}", url.scheme(), url.authority()))
    }
}
