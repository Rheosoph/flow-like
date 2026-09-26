use crate::{
    fs::unix_time,
    host::{Connectivity, Observation},
    limits::{ReplayLimits, validate_request},
    manager::WriteManager,
    outbox::{Outbox, QueuedOperation},
};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Bytes, BytesMut};
use flow_like_device_protocol::{
    OfflineExpected, OfflineMutation, OfflineReplayRequest, OfflineResource, StoragePurpose,
    format_limit,
};
use flow_like_storage::object_store::{
    self, Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, RenameOptions, UploadPart, path::Path as ObjectPath,
};
use flow_like_types::async_stream::try_stream;
use futures_util::{StreamExt, stream::BoxStream};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify};

const LANCE_OBJECTS: &str = "Lance objects cannot be buffered as file writes";
const UNREPLAYABLE: &str = "This cloud provider cannot replay conditional file overwrites or deletes; use a new immutable file name";
const COPY: &str = "Copy involving buffered files requires an explicit read followed by a write";
const RENAME: &str = "Rename involving buffered files is not an atomic offline operation";

/// A buffered directory: object keys under `root` + `prefix` are queued.
#[derive(Clone)]
pub struct FileRoute {
    pub purpose: StoragePurpose,
    /// Authorized object-key prefix of the purpose, ending in '/'.
    pub root: String,
    /// Relative directory under `root`, ending in '/'.
    pub prefix: String,
    /// URL scheme of the cloud store ("s3", "az", "gs", …); decides replayable preconditions.
    pub scheme: String,
}

/// When writes under a route are queued.
#[derive(Clone)]
pub enum FileBuffering {
    /// Standalone: every write under a route is queued.
    Always,
    /// Desktop: writes go to the cloud while it is reachable and are queued when the
    /// breaker is open, a direct write fails offline-class or times out, or the path has
    /// queued operations.
    WhenOffline {
        connectivity: Arc<dyn Connectivity>,
        /// direct_write_timeout(len) = base + len / min_bytes_per_second
        direct_write_base: Duration,
        direct_write_min_bytes_per_second: u64,
        pending_settle_timeout: Duration,
    },
}

pub type OfflineErrorClassifier =
    Arc<dyn Fn(&object_store::Error) -> Option<Observation> + Send + Sync>;

pub struct FileOverlayOptions {
    pub routes: Vec<FileRoute>,
    pub buffering: FileBuffering,
    /// Pre-check; `validate_with(replay_limits)` is authoritative.
    pub max_file_bytes: usize,
    /// Some(kind) for offline-class errors of `inner`.
    pub offline_error: OfflineErrorClassifier,
}

/// What the queue holds for a path.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingFile {
    None,
    Put { meta: ObjectMeta },
    Deleted,
}

/// Table-format commit files (a `_delta_log` or `.hoodie` directory, `version-hint.text`, a
/// `.metadata.json` suffix), relative to a route. They are never buffered.
pub fn table_format_commit(relative: &str) -> bool {
    let mut segments = relative.split('/').collect::<Vec<_>>();
    let name = segments.pop().unwrap_or_default();
    segments
        .iter()
        .any(|segment| matches!(*segment, "_delta_log" | ".hoodie"))
        || name == "version-hint.text"
        || name.ends_with(".metadata.json")
}

type Authorize = Arc<dyn Fn() -> Result<()> + Send + Sync>;
type Changed = Arc<dyn Fn() + Send + Sync>;

#[cfg(feature = "test-support")]
pub struct FileOverlayParts {
    pub inner: Arc<dyn ObjectStore>,
    pub queue: Arc<Outbox>,
    pub authorize: Authorize,
    pub gate: Arc<Mutex<()>>,
    pub wake: Arc<Notify>,
    pub replay_limits: flow_like_device_protocol::OfflineLimits,
    pub options: FileOverlayOptions,
}

#[derive(Clone)]
pub struct FileOverlay {
    inner: Arc<dyn ObjectStore>,
    pub queue: Arc<Outbox>,
    routes: Arc<Vec<FileRoute>>,
    authorize: Authorize,
    gate: Arc<Mutex<()>>,
    wake: Arc<Notify>,
    changed: Changed,
    buffering: FileBuffering,
    max_file_bytes: usize,
    replay_limits: ReplayLimits,
    offline_error: OfflineErrorClassifier,
}

