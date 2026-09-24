use super::{ProjectCredentials, private_cache};
use crate::{
    config::{PlacementConfig, WorkloadIdentity},
    enrollment::unix_time,
};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_device_protocol::{InstanceStorageLocation, OnlineProjectAccess, StoragePurpose};
use flow_like_storage::object_store::{
    ObjectStore, ObjectStoreExt, memory::InMemory, path::Path as ObjectPath,
};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 48 * 1024 * 1024;
const MAX_METADATA_FILES: usize = 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotClaim {
    pub binding: String,
    pub digest: String,
    pub storage_context_digest: String,
    pub grant_expires_at: i64,
}

impl SnapshotClaim {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            [&self.binding, &self.digest, &self.storage_context_digest]
                .iter()
                .all(|s| s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())),
            "Invalid outage snapshot digest"
        );
        ensure!(
            self.grant_expires_at > 0,
            "Cached project authorization has expired; reconnect to authorize this placement"
        );
        Ok(())
    }
}

/// The supervisor authenticates local checkpoints without exposing its key to workflows.
#[async_trait]
pub trait OutageAuthority: Send + Sync {
    async fn seal(&self, claim: &SnapshotClaim) -> Result<String>;
    async fn verify(&self, claim: &SnapshotClaim, seal: &str) -> Result<()>;
    async fn deny(&self, binding: &str) -> Result<()>;
}

pub(crate) fn binding(
    config: &PlacementConfig,
    identity: &WorkloadIdentity,
    api_base: &str,
) -> Result<String> {
    let mut canonical = config.clone();
    // Artifact paths differ between host and sandbox; the immutable revision and
    // all pins remain part of the authenticated configuration.
    canonical.project_path = PathBuf::from("/");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "policy": "validated_cache_reads_and_explicit_logical_outbox_v1",
        "configuration": canonical,
        "api_base": api_base,
        "device_id": identity.device_id,
        "device_auth_epoch": identity.device_auth_epoch,
        "key_epoch": identity.key_epoch,
    }))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StorageContext {
    pub locations: BTreeMap<StoragePurpose, InstanceStorageLocation>,
    pub delegating_user_id: String,
    pub access: OnlineProjectAccess,
    pub scope: String,
    pub grant_expires_at: i64,
}

impl StorageContext {
    fn digest(&self) -> Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(self)?)
            .to_hex()
            .to_string())
    }
}

