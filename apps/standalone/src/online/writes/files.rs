use super::WriteManager;
use crate::outbox::{BufferedFiles, Outbox, QueuedOperation};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Bytes, BytesMut};
use flow_like_device_protocol::{
    MAX_OFFLINE_OPERATION_BYTES, OfflineExpected, OfflineMutation, OfflineReplayRequest,
    OfflineResource, StoragePurpose,
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
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
struct Route {
    purpose: StoragePurpose,
    root: String,
    prefix: String,
    scheme: String,
}

#[derive(Clone)]
struct FileOverlay {
    inner: Arc<dyn ObjectStore>,
    queue: Arc<Outbox>,
    routes: Arc<Vec<Route>>,
    authorize: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    gate: Arc<Mutex<()>>,
    wake: Arc<Notify>,
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
fn unsupported(message: &'static str) -> object_store::Error {
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

pub(in crate::online) fn wrap(
    inner: Arc<dyn ObjectStore>,
    manager: Arc<WriteManager>,
    selected: &[BufferedFiles],
) -> Result<Arc<dyn ObjectStore>> {
    let mut routes = Vec::new();
    for selected in selected {
        let location = manager
            .credentials
            .locations
            .get(&selected.purpose)
            .context("Missing buffered file authorization")?;
        routes.push(Route {
            purpose: selected.purpose,
            root: location.prefix.clone(),
            prefix: format!("{}/", selected.prefix),
            scheme: url::Url::parse(&location.uri)?.scheme().into(),
        });
    }
    let queue = manager.queue.clone();
    let gate = manager.gate.clone();
    let wake = manager.wake.clone();
    Ok(Arc::new(FileOverlay {
        inner,
        queue,
        routes: Arc::new(routes),
        authorize: Arc::new(move || manager.authorize()),
        gate,
        wake,
    }))
}

impl FileOverlay {
    fn resource(
        &self,
        path: &ObjectPath,
    ) -> object_store::Result<Option<(String, OfflineResource, &Route)>> {
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
                return Err(unsupported(
                    "Lance objects cannot be buffered as file writes",
                ));
            }
            let resource = OfflineResource::File {
                purpose: route.purpose,
                path: relative.into(),
            };
            let key = serde_json::to_string(&resource).map_err(error)?;
            return Ok(Some((key, resource, route)));
        }
        Ok(None)
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
    async fn queue_mutation(
        &self,
        path: &ObjectPath,
        mutation: OfflineMutation,
        mode: &PutMode,
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
                Err(error)
                    if matches!(mode, PutMode::Create | PutMode::Overwrite)
                        && matches!(mutation, OfflineMutation::FilePut { .. })
                        && super::super::cache::is_offline(&error) =>
                {
                    // A cold offline PUT is a conditional create. If the cloud
                    // already has this path, replay exposes a conflict.
                    None
                }
                Err(error) => return Err(error),
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
            return Err(unsupported(
                "This cloud provider cannot replay conditional file overwrites or deletes; use a new immutable file name",
            ));
        }
        let request = OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource,
            expected,
            mutation,
        };
        request.validate().map_err(error)?;
        let operation = self
            .queue
            .enqueue(
                &key,
                serde_json::to_value(&request).map_err(error)?,
                Some("file"),
                crate::enrollment::unix_time().map_err(error)?,
            )
            .map_err(error)?;
        self.queue
            .mark_local(&operation.operation_id, 1)
            .map_err(error)?;
        self.wake.notify_one();
        let revision = format!("offline-{}", operation.operation_id);
        Ok(PutResult {
            e_tag: Some(revision.clone()),
            version: Some(revision),
        })
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
        if self.resource(path)?.is_none() {
            return self.inner.put_opts(path, payload, options).await;
        }
        if options.attributes != Attributes::default() || options.tags != Default::default() {
            return Err(unsupported(
                "Offline file writes do not support object attributes or tags",
            ));
        }
        if payload.content_length() > MAX_OFFLINE_OPERATION_BYTES {
            return Err(error("Offline file payload exceeds 8 MiB"));
        }
        let mut data = BytesMut::with_capacity(payload.content_length());
        for chunk in payload {
            data.extend_from_slice(&chunk);
        }
        let mutation = OfflineMutation::FilePut {
            data_base64: STANDARD.encode(&data),
            sha256: format!("{:x}", Sha256::digest(&data)),
        };
        self.queue_mutation(path, mutation, &options.mode).await
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        if self.resource(path)?.is_none() {
            return self.inner.put_multipart_opts(path, options).await;
        }
        if options.attributes != Attributes::default() || options.tags != Default::default() {
            return Err(unsupported(
                "Offline multipart writes do not support object attributes or tags",
            ));
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
        let Some((key, _, _)) = self.resource(path)? else {
            return self.inner.get_opts(path, options).await;
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
                let bytes = STANDARD.decode(data_base64).map_err(error)?;
                if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
                    return Err(error("Queued file payload digest differs"));
                }
                Bytes::from(bytes).slice(range.start as usize..range.end as usize)
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
                if store.resource(&path)?.is_some() {
                    store
                        .queue_mutation(&path, OfflineMutation::FileDelete, &PutMode::Overwrite)
                        .await?;
                } else {
                    store.inner.delete(&path).await?;
                }
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
                if let Some((key, _, _)) = store.resource(&meta.location)? {
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
        if self.resource(from)?.is_some() || self.resource(to)?.is_some() {
            return Err(unsupported(
                "Copy involving buffered files requires an explicit read followed by a write",
            ));
        }
        self.inner.copy_opts(from, to, options).await
    }
    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        if self.resource(from)?.is_some() || self.resource(to)?.is_some() {
            return Err(unsupported(
                "Rename involving buffered files is not an atomic offline operation",
            ));
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
        if self.failed
            || self.bytes.len().saturating_add(payload.content_length())
                > MAX_OFFLINE_OPERATION_BYTES
        {
            self.failed = true;
            return Box::pin(async { Err(error("Offline multipart payload exceeds 8 MiB")) });
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

pub(super) fn remember_acknowledged(
    manager: &WriteManager,
    request: &OfflineReplayRequest,
    revision: &OfflineExpected,
) -> Result<()> {
    let OfflineResource::File { purpose, path } = &request.resource else {
        return Ok(());
    };
    let location = manager
        .credentials
        .locations
        .get(purpose)
        .context("Acknowledged file is outside the authorized scope")?;
    let path = ObjectPath::parse(format!("{}{path}", location.prefix))?;
    match (&request.mutation, revision) {
        (
            OfflineMutation::FilePut {
                data_base64,
                sha256,
            },
            OfflineExpected::FileRevision { e_tag, version },
        ) => {
            let bytes = STANDARD.decode(data_base64)?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == *sha256,
                "Acknowledged file payload digest differs"
            );
            let meta = ObjectMeta {
                location: path,
                size: bytes.len() as u64,
                last_modified: std::time::SystemTime::now().into(),
                e_tag: e_tag.clone(),
                version: version.clone(),
            };
            manager.credentials.cache.remember_file(&meta, &bytes)?;
        }
        (OfflineMutation::FileDelete, OfflineExpected::FileAbsent) => {
            manager.credentials.cache.remember_file_deleted(&path)?
        }
        _ => anyhow::bail!("Acknowledged file revision does not match its mutation"),
    }
    Ok(())
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