enum Target<'a> {
    Outside,
    Lance,
    Commit,
    Buffered {
        key: String,
        resource: OfflineResource,
        route: &'a FileRoute,
    },
}

impl fmt::Debug for FileOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceOfflineFiles")
    }
}
impl fmt::Display for FileOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceOfflineFiles")
    }
}
fn error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> object_store::Error {
    object_store::Error::Generic {
        store: "DeviceOfflineFiles",
        source: error.into(),
    }
}
fn unsupported(
    message: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> object_store::Error {
    object_store::Error::NotSupported {
        source: message.into(),
    }
}
fn missing(path: &ObjectPath) -> object_store::Error {
    object_store::Error::NotFound {
        path: path.to_string(),
        source: "File is absent in the local overlay".into(),
    }
}
fn unknown_existence(path: &ObjectPath) -> object_store::Error {
    error(format!(
        "'{path}' may already exist in the cloud, and this device cannot check while offline. Creating it only if absent needs a connection to the hub."
    ))
}
fn table_format_offline(path: &ObjectPath) -> object_store::Error {
    error(format!(
        "'{path}' belongs to a table format commit log (Delta, Iceberg or Hudi). These commits need a connection to the hub."
    ))
}
fn still_uploading(path: &ObjectPath) -> object_store::Error {
    error(format!(
        "'{path}' is still uploading from an earlier offline change. Try again in a moment."
    ))
}
fn unknown_revision(path: &ObjectPath) -> object_store::Error {
    unsupported(format!(
        "This device does not know the cloud revision of '{path}', so it cannot be replaced or deleted offline. Write a new file name instead."
    ))
}
fn decode_put(data_base64: &str, sha256: &str) -> Result<Bytes> {
    let bytes = STANDARD.decode(data_base64)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == sha256,
        "Queued file payload digest differs"
    );
    Ok(Bytes::from(bytes))
}

/// True once `key` has no non-terminal operations; false after `timeout`.
pub(crate) async fn wait_idle(queue: &Outbox, key: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match queue.has_pending(key) {
            Ok(false) => return true,
            Ok(true) => (),
            Err(_) => return false,
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        tokio::time::sleep((deadline - now).min(Duration::from_millis(50))).await;
    }
}