pub(crate) fn authenticated_context_digest(
    config: &PlacementConfig,
    identity: &WorkloadIdentity,
    api_base: &str,
    lease: &flow_like_device_protocol::InstanceStorageLease,
) -> Result<String> {
    super::validate_lease(config, identity, lease)?;
    StorageContext {
        locations: lease.locations.clone(),
        delegating_user_id: lease.delegating_user_id.clone(),
        access: lease.access,
        scope: super::cache_scope(config, identity, lease, api_base)?,
        grant_expires_at: lease
            .grant_expires_at
            .context("The API must provide a validated grant deadline for outage recovery")?,
    }
    .digest()
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetadataFile {
    path: String,
    content: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u8,
    binding: String,
    context: StorageContext,
    metadata: Vec<MetadataFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    snapshot: Snapshot,
    seal: String,
}

pub(super) struct Restored {
    pub context: StorageContext,
    pub metadata: Arc<dyn ObjectStore>,
}

pub(super) struct SnapshotStore {
    root: PathBuf,
    cache_root: PathBuf,
    placement: String,
    project: String,
    binding: String,
    authority: Arc<dyn OutageAuthority>,
}

impl SnapshotStore {
    pub(super) fn new(
        root: &Path,
        config: &PlacementConfig,
        identity: &WorkloadIdentity,
        base: &str,
        authority: Arc<dyn OutageAuthority>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            root: private_cache(root, &config.id, "outage")?,
            cache_root: root.to_path_buf(),
            placement: config.id.clone(),
            project: config.project_id.clone(),
            binding: binding(config, identity, base)?,
            authority,
        }))
    }

    pub(super) async fn persist(
        &self,
        credentials: &ProjectCredentials,
        metadata: &Arc<dyn ObjectStore>,
    ) -> Result<()> {
        let grant_expires_at = credentials.grant_expires_at.context("This API does not provide grant expiry for restart-safe outage recovery; update the API")?;
        let mut total = 0usize;
        let mut files = Vec::new();
        let mut objects = metadata.list(None);
        while let Some(object) = objects.try_next().await? {
            ensure!(
                files.len() < MAX_METADATA_FILES,
                "Pinned metadata exceeds the outage file limit"
            );
            ensure!(
                object.size <= MAX_METADATA_BYTES as u64,
                "Pinned metadata exceeds the outage size limit"
            );
            let content = metadata.get(&object.location).await?.bytes().await?;
            total = total
                .checked_add(content.len())
                .context("Metadata size overflow")?;
            ensure!(
                total <= MAX_METADATA_BYTES,
                "Pinned metadata exceeds the 32 MiB outage limit"
            );
            files.push(MetadataFile {
                path: object.location.to_string(),
                content: STANDARD.encode(content),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let snapshot = Snapshot {
            version: 1,
            binding: self.binding.clone(),
            context: StorageContext {
                locations: credentials.locations.clone(),
                delegating_user_id: credentials.delegating_user_id.clone(),
                access: credentials.access,
                scope: credentials.scope.clone(),
                grant_expires_at,
            },
            metadata: files,
        };
        let claim = claim(&snapshot)?;
        claim.validate()?;
        ensure!(grant_expires_at > unix_time()?, "Cached grant has expired");
        let seal = self.authority.seal(&claim).await?;
        let bytes = serde_json::to_vec(&Envelope { snapshot, seal })?;
        ensure!(
            bytes.len() <= MAX_SNAPSHOT_BYTES,
            "Outage snapshot exceeds its size limit"
        );
        let _lock = snapshot_lock(&self.root)?;
        credentials.authorization_current()?;
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            if name
                .to_str()
                .and_then(|name| name.strip_suffix(".partial"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                ensure!(
                    entry.file_type()?.is_file(),
                    "Invalid snapshot staging entry"
                );
                std::fs::remove_file(entry.path())?;
            }
        }
        let temporary = self.root.join(format!("{}.partial", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            crate::vault::write_new_private(&temporary, &bytes)?;
            std::fs::rename(&temporary, self.root.join("snapshot.json"))?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    pub(super) async fn restore(&self) -> Result<Restored> {
        let bytes = read_snapshot(&self.root.join("snapshot.json"))
            .context("No valid initialized outage snapshot is available; reconnect this placement to the API")?;
        let envelope: Envelope =
            serde_json::from_slice(&bytes).context("Corrupted outage snapshot")?;
        ensure!(
            envelope.snapshot.version == 1 && envelope.snapshot.binding == self.binding,
            "Cached online deployment identity changed; reconnect to authorize this configuration"
        );
        let claim = claim(&envelope.snapshot)?;
        claim.validate()?;
        if let Err(error) = self.authority.verify(&claim, &envelope.seal).await {
            if error.downcast_ref::<flow_like_types_contracts::authorization::AuthorizationError>()
                == Some(&flow_like_types_contracts::authorization::AuthorizationError::Denied)
            {
                self.revoke().await?;
            }
            return Err(error);
        }
        let metadata = Arc::new(InMemory::new());
        let mut total = 0usize;
        let mut previous: Option<&str> = None;
        ensure!(
            !envelope.snapshot.metadata.is_empty()
                && envelope.snapshot.metadata.len() <= MAX_METADATA_FILES,
            "Invalid outage metadata inventory"
        );
        for file in &envelope.snapshot.metadata {
            ensure!(
                file.path.starts_with(&format!("apps/{}/", self.project))
                    && previous.is_none_or(|path| path < file.path.as_str()),
                "Invalid outage metadata path or duplicate"
            );
            previous = Some(&file.path);
            let path = ObjectPath::parse(&file.path)?;
            let content = STANDARD.decode(&file.content)?;
            total = total
                .checked_add(content.len())
                .context("Metadata size overflow")?;
            ensure!(
                total <= MAX_METADATA_BYTES,
                "Outage metadata exceeds its limit"
            );
            metadata.put(&path, content.into()).await?;
        }
        Ok(Restored {
            context: envelope.snapshot.context,
            metadata,
        })
    }

    pub(super) async fn revoke(&self) -> Result<()> {
        // The supervisor fence remains durable even if child-controlled cache
        // files were damaged. Attempt every cleanup before returning an error.
        let denied = self.authority.deny(&self.binding).await;
        let cache = super::cache::CacheControl::revoke_placement(&self.cache_root, &self.placement);
        let removed = (|| -> Result<()> {
            let _lock = snapshot_lock(&self.root)?;
            match std::fs::remove_file(self.root.join("snapshot.json")) {
                Ok(()) => File::open(&self.root)?.sync_all()?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })();
        denied.and(cache).and(removed)
    }
}

fn snapshot_lock(root: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let lock = options.open(root.join("snapshot.lock"))?;
    ensure!(lock.metadata()?.is_file(), "Invalid snapshot lock");
    // Only bounded local file work holds this lock; network calls precede it.
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}

pub(crate) fn remove_snapshot(root: &Path, placement: &str) -> Result<()> {
    let parent = root
        .join(".standalone-cache")
        .join(placement)
        .join("outage");
    if !parent.try_exists()? {
        return Ok(());
    }
    super::validate_snapshot_parent(root, placement)?;
    let _lock = snapshot_lock(&parent)?;
    match std::fs::remove_file(parent.join("snapshot.json")) {
        Ok(()) => File::open(parent)?.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn claim(snapshot: &Snapshot) -> Result<SnapshotClaim> {
    Ok(SnapshotClaim {
        binding: snapshot.binding.clone(),
        digest: blake3::hash(&serde_json::to_vec(snapshot)?)
            .to_hex()
            .to_string(),
        storage_context_digest: snapshot.context.digest()?,
        grant_expires_at: snapshot.context.grant_expires_at,
    })
}

pub(crate) fn inspect_claim(
    path: &Path,
    expected_binding: &str,
) -> Result<(SnapshotClaim, String)> {
    let envelope: Envelope = serde_json::from_slice(&read_snapshot(path)?)?;
    ensure!(
        envelope.snapshot.version == 1 && envelope.snapshot.binding == expected_binding,
        "Cached deployment differs"
    );
    let claim = claim(&envelope.snapshot)?;
    claim.validate()?;
    Ok((claim, envelope.seal))
}

fn read_snapshot(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_SNAPSHOT_BYTES as u64,
        "Invalid outage snapshot file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() },
            "Outage snapshot must be private and owned by this user"
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_SNAPSHOT_BYTES,
        "Outage snapshot exceeds its limit"
    );
    Ok(bytes)
}

pub(super) struct FencedMetadata {
    inner: Arc<dyn ObjectStore>,
    credentials: Arc<ProjectCredentials>,
}
impl FencedMetadata {
    pub(super) fn new(inner: Arc<dyn ObjectStore>, credentials: Arc<ProjectCredentials>) -> Self {
        Self { inner, credentials }
    }
    fn authorize(&self) -> flow_like_storage::object_store::Result<()> {
        self.credentials.authorization_current().map_err(|error| {
            flow_like_storage::object_store::Error::PermissionDenied {
                path: "project metadata".into(),
                source: error.into(),
            }
        })
    }
}
impl std::fmt::Debug for FencedMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizedProjectMetadata")
    }
}
impl std::fmt::Display for FencedMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizedProjectMetadata")
    }
}
fn immutable() -> flow_like_storage::object_store::Error {
    flow_like_storage::object_store::Error::NotSupported {
        source: "Pinned project metadata is immutable".into(),
    }
}
#[async_trait]
impl ObjectStore for FencedMetadata {
    async fn put_opts(
        &self,
        _: &ObjectPath,
        _: flow_like_storage::object_store::PutPayload,
        _: flow_like_storage::object_store::PutOptions,
    ) -> flow_like_storage::object_store::Result<flow_like_storage::object_store::PutResult> {
        Err(immutable())
    }
    async fn put_multipart_opts(
        &self,
        _: &ObjectPath,
        _: flow_like_storage::object_store::PutMultipartOptions,
    ) -> flow_like_storage::object_store::Result<
        Box<dyn flow_like_storage::object_store::MultipartUpload>,
    > {
        Err(immutable())
    }
    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: flow_like_storage::object_store::GetOptions,
    ) -> flow_like_storage::object_store::Result<flow_like_storage::object_store::GetResult> {
        use flow_like_storage::object_store::{GetResult, GetResultPayload};
        use futures_util::StreamExt;
        self.authorize()?;
        let result = self.inner.get_opts(path, options).await?;
        self.authorize()?;
        let meta = result.meta.clone();
        let range = result.range.clone();
        let attributes = result.attributes.clone();
        let credentials = self.credentials.clone();
        let stream = result.into_stream().map(move |chunk| {
            credentials.authorization_current().map_err(|error| {
                flow_like_storage::object_store::Error::PermissionDenied {
                    path: "project metadata".into(),
                    source: error.into(),
                }
            })?;
            chunk
        });
        Ok(GetResult {
            meta,
            range,
            attributes,
            payload: GetResultPayload::Stream(Box::pin(stream)),
        })
    }
    fn delete_stream(
        &self,
        _: futures_util::stream::BoxStream<
            'static,
            flow_like_storage::object_store::Result<ObjectPath>,
        >,
    ) -> futures_util::stream::BoxStream<'static, flow_like_storage::object_store::Result<ObjectPath>>
    {
        Box::pin(futures_util::stream::once(async { Err(immutable()) }))
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> futures_util::stream::BoxStream<
        'static,
        flow_like_storage::object_store::Result<flow_like_storage::object_store::ObjectMeta>,
    > {
        use futures_util::StreamExt;
        let credentials = self.credentials.clone();
        Box::pin(self.inner.list(prefix).map(move |item| {
            credentials.authorization_current().map_err(|error| {
                flow_like_storage::object_store::Error::PermissionDenied {
                    path: "project metadata".into(),
                    source: error.into(),
                }
            })?;
            item
        }))
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> flow_like_storage::object_store::Result<flow_like_storage::object_store::ListResult> {
        self.authorize()?;
        let result = self.inner.list_with_delimiter(prefix).await?;
        self.authorize()?;
        Ok(result)
    }
    async fn copy_opts(
        &self,
        _: &ObjectPath,
        _: &ObjectPath,
        _: flow_like_storage::object_store::CopyOptions,
    ) -> flow_like_storage::object_store::Result<()> {
        Err(immutable())
    }
}