impl FileOverlay {
    pub(crate) fn new(
        inner: Arc<dyn ObjectStore>,
        manager: Arc<WriteManager>,
        options: FileOverlayOptions,
    ) -> Self {
        let queue = manager.queue.clone();
        let gate = manager.gate.clone();
        let wake = manager.wake.clone();
        let replay_limits = manager.replay_limits.clone();
        let host = manager.host.clone();
        Self::build(
            inner,
            queue,
            Arc::new(move || manager.authorize()),
            gate,
            wake,
            Arc::new(move || host.queue_changed()),
            replay_limits,
            options,
        )
    }
    #[cfg(feature = "test-support")]
    pub fn for_tests(parts: FileOverlayParts) -> Self {
        Self::build(
            parts.inner,
            parts.queue,
            parts.authorize,
            parts.gate,
            parts.wake,
            Arc::new(|| ()),
            ReplayLimits::new(parts.replay_limits),
            parts.options,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn build(
        inner: Arc<dyn ObjectStore>,
        queue: Arc<Outbox>,
        authorize: Authorize,
        gate: Arc<Mutex<()>>,
        wake: Arc<Notify>,
        changed: Changed,
        replay_limits: ReplayLimits,
        options: FileOverlayOptions,
    ) -> Self {
        Self {
            inner,
            queue,
            routes: Arc::new(options.routes),
            authorize,
            gate,
            wake,
            changed,
            buffering: options.buffering,
            max_file_bytes: options.max_file_bytes,
            replay_limits,
            offline_error: options.offline_error,
        }
    }
    fn target(&self, path: &ObjectPath) -> object_store::Result<Target<'_>> {
        for route in self.routes.iter() {
            let Some(relative) = path.as_ref().strip_prefix(&route.root) else {
                continue;
            };
            if !relative.starts_with(&route.prefix) {
                continue;
            }
            if relative.split('/').any(|part| part.ends_with(".lance"))
                || (matches!(
                    route.purpose,
                    StoragePurpose::Storage | StoragePurpose::User
                ) && relative.split('/').next() == Some("db"))
            {
                return Ok(Target::Lance);
            }
            if table_format_commit(relative) {
                return Ok(Target::Commit);
            }
            let resource = OfflineResource::File {
                purpose: route.purpose,
                path: relative.into(),
            };
            let key = serde_json::to_string(&resource).map_err(error)?;
            return Ok(Target::Buffered {
                key,
                resource,
                route,
            });
        }
        Ok(Target::Outside)
    }
    /// The queue key of a buffered path. Lance paths are rejected as in the standalone.
    fn resource(
        &self,
        path: &ObjectPath,
    ) -> object_store::Result<Option<(String, OfflineResource, &FileRoute)>> {
        match self.target(path)? {
            Target::Lance => Err(unsupported(LANCE_OBJECTS)),
            Target::Buffered {
                key,
                resource,
                route,
            } => Ok(Some((key, resource, route))),
            Target::Outside | Target::Commit => Ok(None),
        }
    }
    fn connectivity(&self) -> Option<&Arc<dyn Connectivity>> {
        match &self.buffering {
            FileBuffering::Always => None,
            FileBuffering::WhenOffline { connectivity, .. } => Some(connectivity),
        }
    }
    fn is_offline(&self) -> bool {
        self.connectivity()
            .is_some_and(|connectivity| connectivity.is_offline())
    }
    fn observe(&self, observation: Observation) {
        if let Some(connectivity) = self.connectivity() {
            connectivity.observe(observation);
        }
    }
    /// `WhenOffline` passes Lance paths through while the breaker is closed.
    fn lance_passes(&self) -> bool {
        self.connectivity().is_some() && !self.is_offline()
    }
    /// Table-format commits go to the cloud; offline they fail with E22.
    async fn commit<T>(
        &self,
        path: &ObjectPath,
        call: impl Future<Output = object_store::Result<T>>,
    ) -> object_store::Result<T> {
        if self.is_offline() {
            return Err(table_format_offline(path));
        }
        call.await.map_err(|failure| {
            if (self.offline_error)(&failure).is_some() {
                table_format_offline(path)
            } else {
                failure
            }
        })
    }
    fn latest(
        &self,
        resource: &str,
    ) -> object_store::Result<Option<(QueuedOperation, OfflineReplayRequest)>> {
        (self.authorize)().map_err(error)?;
        self.queue
            .latest_pending(resource)
            .map_err(error)?
            .map(|mut operation| {
                let request = serde_json::from_value(std::mem::take(&mut operation.payload))
                    .map_err(error)?;
                Ok((operation, request))
            })
            .transpose()
    }
    fn pending_meta(
        path: &ObjectPath,
        operation: &QueuedOperation,
        request: &OfflineReplayRequest,
    ) -> object_store::Result<Option<ObjectMeta>> {
        match &request.mutation {
            OfflineMutation::FileDelete => Ok(None),
            OfflineMutation::FilePut { data_base64, .. } => {
                let padding = data_base64
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|byte| **byte == b'=')
                    .count();
                let size = (data_base64.len() / 4 * 3).saturating_sub(padding) as u64;
                Ok(Some(ObjectMeta {
                    location: path.clone(),
                    last_modified: (UNIX_EPOCH
                        + Duration::from_secs(operation.created_at.max(0) as u64))
                    .into(),
                    size,
                    e_tag: Some(format!("offline-{}", operation.operation_id)),
                    version: Some(format!("offline-{}", operation.operation_id)),
                }))
            }
            _ => Err(error("Non-file mutation in file overlay")),
        }
    }
    /// What the queue holds for `path`.
    pub fn pending(&self, path: &ObjectPath) -> object_store::Result<PendingFile> {
        let Target::Buffered { key, .. } = self.target(path)? else {
            return Ok(PendingFile::None);
        };
        Ok(match self.latest(&key)? {
            None => PendingFile::None,
            Some((operation, request)) => match Self::pending_meta(path, &operation, &request)? {
                Some(meta) => PendingFile::Put { meta },
                None => PendingFile::Deleted,
            },
        })
    }
    /// Digest-checked bytes of the latest queued put of `path`.
    pub fn pending_bytes(&self, path: &ObjectPath) -> object_store::Result<Option<Bytes>> {
        let Target::Buffered { key, .. } = self.target(path)? else {
            return Ok(None);
        };
        match self.latest(&key)? {
            Some((
                _,
                OfflineReplayRequest {
                    mutation:
                        OfflineMutation::FilePut {
                            data_base64,
                            sha256,
                        },
                    ..
                },
            )) => decode_put(&data_base64, &sha256).map(Some).map_err(error),
            _ => Ok(None),
        }
    }
    async fn queue_mutation(
        &self,
        path: &ObjectPath,
        mutation: OfflineMutation,
        mode: &PutMode,
        in_flight: bool,
    ) -> object_store::Result<PutResult> {
        let Some((key, resource, route)) = self.resource(path)? else {
            return Err(unsupported(
                "File is outside the configured buffered directories",
            ));
        };
        let _guard = self.gate.lock().await;
        (self.authorize)().map_err(error)?;
        let pending = self.latest(&key)?;
        let visible = match &pending {
            Some((operation, request)) => Self::pending_meta(path, operation, request)?,
            None => match self.inner.head(path).await {
                Ok(meta) => Some(meta),
                Err(object_store::Error::NotFound { .. }) => None,
                Err(failure)
                    if matches!(mutation, OfflineMutation::FilePut { .. })
                        && (self.offline_error)(&failure).is_some() =>
                {
                    match mode {
                        PutMode::Create => return Err(unknown_existence(path)),
                        // A cold offline PUT is a conditional create. If the cloud
                        // already has this path, replay exposes a conflict.
                        PutMode::Overwrite => None,
                        PutMode::Update(_) => return Err(failure),
                    }
                }
                Err(failure) => return Err(failure),
            },
        };
        match mode {
            PutMode::Create if visible.is_some() => {
                return Err(object_store::Error::AlreadyExists {
                    path: path.to_string(),
                    source: "File already exists in the device view".into(),
                });
            }
            PutMode::Update(version) => {
                let matches = visible.as_ref().is_some_and(|meta| {
                    (version.e_tag.is_some() || version.version.is_some())
                        && version
                            .e_tag
                            .as_ref()
                            .is_none_or(|value| meta.e_tag.as_ref() == Some(value))
                        && version
                            .version
                            .as_ref()
                            .is_none_or(|value| meta.version.as_ref() == Some(value))
                });
                if !matches {
                    return Err(object_store::Error::Precondition {
                        path: path.to_string(),
                        source: "File changed in the device view".into(),
                    });
                }
            }
            _ => (),
        }
        if pending.is_none() {
            let expected = visible
                .as_ref()
                .map_or(OfflineExpected::FileAbsent, |meta| {
                    OfflineExpected::FileRevision {
                        e_tag: meta.e_tag.clone(),
                        version: meta.version.clone(),
                    }
                });
            self.queue
                .initialize_resource(&key, &serde_json::to_value(&expected).map_err(error)?, None)
                .map_err(error)?;
            self.queue
                .replace_resource_revision_if_idle(
                    &key,
                    &serde_json::to_value(&expected).map_err(error)?,
                )
                .map_err(error)?;
        }
        let expected: OfflineExpected = serde_json::from_value(
            self.queue
                .resource_revision(&key)
                .map_err(error)?
                .ok_or_else(|| error("Missing file base revision"))?,
        )
        .map_err(error)?;
        if matches!(
            expected,
            OfflineExpected::FileRevision {
                e_tag: None,
                version: None
            }
        ) {
            return Err(unknown_revision(path));
        }
        let replacing_undispatched =
            pending.is_some() && self.queue.can_coalesce(&key, "file").map_err(error)?;
        // Existing-object operations require provider generations/ETags that the
        // replay API can bind without mistaking delete-and-recreate for a retry.
        let replayable = match (&*route.scheme, &expected) {
            ("az" | "gs", OfflineExpected::FileAbsent) => true,
            ("az", OfflineExpected::FileRevision { e_tag, .. }) => {
                e_tag.as_ref().is_some_and(|tag| !tag.is_empty())
            }
            ("gs", OfflineExpected::FileRevision { version, .. }) => version
                .as_ref()
                .and_then(|version| version.parse::<u64>().ok())
                .is_some_and(|version| version > 0),
            ("s3", OfflineExpected::FileAbsent) => pending.is_none() || replacing_undispatched,
            _ => false,
        };
        if !replayable {
            if in_flight && route.scheme == "s3" {
                return Err(still_uploading(path));
            }
            return Err(unsupported(UNREPLAYABLE));
        }
        let request = OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource,
            expected,
            mutation,
        };
        validate_request(&request, &self.replay_limits.get())
            .map_err(|failure| error(anyhow::Error::from(failure).to_string()))?;
        let operation = self
            .queue
            .enqueue(
                &key,
                serde_json::to_value(&request).map_err(error)?,
                Some("file"),
                unix_time().map_err(error)?,
            )
            .map_err(error)?;
        self.queue
            .mark_local(&operation.operation_id, 1)
            .map_err(error)?;
        self.wake.notify_one();
        (self.changed)();
        let revision = format!("offline-{}", operation.operation_id);
        Ok(PutResult {
            e_tag: Some(revision.clone()),
            version: Some(revision),
        })
    }
    /// `WhenOffline` for a buffered path.
    async fn put_when_offline(
        &self,
        path: &ObjectPath,
        key: &str,
        payload: PutPayload,
        options: PutOptions,
        mutation: OfflineMutation,
    ) -> object_store::Result<PutResult> {
        let FileBuffering::WhenOffline {
            pending_settle_timeout,
            ..
        } = &self.buffering
        else {
            return self
                .queue_mutation(path, mutation, &options.mode, false)
                .await;
        };
        let offline = self.is_offline();
        if self.queue.has_pending(key).map_err(error)? {
            let in_flight = !offline
                && self
                    .queue
                    .pending_states(key)
                    .map_err(error)?
                    .iter()
                    .all(|state| state == "attempting");
            if in_flight {
                if wait_idle(&self.queue, key, *pending_settle_timeout).await {
                    return self.direct_put(path, payload, options, mutation).await;
                }
                return self
                    .queue_mutation(path, mutation, &options.mode, true)
                    .await;
            }
            return self
                .queue_mutation(path, mutation, &options.mode, false)
                .await;
        }
        if offline {
            return self
                .queue_mutation(path, mutation, &options.mode, false)
                .await;
        }
        self.direct_put(path, payload, options, mutation).await
    }
    async fn direct_put(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
        mutation: OfflineMutation,
    ) -> object_store::Result<PutResult> {
        let FileBuffering::WhenOffline {
            direct_write_base,
            direct_write_min_bytes_per_second,
            ..
        } = &self.buffering
        else {
            return self
                .queue_mutation(path, mutation, &options.mode, false)
                .await;
        };
        let timeout = *direct_write_base
            + Duration::from_secs_f64(
                payload.content_length() as f64
                    / (*direct_write_min_bytes_per_second).max(1) as f64,
            );
        let mode = options.mode.clone();
        match tokio::time::timeout(timeout, self.inner.put_opts(path, payload, options)).await {
            Ok(Ok(result)) => {
                self.observe(Observation::Succeeded);
                Ok(result)
            }
            Ok(Err(failure)) => match (self.offline_error)(&failure) {
                Some(observation) => {
                    self.observe(observation);
                    self.queue_mutation(path, mutation, &mode, false).await
                }
                None => Err(failure),
            },
            Err(_) => {
                self.observe(Observation::TimedOut);
                self.queue_mutation(path, mutation, &mode, false).await
            }
        }
    }
    async fn delete_one(&self, path: &ObjectPath) -> object_store::Result<()> {
        let key = match self.target(path)? {
            Target::Outside => return self.inner.delete(path).await,
            Target::Lance if self.lance_passes() => return self.inner.delete(path).await,
            Target::Lance => return Err(unsupported(LANCE_OBJECTS)),
            Target::Commit => return self.commit(path, self.inner.delete(path)).await,
            Target::Buffered { key, .. } => key,
        };
        let direct = matches!(self.buffering, FileBuffering::WhenOffline { .. })
            && !self.is_offline()
            && !self.queue.has_pending(&key).map_err(error)?;
        if direct {
            match self.inner.delete(path).await {
                Ok(()) => {
                    self.observe(Observation::Succeeded);
                    return Ok(());
                }
                Err(failure) => match (self.offline_error)(&failure) {
                    Some(observation) => self.observe(observation),
                    None => return Err(failure),
                },
            }
        }
        self.queue_mutation(
            path,
            OfflineMutation::FileDelete,
            &PutMode::Overwrite,
            false,
        )
        .await?;
        Ok(())
    }
    /// Copy and rename refuse buffered paths that are offline or pending.
    fn transfer_blocked(&self, path: &ObjectPath) -> object_store::Result<bool> {
        if self.connectivity().is_none() {
            return Ok(self.resource(path)?.is_some());
        }
        Ok(match self.target(path)? {
            Target::Outside | Target::Commit => false,
            Target::Lance if self.is_offline() => return Err(unsupported(LANCE_OBJECTS)),
            Target::Lance => false,
            Target::Buffered { key, .. } => {
                self.is_offline() || self.queue.has_pending(&key).map_err(error)?
            }
        })
    }
    fn is_commit(&self, path: &ObjectPath) -> object_store::Result<bool> {
        Ok(matches!(self.target(path)?, Target::Commit))
    }
}

#[async_trait]
impl ObjectStore for FileOverlay {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        let key = match self.target(path)? {
            Target::Outside => return self.inner.put_opts(path, payload, options).await,
            Target::Lance if self.lance_passes() => {
                return self.inner.put_opts(path, payload, options).await;
            }
            Target::Lance => return Err(unsupported(LANCE_OBJECTS)),
            Target::Commit => {
                return self
                    .commit(path, self.inner.put_opts(path, payload, options))
                    .await;
            }
            Target::Buffered { key, .. } => key,
        };
        if options.attributes != Attributes::default() || options.tags != Default::default() {
            return Err(unsupported(
                "Offline file writes do not support object attributes or tags",
            ));
        }
        if payload.content_length() > self.max_file_bytes {
            return Err(error(format!(
                "Offline file payload exceeds {}",
                format_limit(self.max_file_bytes)
            )));
        }
        let mut data = BytesMut::with_capacity(payload.content_length());
        for chunk in payload.clone() {
            data.extend_from_slice(&chunk);
        }
        let mutation = OfflineMutation::FilePut {
            data_base64: STANDARD.encode(&data),
            sha256: format!("{:x}", Sha256::digest(&data)),
        };
        match self.buffering {
            FileBuffering::Always => {
                self.queue_mutation(path, mutation, &options.mode, false)
                    .await
            }
            FileBuffering::WhenOffline { .. } => {
                self.put_when_offline(path, &key, payload, options, mutation)
                    .await
            }
        }
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        let key = match self.target(path)? {
            Target::Outside => return self.inner.put_multipart_opts(path, options).await,
            Target::Lance if self.lance_passes() => {
                return self.inner.put_multipart_opts(path, options).await;
            }
            Target::Lance => return Err(unsupported(LANCE_OBJECTS)),
            Target::Commit => {
                return self
                    .commit(path, self.inner.put_multipart_opts(path, options))
                    .await;
            }
            Target::Buffered { key, .. } => key,
        };
        if options.attributes != Attributes::default() || options.tags != Default::default() {
            return Err(unsupported(
                "Offline multipart writes do not support object attributes or tags",
            ));
        }
        if matches!(self.buffering, FileBuffering::WhenOffline { .. })
            && !self.is_offline()
            && !self.queue.has_pending(&key).map_err(error)?
        {
            return self.inner.put_multipart_opts(path, options).await;
        }
        Ok(Box::new(FileUpload {
            store: self.clone(),
            path: path.clone(),
            bytes: BytesMut::new(),
            failed: false,
        }))
    }
    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        let key = match self.target(path)? {
            Target::Buffered { key, .. } => key,
            Target::Lance if self.connectivity().is_none() => {
                return Err(unsupported(LANCE_OBJECTS));
            }
            _ => return self.inner.get_opts(path, options).await,
        };
        (self.authorize)().map_err(error)?;
        if options
            .version
            .as_ref()
            .is_some_and(|version| !version.starts_with("offline-"))
        {
            // An explicit cloud version selects that version even while a
            // newer local write or deletion awaits replay.
            return self.inner.get_opts(path, options).await;
        }
        if let Some((operation, request)) = self.latest(&key)? {
            let meta =
                Self::pending_meta(path, &operation, &request)?.ok_or_else(|| missing(path))?;
            options.check_preconditions(&meta)?;
            if options
                .version
                .as_ref()
                .is_some_and(|version| meta.version.as_ref() != Some(version))
            {
                return Err(missing(path));
            }
            let range = if options.head {
                0..0
            } else {
                options
                    .range
                    .as_ref()
                    .map(|range| range.as_range(meta.size))
                    .transpose()
                    .map_err(error)?
                    .unwrap_or(0..meta.size)
            };
            let OfflineMutation::FilePut {
                data_base64,
                sha256,
            } = request.mutation
            else {
                return Err(missing(path));
            };
            let data = if options.head {
                Bytes::new()
            } else {
                decode_put(&data_base64, &sha256)
                    .map_err(error)?
                    .slice(range.start as usize..range.end as usize)
            };
            let authorize = self.authorize.clone();
            return Ok(GetResult {
                payload: GetResultPayload::Stream(Box::pin(futures_util::stream::once(
                    async move {
                        authorize().map_err(error)?;
                        Ok(data)
                    },
                ))),
                meta,
                range,
                attributes: Attributes::default(),
            });
        }
        self.inner.get_opts(path, options).await
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let store = self.clone();
        Box::pin(paths.then(move |path| {
            let store = store.clone();
            async move {
                let path = path?;
                store.delete_one(&path).await?;
                Ok(path)
            }
        }))
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let store = self.clone();
        let prefix = prefix.cloned();
        Box::pin(try_stream! {
            (store.authorize)().map_err(error)?;
            let mut cloud = store.inner.list(prefix.as_ref());
            while let Some(meta) = cloud.next().await {
                let meta = meta?;
                (store.authorize)().map_err(error)?;
                let key = match store.target(&meta.location)? {
                    Target::Buffered { key, .. } => Some(key),
                    Target::Lance if store.connectivity().is_none() => store.resource(&meta.location)?.map(|(key, _, _)| key),
                    _ => None,
                };
                if let Some(key) = key {
                    if store.latest(&key)?.is_some() { continue; }
                }
                yield meta;
            }
            let mut after = None;
            loop {
                let resources = store.queue.pending_file_resources(after.as_deref(), 64).map_err(error)?;
                if resources.is_empty() { break; }
                for key in &resources {
                    let resource: OfflineResource = serde_json::from_str(key).map_err(error)?;
                    let OfflineResource::File { purpose, path } = resource else { continue };
                    let Some(route) = store.routes.iter().find(|route| route.purpose == purpose && path.starts_with(&route.prefix)) else { continue };
                    let path = ObjectPath::parse(format!("{}{path}", route.root)).map_err(error)?;
                    if prefix.as_ref().is_some_and(|prefix| !path.prefix_matches(prefix)) { continue; }
                    if let Some((operation, request)) = store.latest(key)? {
                        if let Some(meta) = Self::pending_meta(&path, &operation, &request)? {
                            (store.authorize)().map_err(error)?;
                            yield meta;
                        }
                    }
                }
                after = resources.last().cloned();
            }
        })
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        let mut objects = Vec::new();
        let mut prefixes = BTreeSet::new();
        let mut bytes = 0usize;
        let root = prefix.map_or(String::new(), |prefix| format!("{prefix}/"));
        let mut stream = self.list(prefix);
        while let Some(meta) = stream.next().await {
            let meta = meta?;
            bytes += meta.location.as_ref().len() + 128;
            if bytes > 8 * 1024 * 1024 {
                return Err(error(
                    "Offline file listing exceeds 8 MiB; use a narrower prefix",
                ));
            }
            let Some(relative) = meta.location.as_ref().strip_prefix(&root) else {
                continue;
            };
            if let Some((directory, _)) = relative.split_once('/') {
                prefixes.insert(ObjectPath::parse(format!("{root}{directory}")).map_err(error)?);
            } else {
                objects.push(meta);
            }
        }
        Ok(ListResult {
            common_prefixes: prefixes.into_iter().collect(),
            objects,
        })
    }
    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        if self.transfer_blocked(from)? || self.transfer_blocked(to)? {
            return Err(unsupported(COPY));
        }
        if self.is_commit(from)? || self.is_commit(to)? {
            return self
                .commit(to, self.inner.copy_opts(from, to, options))
                .await;
        }
        self.inner.copy_opts(from, to, options).await
    }
    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        if self.transfer_blocked(from)? || self.transfer_blocked(to)? {
            return Err(unsupported(RENAME));
        }
        if self.is_commit(from)? || self.is_commit(to)? {
            return self
                .commit(to, self.inner.rename_opts(from, to, options))
                .await;
        }
        self.inner.rename_opts(from, to, options).await
    }
}

#[derive(Debug)]
struct FileUpload {
    store: FileOverlay,
    path: ObjectPath,
    bytes: BytesMut,
    failed: bool,
}
#[async_trait]
impl MultipartUpload for FileUpload {
    fn put_part(&mut self, payload: PutPayload) -> UploadPart {
        let limit = self.store.max_file_bytes;
        if self.failed || self.bytes.len().saturating_add(payload.content_length()) > limit {
            self.failed = true;
            return Box::pin(async move {
                Err(error(format!(
                    "Offline multipart payload exceeds {}",
                    format_limit(limit)
                )))
            });
        }
        for chunk in payload {
            self.bytes.extend_from_slice(&chunk);
        }
        Box::pin(async { Ok(()) })
    }
    async fn complete(&mut self) -> object_store::Result<PutResult> {
        if self.failed {
            return Err(error(
                "Offline multipart upload was aborted or exceeded its limit",
            ));
        }
        self.failed = true;
        self.store
            .put_opts(
                &self.path,
                std::mem::take(&mut self.bytes).freeze().into(),
                PutOptions::default(),
            )
            .await
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        self.bytes.clear();
        self.failed = true;
        Ok(())
    }
}

pub(crate) fn file_path(
    manager: &WriteManager,
    resource: &OfflineResource,
) -> Result<Option<ObjectPath>> {
    let OfflineResource::File { purpose, path } = resource else {
        return Ok(None);
    };
    let prefix = manager
        .host
        .location_prefix(*purpose)
        .context("Acknowledged file is outside the authorized scope")?;
    Ok(Some(ObjectPath::parse(format!("{prefix}{path}"))?))
}

pub(crate) fn remember_acknowledged(
    manager: &WriteManager,
    request: &OfflineReplayRequest,
    revision: &OfflineExpected,
) -> Result<()> {
    let Some(path) = file_path(manager, &request.resource)? else {
        return Ok(());
    };
    match (&request.mutation, revision) {
        (
            OfflineMutation::FilePut {
                data_base64,
                sha256,
            },
            OfflineExpected::FileRevision { .. },
        ) => {
            let bytes = STANDARD.decode(data_base64)?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == *sha256,
                "Acknowledged file payload digest differs"
            );
            manager
                .host
                .file_acknowledged(&path, &request.operation_id, &bytes, revision)?;
        }
        (OfflineMutation::FileDelete, OfflineExpected::FileAbsent) => {
            manager.host.file_deleted(&path, &request.operation_id)?
        }
        _ => anyhow::bail!("Acknowledged file revision does not match its mutation"),
    }
    Ok(())
}

pub(crate) fn payload_bytes(request: &OfflineReplayRequest) -> Result<Option<Bytes>> {
    match &request.mutation {
        OfflineMutation::FilePut {
            data_base64,
            sha256,
        } => decode_put(data_base64, sha256).map(Some),
        _ => Ok(None),
    }
}
