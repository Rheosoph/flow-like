//! Imported revisions bind a complete initial project snapshot. Runtime databases may change later.
use crate::{enrollment::unix_time, state::StateStore, supervisor, vault};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    ArtifactTransferState, ArtifactTransferStatus, PROJECT_ARTIFACT_CHUNK_BYTES,
    PROJECT_ARTIFACT_TTL_SECONDS, ProjectArtifactDescriptor, ProjectArtifactManifest,
    artifact_sha256, validate_artifact_digest, validate_artifact_project_id,
};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

struct Transfer {
    descriptor: ProjectArtifactDescriptor,
    manifest_ready: bool,
    state: ArtifactTransferState,
    expires_at: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReservation {
    version: u32,
    principal: String,
    descriptor: ProjectArtifactDescriptor,
    created_at: i64,
    expires_at: i64,
}

fn reservation(path: &Path) -> Result<ArtifactReservation> {
    let bytes = vault::read_private(&path.join("reservation.json"))?;
    ensure!(
        bytes.len() <= 4096,
        "Artifact reservation exceeds its bound"
    );
    let value: ArtifactReservation = serde_json::from_slice(&bytes)
        .context("Untracked artifact reservation cannot be reconciled")?;
    value.descriptor.validate()?;
    principal(&value.principal)?;
    ensure!(
        value.version == 1
            && value.created_at >= 0
            && value.created_at.checked_add(PROJECT_ARTIFACT_TTL_SECONDS) == Some(value.expires_at),
        "Invalid artifact reservation lifetime"
    );
    Ok(value)
}

#[derive(Clone, Copy, Default, Debug)]
struct ArtifactUsage {
    bytes: u64,
    files: u64,
    revisions: u64,
}

impl ArtifactUsage {
    fn add(&mut self, other: Self) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(other.bytes)
            .context("Artifact byte accounting overflow")?;
        self.files = self
            .files
            .checked_add(other.files)
            .context("Artifact file accounting overflow")?;
        self.revisions = self
            .revisions
            .checked_add(other.revisions)
            .context("Artifact revision accounting overflow")?;
        Ok(())
    }

    fn enforce(&self, limit: crate::isolation::ArtifactLimits, scope: &str) -> Result<()> {
        ensure!(
            self.bytes <= limit.bytes
                && self.files <= limit.files
                && self.revisions <= limit.revisions,
            "Artifact storage budget exceeded for {scope}: {} / {} charged bytes, {} / {} file/directory entries, {} / {} revisions. Each in-flight file reserves 34 entries for maximum path depth until commit. An operator must remove unused revisions or increase the limit in agent.env",
            self.bytes,
            limit.bytes,
            self.files,
            limit.files,
            self.revisions,
            limit.revisions
        );
        Ok(())
    }

    fn reservation(descriptor: &ProjectArtifactDescriptor) -> Self {
        // Before receiving the manifest, reserve the protocol's maximum path
        // depth for every file, verification markers, and both manifest copies.
        // Directory entries cost 4 KiB each; this is an admission budget, not a
        // claim about the backing filesystem's allocation or journal overhead.
        let files = u64::from(descriptor.file_count);
        Self {
            bytes: descriptor.total_bytes + descriptor.manifest_size * 2 + (files * 34 + 8) * 4096,
            files: files * 34 + 8,
            revisions: 1,
        }
    }
}

struct ArtifactAccounting {
    device: ArtifactUsage,
    projects: std::collections::BTreeMap<String, ArtifactUsage>,
    scanned: usize,
}

fn entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_expired_orphan_receipt(root: &Path, pending: &ArtifactReservation) -> Result<()> {
    ensure!(
        pending.expires_at <= unix_time()?,
        "Artifact reservation has not expired"
    );
    let mut revisions = root.to_path_buf();
    for component in [
        "projects",
        pending.descriptor.project_id.as_str(),
        "revisions",
    ] {
        revisions.push(component);
        if !entry_exists(&revisions)? {
            return Ok(());
        }
        directory(&revisions)?;
    }
    let digest = &pending.descriptor.manifest_sha256;
    if entry_exists(&revisions.join(digest))? {
        return Ok(());
    }
    let receipt = revisions.join(format!("{digest}.manifest.json"));
    if !entry_exists(&receipt)? {
        return Ok(());
    }
    let file = open(&receipt, false, false)?;
    ensure!(
        file.metadata()?.len() == pending.descriptor.manifest_size,
        "Orphan artifact receipt size differs"
    );
    let mut bytes = Vec::new();
    file.take(pending.descriptor.manifest_size + 1)
        .read_to_end(&mut bytes)?;
    let manifest: ProjectArtifactManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        artifact_sha256(&bytes) == *digest
            && manifest.canonical_bytes()? == bytes
            && manifest.descriptor()? == pending.descriptor,
        "Orphan artifact receipt does not match its expired reservation"
    );
    std::fs::remove_file(receipt)?;
    File::open(revisions)?.sync_all()?;
    Ok(())
}

fn reconcile_expired_reservations(store: &StateStore, root: &Path) -> Result<()> {
    let staging = root.join("artifact-transfers");
    if !entry_exists(&staging)? {
        return Ok(());
    }
    directory(&staging)?;
    let mut accounting = ArtifactAccounting {
        device: Default::default(),
        projects: Default::default(),
        scanned: 0,
    };
    for (index, entry) in std::fs::read_dir(&staging)?.enumerate() {
        ensure!(
            index < 2_000_000,
            "Artifact staging reconciliation exceeds its entry bound"
        );
        let entry = entry?;
        let marker = entry.path().join("reservation.json");
        if !entry_exists(&marker)? {
            continue;
        }
        ensure!(
            accounting.entry(&entry.path())?.0.is_dir(),
            "Artifact staging must be a directory"
        );
        let pending = reservation(&entry.path()).context(
            "Untracked artifact staging data requires operator reconciliation before new uploads",
        )?;
        if pending.expires_at > unix_time()? {
            continue;
        }
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid untracked transfer ID"))?;
        uuid(&id)?;
        let tracked: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_artifact_transfers WHERE transfer_id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if tracked {
            continue;
        }
        accounting.tree(&entry.path(), 0)?;
        remove_expired_orphan_receipt(root, &pending)?;
        std::fs::remove_dir_all(entry.path())?;
        File::open(&staging)?.sync_all()?;
    }
    Ok(())
}

impl ArtifactAccounting {
    fn add(&mut self, project: &str, usage: ArtifactUsage) -> Result<()> {
        ensure!(
            self.projects.contains_key(project) || self.projects.len() < 16_384,
            "Artifact accounting exceeds 16384 project identities; operator reconciliation required"
        );
        self.device.add(usage)?;
        self.projects
            .entry(project.to_owned())
            .or_default()
            .add(usage)
    }

    fn entry(&mut self, path: &Path) -> Result<(std::fs::Metadata, ArtifactUsage)> {
        self.scanned += 1;
        ensure!(
            self.scanned <= 2_000_000,
            "Artifact accounting scan exceeds 2000000 entries; new uploads are disabled until an operator reconciles retained artifacts"
        );
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
            "Artifact accounting refuses symlinks or special files; new uploads require operator reconciliation"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o077 == 0
                    && (!metadata.is_file() || metadata.nlink() == 1),
                "Artifact accounting requires private owned entries without hard links"
            );
        }
        let usage = ArtifactUsage {
            bytes: (if metadata.is_file() {
                metadata.len()
            } else {
                0
            })
            .checked_add(4096)
            .context("Artifact entry is too large")?,
            files: 1,
            revisions: 0,
        };
        Ok((metadata, usage))
    }

    fn tree(&mut self, path: &Path, depth: usize) -> Result<ArtifactUsage> {
        ensure!(
            depth <= 40,
            "Artifact accounting exceeds its directory depth bound"
        );
        let (metadata, mut usage) = self.entry(path)?;
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                usage.add(self.tree(&entry?.path(), depth + 1)?)?;
            }
        }
        Ok(usage)
    }
}

/// Reconstruct usage from durable receipts and directories, rather than expiring
/// transfer rows. The artifact lock serializes admissions with commit/abort.
/// Existing revision reads do not need this scan or a budget increase.
fn admit_artifact(
    store: &StateStore,
    root: &Path,
    descriptor: &ProjectArtifactDescriptor,
    recovered: Option<&str>,
) -> Result<()> {
    let budgets = crate::isolation::artifact_budgets(root)?;
    reconcile_expired_reservations(store, root)?;
    let mut accounting = ArtifactAccounting {
        device: ArtifactUsage::default(),
        projects: Default::default(),
        scanned: 0,
    };
    let projects = directory(&root.join("projects"))?;
    crate::config::ensure_unaliased_child(&projects, &descriptor.project_id)?;
    let mut project_count = 0;
    for project in std::fs::read_dir(&projects)? {
        let project = project?;
        project_count += 1;
        ensure!(
            project_count <= 16_384,
            "Artifact accounting exceeds 16384 project directories"
        );
        let id = project
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid managed project name"))?;
        validate_artifact_project_id(&id)?;
        let (metadata, usage) = accounting.entry(&project.path())?;
        ensure!(metadata.is_dir(), "Managed project is not a directory");
        accounting.add(&id, usage)?;
        let revisions = project.path().join("revisions");
        if !entry_exists(&revisions)? {
            continue;
        }
        let (metadata, usage) = accounting.entry(&revisions)?;
        ensure!(metadata.is_dir(), "Managed revisions must be a directory");
        accounting.add(&id, usage)?;
        let mut revision_count = 0;
        for revision in std::fs::read_dir(&revisions)? {
            let revision = revision?;
            revision_count += 1;
            ensure!(
                revision_count <= 32_768,
                "Artifact accounting exceeds its revision entry bound"
            );
            let name = revision
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid revision name"))?;
            if let Some(digest) = name.strip_suffix(".manifest.json") {
                validate_artifact_digest(digest)?;
                let (metadata, mut usage) = accounting.entry(&revision.path())?;
                ensure!(
                    metadata.is_file()
                        && metadata.len()
                            <= flow_like_device_protocol::PROJECT_ARTIFACT_MANIFEST_BYTES,
                    "Invalid retained artifact receipt"
                );
                let mut bytes = Vec::new();
                open(&revision.path(), false, false)?
                    .take(flow_like_device_protocol::PROJECT_ARTIFACT_MANIFEST_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                let manifest: ProjectArtifactManifest = serde_json::from_slice(&bytes)
                    .context("Retained artifact receipt cannot be reconciled")?;
                ensure!(
                    manifest.project_id == id
                        && manifest.canonical_bytes()? == bytes
                        && artifact_sha256(&bytes) == digest,
                    "Retained artifact receipt cannot be reconciled"
                );
                usage.revisions = 1;
                accounting.add(&id, usage)?;
            } else {
                validate_artifact_digest(&name)
                    .context("Unknown retained revision entry; operator reconciliation required")?;
                ensure!(
                    entry_exists(&revisions.join(format!("{name}.manifest.json")))?,
                    "Retained revision has no receipt; operator reconciliation required before new uploads"
                );
                ensure!(
                    std::fs::symlink_metadata(revision.path())?.is_dir(),
                    "Retained revision must be a directory"
                );
                let usage = accounting.tree(&revision.path(), 0)?;
                accounting.add(&id, usage)?;
            }
        }
    }

    let mut known = std::collections::HashSet::new();
    let mut statement = store.connection.prepare("SELECT transfer_id,project_id,descriptor_json,state FROM project_artifact_transfers LIMIT 4097")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        ensure!(
            known.len() < 4096,
            "Artifact transfer inventory exceeds its bound"
        );
        let id: String = row.get(0)?;
        uuid(&id)?;
        known.insert(id.clone());
        let project: String = row.get(1)?;
        let descriptor: ProjectArtifactDescriptor =
            serde_json::from_str(&row.get::<_, String>(2)?)?;
        descriptor.validate()?;
        ensure!(
            descriptor.project_id == project,
            "Artifact transfer accounting binding differs"
        );
        let state: String = row.get(3)?;
        let path = root.join("artifact-transfers").join(&id);
        let actual = if entry_exists(&path)? {
            accounting.tree(&path, 0)?
        } else {
            ArtifactUsage::default()
        };
        let usage = match state.as_str() {
            "receiving" => {
                let reserved = ArtifactUsage::reservation(&descriptor);
                ArtifactUsage {
                    bytes: actual.bytes.max(reserved.bytes),
                    files: actual.files.max(reserved.files),
                    revisions: 1,
                }
            }
            "committed" | "aborted" => actual,
            _ => anyhow::bail!("Invalid artifact transfer state during accounting"),
        };
        accounting.add(&project, usage)?;
    }
    let mut incoming = ArtifactUsage::reservation(descriptor);
    let staging = root.join("artifact-transfers");
    if entry_exists(&staging)? {
        let (metadata, usage) = accounting.entry(&staging)?;
        ensure!(metadata.is_dir(), "Artifact staging must be a directory");
        accounting.device.add(usage)?;
        for entry in std::fs::read_dir(&staging)? {
            let entry = entry?;
            if known.contains(&entry.file_name().to_string_lossy().into_owned()) {
                continue;
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid untracked transfer ID"))?;
            uuid(&id)?;
            ensure!(
                accounting.entry(&entry.path())?.0.is_dir(),
                "Untracked artifact staging must be a directory"
            );
            let actual = accounting.tree(&entry.path(), 0)?;
            if std::fs::read_dir(entry.path())?.next().is_none() {
                accounting.device.add(actual)?;
                continue;
            }
            // A durable marker survives a crash before the enclosing SQLite
            // transaction commits. Keep its full reservation until expiry;
            // only that staging directory is removed, never project revisions.
            let pending = reservation(&entry.path()).context("Untracked artifact staging data requires operator reconciliation before new uploads")?;
            if pending.expires_at <= unix_time()? {
                remove_expired_orphan_receipt(root, &pending)?;
                std::fs::remove_dir_all(entry.path())?;
                File::open(&staging)?.sync_all()?;
                continue;
            }
            let reserved = ArtifactUsage::reservation(&pending.descriptor);
            let usage = ArtifactUsage {
                bytes: actual.bytes.max(reserved.bytes),
                files: actual.files.max(reserved.files),
                revisions: 1,
            };
            if recovered == Some(id.as_str()) {
                incoming = usage;
            } else {
                accounting.add(&pending.descriptor.project_id, usage)?;
            }
        }
    }
    accounting.add(&descriptor.project_id, incoming)?;
    accounting.device.enforce(budgets.device, "device")?;
    accounting.projects[&descriptor.project_id].enforce(budgets.project, "project")?;
    Ok(())
}

fn uuid(value: &str) -> Result<()> {
    ensure!(
        uuid::Uuid::parse_str(value)?.to_string() == value,
        "Invalid artifact transfer ID"
    );
    Ok(())
}
fn principal(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
        "Invalid artifact principal"
    );
    Ok(())
}
fn directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    File::open(parent)?.sync_all()?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0,
            "Artifact directories must be private, owned directories"
        );
        Ok(path.to_path_buf())
    }
    #[cfg(not(unix))]
    anyhow::bail!("Project artifact storage requires Unix file permissions");
}
fn private_root(root: &Path) -> Result<PathBuf> {
    directory(root)
}
fn transfer_dir(root: &Path, transfer: &str) -> Result<PathBuf> {
    uuid(transfer)?;
    let root = private_root(root)?;
    directory(&directory(&root.join("artifact-transfers"))?.join(transfer))
}
fn open(path: &Path, write: bool, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).context("Open artifact data")?;
    let meta = file.metadata()?;
    ensure!(meta.is_file(), "Artifact data must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.nlink() == 1,
            "Artifact data must be private and have one link"
        );
    }
    Ok(file)
}
fn load(
    store: &StateStore,
    id: &str,
    project: &str,
    owner: &str,
    allow_finished: bool,
) -> Result<Transfer> {
    uuid(id)?;
    principal(owner)?;
    validate_artifact_project_id(project)?;
    let row: Option<(String, String, String, bool, String, i64)> = store.connection.query_row(
        "SELECT project_id,principal,descriptor_json,manifest_ready,state,expires_at FROM project_artifact_transfers WHERE transfer_id=?1", [id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let (stored_project, stored_owner, descriptor, ready, state, expires_at) =
        row.context("Unknown artifact transfer")?;
    ensure!(
        stored_project == project && stored_owner == owner,
        "Artifact transfer authority differs"
    );
    let descriptor: ProjectArtifactDescriptor = serde_json::from_str(&descriptor)?;
    descriptor.validate()?;
    ensure!(
        descriptor.project_id == project,
        "Artifact transfer binding differs"
    );
    let state = match state.as_str() {
        "receiving" => ArtifactTransferState::Receiving,
        "committed" => ArtifactTransferState::Committed,
        "aborted" => ArtifactTransferState::Aborted,
        _ => anyhow::bail!("Invalid artifact transfer state"),
    };
    ensure!(
        allow_finished || state == ArtifactTransferState::Receiving,
        "Artifact transfer is finished"
    );
    ensure!(
        allow_finished || state != ArtifactTransferState::Receiving || expires_at > unix_time()?,
        "Artifact transfer expired"
    );
    Ok(Transfer {
        descriptor,
        manifest_ready: ready,
        state,
        expires_at,
    })
}
struct CachedManifest {
    path: PathBuf,
    descriptor: ProjectArtifactDescriptor,
    stamp: (u64, u64, i64, i64, i64, i64),
    value: Arc<ProjectArtifactManifest>,
}
static MANIFESTS: OnceLock<Mutex<std::collections::VecDeque<CachedManifest>>> = OnceLock::new();
fn manifest(
    dir: &Path,
    descriptor: &ProjectArtifactDescriptor,
) -> Result<Arc<ProjectArtifactManifest>> {
    let path = dir.join("manifest.json");
    let file = open(&path, false, false)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.len() == descriptor.manifest_size,
        "Artifact manifest is incomplete"
    );
    #[cfg(unix)]
    let stamp = {
        use std::os::unix::fs::MetadataExt;
        (
            metadata.dev(),
            metadata.ino(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    };
    #[cfg(not(unix))]
    let stamp = (0, 0, 0, 0, 0, 0);
    let cache = MANIFESTS.get_or_init(Default::default);
    if let Some(value) = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("Artifact manifest cache unavailable"))?
        .iter()
        .find(|entry| entry.path == path && entry.descriptor == *descriptor && entry.stamp == stamp)
        .map(|entry| entry.value.clone())
    {
        return Ok(value);
    }
    let mut bytes = Vec::new();
    file.take(descriptor.manifest_size + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        artifact_sha256(&bytes) == descriptor.manifest_sha256,
        "Artifact manifest digest differs"
    );
    let value: ProjectArtifactManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        value.canonical_bytes()? == bytes && value.descriptor()? == *descriptor,
        "Artifact manifest binding differs"
    );
    let value = Arc::new(value);
    let mut cache = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("Artifact manifest cache unavailable"))?;
    cache.retain(|entry| entry.path != path);
    if cache.len() >= 8 {
        cache.pop_front();
    }
    cache.push_back(CachedManifest {
        path,
        descriptor: descriptor.clone(),
        stamp,
        value: value.clone(),
    });
    Ok(value)
}

fn data_path(dir: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = directory(&dir.join("data"))?;
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        if components.peek().is_some() {
            directory(&path)?;
        }
    }
    Ok(path)
}
fn file_info(
    dir: &Path,
    transfer: &Transfer,
    index: Option<u32>,
) -> Result<(PathBuf, u64, String, Option<PathBuf>)> {
    match index {
        None => Ok((
            dir.join("manifest.json"),
            transfer.descriptor.manifest_size,
            transfer.descriptor.manifest_sha256.clone(),
            None,
        )),
        Some(index) => {
            ensure!(
                transfer.manifest_ready,
                "Upload the complete artifact manifest first"
            );
            let value = manifest(dir, &transfer.descriptor)?;
            let item = value
                .files
                .get(index as usize)
                .context("Invalid artifact file index")?;
            Ok((
                data_path(dir, &item.path)?,
                item.size,
                item.sha256.clone(),
                Some(directory(&dir.join("verified"))?.join(index.to_string())),
            ))
        }
    }
}
fn checked_marker(path: &Path, expected: &str) -> Result<bool> {
    if !path.try_exists()? {
        return Ok(false);
    }
    ensure!(
        vault::read_private(path)?.as_slice() == expected.as_bytes(),
        "Artifact verification marker differs"
    );
    Ok(true)
}
fn status_inner(
    root: &Path,
    id: &str,
    transfer: Transfer,
    index: Option<u32>,
) -> Result<ArtifactTransferStatus> {
    let mut offset = 0;
    let mut complete = false;
    let mut path = None;
    if transfer.state == ArtifactTransferState::Committed {
        path = Some(
            managed_revision_for_source(
                root,
                &transfer.descriptor.project_id,
                &transfer.descriptor.manifest_sha256,
                transfer.descriptor.source,
            )?
            .to_string_lossy()
            .into_owned(),
        );
        complete = true;
        offset = if let Some(index) = index {
            let m = manifest(&transfer_dir(root, id)?, &transfer.descriptor)?;
            m.files
                .get(index as usize)
                .context("Invalid artifact file index")?
                .size
        } else {
            transfer.descriptor.manifest_size
        };
    } else if transfer.state == ArtifactTransferState::Receiving {
        let dir = transfer_dir(root, id)?;
        let (file, size, digest, marker) = file_info(&dir, &transfer, index)?;
        if file.try_exists()? {
            offset = open(&file, false, false)?.metadata()?.len();
            ensure!(offset <= size, "Artifact file exceeds its declared size");
        }
        complete = match marker {
            Some(marker) => checked_marker(&marker, &digest)?,
            None => transfer.manifest_ready,
        };
        ensure!(!complete || offset == size, "Verified artifact was changed");
    }
    Ok(ArtifactTransferStatus {
        transfer_id: id.into(),
        descriptor: transfer.descriptor,
        state: transfer.state,
        expires_at: transfer.expires_at,
        manifest_ready: transfer.manifest_ready,
        file_index: index,
        offset,
        complete,
        project_path: path,
    })
}

/// The management dispatcher authorizes the project and journals begin/commit in its transaction.
/// Run file I/O on a blocking task; a final chunk hashes the complete file before acknowledging it.
pub fn begin(
    store: &StateStore,
    root: &Path,
    transfer_id: &str,
    owner: &str,
    descriptor: &ProjectArtifactDescriptor,
) -> Result<ArtifactTransferStatus> {
    descriptor.validate()?;
    uuid(transfer_id)?;
    principal(owner)?;
    private_root(root)?;
    let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
    let exists: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM project_artifact_transfers WHERE transfer_id=?1)",
        [transfer_id],
        |r| r.get(0),
    )?;
    if exists {
        let value = load(store, transfer_id, &descriptor.project_id, owner, true)?;
        ensure!(
            value.descriptor == *descriptor,
            "Artifact transfer descriptor differs"
        );
        return status_inner(root, transfer_id, value, None);
    }
    let now = unix_time()?;
    let reservation_path = root
        .join("artifact-transfers")
        .join(transfer_id)
        .join("reservation.json");
    let previous = if entry_exists(&reservation_path)? {
        let pending = reservation(reservation_path.parent().unwrap())?;
        ensure!(
            pending.descriptor == *descriptor && pending.principal == owner,
            "Artifact reservation authority or descriptor differs"
        );
        (pending.expires_at > now).then_some(pending)
    } else {
        None
    };
    let expired: Vec<String> = {
        let mut statement = store.connection.prepare(
            "SELECT transfer_id FROM project_artifact_transfers WHERE expires_at<=?1 LIMIT 4096",
        )?;
        statement
            .query_map([now], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?
    };
    for id in expired {
        uuid(&id)?;
        let dir = transfer_dir(root, &id)?;
        if entry_exists(&dir.join("reservation.json"))? {
            let pending = reservation(&dir)?;
            if pending.expires_at <= now {
                remove_expired_orphan_receipt(root, &pending)?;
            }
        }
        std::fs::remove_dir_all(dir)?;
        File::open(root.join("artifact-transfers"))?.sync_all()?;
        store.connection.execute(
            "DELETE FROM project_artifact_transfers WHERE transfer_id=?1",
            [id],
        )?;
    }
    let (count,bytes,rows):(u64,u64,u64)=store.connection.query_row("SELECT COALESCE(SUM(CASE WHEN state='receiving' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='receiving' THEN json_extract(descriptor_json,'$.total_bytes')+json_extract(descriptor_json,'$.manifest_size') ELSE 0 END),0),COUNT(*) FROM project_artifact_transfers",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    ensure!(
        count < 4
            && bytes
                .checked_add(descriptor.total_bytes + descriptor.manifest_size)
                .is_some_and(|v| v <= 16 * 1024 * 1024 * 1024)
            && rows < 4096,
        "Artifact staging quota exceeded"
    );
    admit_artifact(
        store,
        root,
        descriptor,
        previous.as_ref().map(|_| transfer_id),
    )?;
    managed_project_root(root, &descriptor.project_id)?;
    let staging = transfer_dir(root, transfer_id)?;
    // The outer management transaction may commit after this lock is released.
    // Its fsynced reservation remains visible to every process in that window.
    let pending = if let Some(pending) = previous {
        pending
    } else {
        let pending = ArtifactReservation {
            version: 1,
            principal: owner.into(),
            descriptor: descriptor.clone(),
            created_at: now,
            expires_at: now + PROJECT_ARTIFACT_TTL_SECONDS,
        };
        vault::write_new_private(
            &staging.join("reservation.json"),
            &serde_json::to_vec(&pending)?,
        )?;
        pending
    };
    store.connection.execute("INSERT INTO project_artifact_transfers(transfer_id,project_id,principal,descriptor_json,manifest_ready,state,created_at,expires_at) VALUES(?1,?2,?3,?4,0,'receiving',?5,?6)",params![transfer_id,descriptor.project_id,owner,serde_json::to_string(descriptor)?,pending.created_at,pending.expires_at])?;
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, &descriptor.project_id, owner, false)?,
        None,
    )
}
pub fn status(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
    index: Option<u32>,
) -> Result<ArtifactTransferStatus> {
    let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, project, owner, true)?,
        index,
    )
}
pub fn chunk(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
    index: Option<u32>,
    offset: u64,
    data: &str,
) -> Result<ArtifactTransferStatus> {
    ensure!(
        data.len() <= PROJECT_ARTIFACT_CHUNK_BYTES.div_ceil(3) * 4,
        "Artifact chunk exceeds its bound"
    );
    let bytes = URL_SAFE_NO_PAD
        .decode(data)
        .context("Invalid artifact chunk encoding")?;
    ensure!(
        bytes.len() <= PROJECT_ARTIFACT_CHUNK_BYTES && URL_SAFE_NO_PAD.encode(&bytes) == data,
        "Invalid artifact chunk encoding"
    );
    let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
    let value = load(store, transfer_id, project, owner, false)?;
    let dir = transfer_dir(root, transfer_id)?;
    let (path, size, digest, marker) = file_info(&dir, &value, index)?;
    let end = offset
        .checked_add(bytes.len() as u64)
        .context("Artifact chunk offset overflow")?;
    ensure!(
        end <= size && (!bytes.is_empty() || size == 0),
        "Artifact chunk exceeds declared size"
    );
    let mut file = open(&path, true, true)?;
    let length = file.metadata()?.len();
    ensure!(
        length <= size && offset <= length,
        "Artifact chunk offset is not contiguous"
    );
    if offset < length {
        ensure!(end <= length, "Artifact retry overlaps unwritten bytes");
        let mut existing = vec![0; bytes.len()];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut existing)?;
        ensure!(
            existing == bytes,
            "Artifact retry differs from received bytes"
        );
    } else {
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    if file.metadata()?.len() == size {
        file.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut received = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received += count as u64;
            ensure!(received <= size, "Artifact changed during verification");
            hash.update(&buffer[..count]);
        }
        if received != size || format!("{:x}", hash.finalize()) != digest {
            file.set_len(0)?;
            file.sync_all()?;
            if let Some(marker) = &marker {
                if marker.try_exists()? {
                    std::fs::remove_file(marker)?;
                }
            } else {
                store.connection.execute(
                    "UPDATE project_artifact_transfers SET manifest_ready=0 WHERE transfer_id=?1",
                    [transfer_id],
                )?;
            }
            anyhow::bail!("Artifact SHA256 differs; restart this file from offset zero");
        }
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        if let Some(marker) = marker {
            if !checked_marker(&marker, &digest)? {
                vault::write_new_private(&marker, digest.as_bytes())?;
            }
        } else {
            manifest(&dir, &value.descriptor)?;
            store.connection.execute(
                "UPDATE project_artifact_transfers SET manifest_ready=1 WHERE transfer_id=?1",
                [transfer_id],
            )?;
        }
    }
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, project, owner, false)?,
        index,
    )
}
pub fn managed_revision(root: &Path, project: &str, digest: &str) -> Result<PathBuf> {
    managed_revision_for_source(
        root,
        project,
        digest,
        flow_like_device_protocol::ProjectArtifactSource::Offline,
    )
}

pub fn managed_revision_for_source(
    root: &Path,
    project: &str,
    digest: &str,
    source: flow_like_device_protocol::ProjectArtifactSource,
) -> Result<PathBuf> {
    validate_artifact_project_id(project)?;
    validate_artifact_digest(digest)?;
    let base = directory(&managed_project_root(root, project)?.join("revisions"))?;
    let path = base.join(digest);
    let meta =
        std::fs::symlink_metadata(&path).context("Project revision has not been imported")?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "Managed project revision is invalid"
    );
    let receipt = base.join(format!("{digest}.manifest.json"));
    let file = open(&receipt, false, false)?;
    ensure!(
        file.metadata()?.len() <= flow_like_device_protocol::PROJECT_ARTIFACT_MANIFEST_BYTES,
        "Imported manifest exceeds its bound"
    );
    let mut bytes = Vec::new();
    file.take(flow_like_device_protocol::PROJECT_ARTIFACT_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let manifest: ProjectArtifactManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.project_id == project
            && manifest.source == source
            && manifest.canonical_bytes()? == bytes
            && artifact_sha256(&bytes) == digest,
        "Imported revision receipt differs"
    );
    Ok(path)
}
pub fn managed_project_root(root: &Path, project: &str) -> Result<PathBuf> {
    validate_artifact_project_id(project)?;
    let projects = directory(&private_root(root)?.join("projects"))?;
    crate::config::ensure_unaliased_child(&projects, project)?;
    let path = directory(&projects.join(project))?;
    crate::config::ensure_unaliased_child(&projects, project)?;
    Ok(path)
}
pub fn prepare_online_cache(root: &Path, project: &str) -> Result<PathBuf> {
    directory(&managed_project_root(root, project)?.join("online-cache"))
}
pub fn commit(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
) -> Result<ArtifactTransferStatus> {
    let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
    let value = load(store, transfer_id, project, owner, true)?;
    if value.state == ArtifactTransferState::Committed {
        return status_inner(root, transfer_id, value, None);
    }
    ensure!(
        value.state == ArtifactTransferState::Receiving
            && value.manifest_ready
            && value.expires_at > unix_time()?,
        "Artifact manifest is not ready"
    );
    let dir = transfer_dir(root, transfer_id)?;
    let manifest = manifest(&dir, &value.descriptor)?;
    let has_staged_data = dir.join("data").try_exists()?;
    for (index, item) in manifest.files.iter().enumerate() {
        if has_staged_data {
            ensure!(
                open(&data_path(&dir, &item.path)?, false, false)?
                    .metadata()?
                    .len()
                    == item.size,
                "Verified artifact file is missing or changed"
            );
        }
        ensure!(
            checked_marker(&dir.join("verified").join(index.to_string()), &item.sha256)?,
            "Artifact file is not verified"
        );
    }
    let revisions = directory(&managed_project_root(root, project)?.join("revisions"))?;
    let final_path = revisions.join(&value.descriptor.manifest_sha256);
    let receipt = revisions.join(format!(
        "{}.manifest.json",
        value.descriptor.manifest_sha256
    ));
    let verification_root = if has_staged_data {
        dir.join("data")
    } else {
        final_path.clone()
    };
    validate_selected_assets(&verification_root, &manifest)?;
    let canonical = manifest.canonical_bytes()?;
    ensure!(
        !final_path.try_exists()? || receipt.try_exists()?,
        "Existing revision has no matching import receipt"
    );
    if receipt.try_exists()? {
        let mut bytes = Vec::new();
        open(&receipt, false, false)?
            .take(canonical.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes == canonical, "Imported revision manifest differs");
    } else {
        vault::write_new_private(&receipt, &canonical)?;
    }
    if final_path.try_exists()? {
        managed_revision_for_source(
            root,
            project,
            &value.descriptor.manifest_sha256,
            value.descriptor.source,
        )?;
        if dir.join("data").try_exists()? {
            std::fs::remove_dir_all(dir.join("data"))?;
        }
    } else {
        std::fs::rename(dir.join("data"), &final_path)?;
        File::open(&revisions)?.sync_all()?;
    }
    store.connection.execute(
        "UPDATE project_artifact_transfers SET state='committed' WHERE transfer_id=?1",
        [transfer_id],
    )?;
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, project, owner, true)?,
        None,
    )
}
pub fn abort(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
) -> Result<ArtifactTransferStatus> {
    let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
    let value = load(store, transfer_id, project, owner, true)?;
    ensure!(
        value.state != ArtifactTransferState::Committed,
        "Committed revisions cannot be aborted"
    );
    std::fs::remove_dir_all(transfer_dir(root, transfer_id)?)?;
    store.connection.execute(
        "UPDATE project_artifact_transfers SET state='aborted' WHERE transfer_id=?1",
        [transfer_id],
    )?;
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, project, owner, true)?,
        None,
    )
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectArtifactAssets {
    #[serde(default)]
    pub bit_pins: Vec<flow_like_device_protocol::ProjectBitPin>,
    #[serde(default)]
    pub package_pins: Vec<flow_like_device_protocol::ProjectPackagePin>,
}
fn selected_source(root: &Path, relative: &str) -> Result<PathBuf> {
    flow_like_device_protocol::validate_artifact_relative_path(relative)?;
    let mut path = root.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path)?;
        ensure!(
            !meta.file_type().is_symlink()
                && if parts.peek().is_some() {
                    meta.is_dir()
                } else {
                    meta.is_file()
                },
            "Selected asset must be a regular file below its source"
        );
    }
    Ok(path)
}
fn metadata_assets(
    bytes: &[u8],
    pin: &flow_like_device_protocol::ProjectBitPin,
) -> Result<std::collections::BTreeMap<String, (u64, String)>> {
    ensure!(
        bytes.len() <= 16 * 1024 * 1024 && artifact_sha256(bytes) == pin.metadata_sha256,
        "Selected Bit metadata digest differs"
    );
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let wrapper = value.as_object().context("Invalid Bit metadata")?;
    ensure!(
        wrapper.len() == 3
            && wrapper.contains_key("bit")
            && wrapper.contains_key("dependencies")
            && wrapper.contains_key("artifacts"),
        "Invalid Bit metadata fields"
    );
    let bit = value.get("bit").context("Missing Bit metadata")?;
    ensure!(
        bit.get("id").and_then(serde_json::Value::as_str) == Some(pin.bit_id.as_str()),
        "Selected Bit identity differs"
    );
    let dependencies = value
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
        .context("Missing Bit dependencies")?;
    ensure!(
        dependencies.len() <= 2048,
        "Too many selected Bit dependencies"
    );
    let mut paths = std::collections::BTreeMap::new();
    for bit in std::iter::once(bit).chain(dependencies.iter()) {
        if let Some(name) = bit.get("file_name").filter(|v| !v.is_null()) {
            let name = name.as_str().context("Invalid Bit asset name")?;
            let hash = bit
                .get("hash")
                .and_then(serde_json::Value::as_str)
                .context("Missing Bit asset hash")?;
            ensure!(
                !hash.contains('/') && hash != "metadata" && hash != "deps-cache",
                "Invalid selected Bit asset path"
            );
            let path = format!("bits/{hash}/{name}");
            flow_like_device_protocol::validate_artifact_relative_path(&path)?;
            let size = bit
                .get("size")
                .filter(|v| !v.is_null())
                .map(|v| v.as_u64().context("Invalid Bit asset size"))
                .transpose()?;
            if let Some(previous) = paths.insert(path, size) {
                ensure!(previous == size, "Conflicting selected Bit assets");
            }
        }
    }
    let artifacts: Vec<flow_like_device_protocol::ProjectArtifactFile> = serde_json::from_value(
        value
            .get("artifacts")
            .context("Missing Bit asset digests")?
            .clone(),
    )?;
    ensure!(
        artifacts.len() <= 2048 && artifacts.len() == paths.len(),
        "Bit assets must match metadata exactly"
    );
    let mut selected = std::collections::BTreeMap::new();
    for file in artifacts {
        let size = paths
            .get(&file.path)
            .context("Unselected Bit asset digest")?;
        validate_artifact_digest(&file.sha256)?;
        ensure!(
            size.is_none_or(|size| size == file.size)
                && file.size <= flow_like_device_protocol::PROJECT_ARTIFACT_MAX_FILE_BYTES
                && selected
                    .insert(file.path, (file.size, file.sha256))
                    .is_none(),
            "Selected Bit asset size or identity differs"
        );
    }
    Ok(selected)
}
fn read_selected_metadata(
    root: &Path,
    pin: &flow_like_device_protocol::ProjectBitPin,
) -> Result<Vec<u8>> {
    let path = selected_source(root, &format!("bits/metadata/{}.json", pin.bit_id))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 16 * 1024 * 1024,
        "Selected Bit metadata exceeds its bound"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn validate_selected_assets(root: &Path, manifest: &ProjectArtifactManifest) -> Result<()> {
    if manifest.source == flow_like_device_protocol::ProjectArtifactSource::Online {
        let path = selected_source(
            root,
            &format!("apps/{}/online-source.json", manifest.project_id),
        )?;
        let file = open(&path, false, false)?;
        ensure!(
            file.metadata()?.len() <= 512,
            "Online source marker exceeds its bound"
        );
        let mut bytes = Vec::new();
        file.take(513).read_to_end(&mut bytes)?;
        let marker: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(
            marker
                == serde_json::json!({"version":1,"project_id":manifest.project_id,"source":"online"}),
            "Online source marker differs from its artifact"
        );
    }
    let mut selected = std::collections::BTreeMap::new();
    for pin in &manifest.bit_pins {
        let bytes = read_selected_metadata(root, pin)?;
        for (path, size) in metadata_assets(&bytes, pin)? {
            if let Some(previous) = selected.insert(path, size.clone()) {
                ensure!(previous == size, "Conflicting selected Bit asset sizes");
            }
        }
    }
    for (path, (size, digest)) in &selected {
        ensure!(
            manifest
                .files
                .iter()
                .any(|f| &f.path == path && f.size == *size && &f.sha256 == digest),
            "Selected Bit asset is missing or its size differs"
        );
    }
    for file in &manifest.files {
        if file.path.starts_with("bits/") && !file.path.starts_with("bits/metadata/") {
            ensure!(
                selected.contains_key(&file.path),
                "Artifact contains an unselected Bit asset"
            );
        }
    }
    Ok(())
}

/// Import an existing object-store root. Other projects and global user data are never copied.
pub fn import_local(
    store: &StateStore,
    root: &Path,
    project: &str,
    source: &Path,
) -> Result<ArtifactTransferStatus> {
    import_local_selected(
        store,
        root,
        project,
        source,
        &ProjectArtifactAssets::default(),
    )
}
pub fn import_local_selected(
    store: &StateStore,
    root: &Path,
    project: &str,
    source: &Path,
    assets: &ProjectArtifactAssets,
) -> Result<ArtifactTransferStatus> {
    use flow_like_device_protocol::{
        PROJECT_ARTIFACT_MAX_FILE_BYTES, PROJECT_ARTIFACT_MAX_FILES, ProjectArtifactFile,
    };
    validate_artifact_project_id(project)?;
    let source_meta = std::fs::symlink_metadata(source)?;
    ensure!(
        source_meta.is_dir() && !source_meta.file_type().is_symlink(),
        "Import source must be a directory without symlinks"
    );
    let source = source.canonicalize()?;
    let mut base = source.clone();
    for part in ["apps", project] {
        base.push(part);
        let m = std::fs::symlink_metadata(&base)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "Project source directory is invalid"
        );
    }
    fn walk(
        base: &Path,
        root: &Path,
        files: &mut Vec<(PathBuf, String)>,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth <= 32, "Project source exceeds path depth");
        for entry in std::fs::read_dir(base)? {
            let entry = entry?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            ensure!(
                !meta.file_type().is_symlink(),
                "Project import refuses symbolic links"
            );
            if meta.is_dir() {
                walk(&path, root, files, depth + 1)?;
            } else {
                ensure!(
                    meta.is_file() && files.len() < PROJECT_ARTIFACT_MAX_FILES,
                    "Project import exceeds file bounds"
                );
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .context("Project file names must be UTF-8")?
                    .to_owned();
                files.push((path, relative));
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    walk(&base, &source, &mut paths, 0)?;
    for (_, relative) in &mut paths {
        *relative = flow_like_device_protocol::normalize_artifact_path(project, relative)?;
    }
    ensure!(
        assets.bit_pins.len() <= 256 && assets.package_pins.len() <= 256,
        "Too many selected project assets"
    );
    let mut selected = std::collections::BTreeSet::new();
    for pin in &assets.bit_pins {
        pin.validate()?;
        let metadata = read_selected_metadata(&source, pin)?;
        selected.insert(format!("bits/metadata/{}.json", pin.bit_id));
        selected.extend(metadata_assets(&metadata, pin)?.into_keys());
    }
    for pin in &assets.package_pins {
        pin.validate()?;
        for name in ["manifest.json", "module.wasm"] {
            selected.insert(format!(
                "packages/{}/{}/{name}",
                pin.package_id, pin.version
            ));
        }
    }
    for relative in selected {
        paths.push((selected_source(&source, &relative)?, relative));
    }
    paths.sort_by(|a, b| a.1.cmp(&b.1));
    let mut files = Vec::new();
    for (path, relative) in &paths {
        flow_like_device_protocol::validate_artifact_relative_path(relative)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut input = options.open(path)?;
        let meta = input.metadata()?;
        ensure!(
            meta.is_file() && meta.len() <= PROJECT_ARTIFACT_MAX_FILE_BYTES,
            "Project file exceeds its bound"
        );
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 64 * 1024];
        let mut count = 0;
        loop {
            let size = input.read(&mut bytes)?;
            if size == 0 {
                break;
            }
            count += size as u64;
            ensure!(count <= meta.len(), "Project changed during import");
            hash.update(&bytes[..size]);
        }
        ensure!(count == meta.len(), "Project changed during import");
        files.push(ProjectArtifactFile {
            path: relative.clone(),
            size: count,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    let manifest = ProjectArtifactManifest {
        version: 1,
        source: flow_like_device_protocol::ProjectArtifactSource::Offline,
        project_id: project.into(),
        bit_pins: assets.bit_pins.clone(),
        package_pins: assets.package_pins.clone(),
        files,
    };
    let descriptor = manifest.descriptor()?;
    validate_selected_assets(&source, &manifest)?;
    let id = uuid::Uuid::new_v4().to_string();
    let owner = "local-cli";
    begin(store, root, &id, owner, &descriptor)?;
    for (index, bytes) in manifest
        .canonical_bytes()?
        .chunks(PROJECT_ARTIFACT_CHUNK_BYTES)
        .enumerate()
    {
        chunk(
            store,
            root,
            project,
            &id,
            owner,
            None,
            (index * PROJECT_ARTIFACT_CHUNK_BYTES) as u64,
            &URL_SAFE_NO_PAD.encode(bytes),
        )?;
    }
    {
        let _lock = supervisor::lock_file(&root.join("artifact.lock"))?;
        let transfer = load(store, &id, project, owner, false)?;
        let dir = transfer_dir(root, &id)?;
        for (index, (source, _)) in paths.iter().enumerate() {
            let (destination, size, digest, marker) =
                file_info(&dir, &transfer, Some(index as u32))?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let mut input = options.open(source)?;
            ensure!(
                input.metadata()?.is_file(),
                "Project source changed during import"
            );
            let mut output = open(&destination, true, true)?;
            ensure!(
                output.metadata()?.len() == 0,
                "Import destination already exists"
            );
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 64 * 1024];
            let mut count = 0u64;
            loop {
                let length = input.read(&mut bytes)?;
                if length == 0 {
                    break;
                }
                count += length as u64;
                ensure!(count <= size, "Project changed during import");
                output.write_all(&bytes[..length])?;
                hash.update(&bytes[..length]);
            }
            ensure!(
                count == size && format!("{:x}", hash.finalize()) == digest,
                "Project changed during import"
            );
            output.sync_all()?;
            if let Some(parent) = destination.parent() {
                File::open(parent)?.sync_all()?;
            }
            vault::write_new_private(
                &marker.context("Missing artifact verification path")?,
                digest.as_bytes(),
            )?;
        }
    }
    commit(store, root, project, &id, owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::ProjectArtifactFile;
    fn setup() -> (tempfile::TempDir, StateStore, ProjectArtifactManifest) {
        let dir = tempfile::tempdir().unwrap();
        supervisor::prepare_state_dir(dir.path()).unwrap();
        let store = StateStore::open(&dir.path().join("management.sqlite")).unwrap();
        let manifest = ProjectArtifactManifest {
            version: 1,
            source: flow_like_device_protocol::ProjectArtifactSource::Offline,
            project_id: "project".into(),
            bit_pins: vec![],
            package_pins: vec![],
            files: vec![ProjectArtifactFile {
                path: "apps/project/manifest.app".into(),
                size: 5,
                sha256: artifact_sha256(b"hello"),
            }],
        };
        (dir, store, manifest)
    }
    fn start(store: &StateStore, root: &Path, m: &ProjectArtifactManifest) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        begin(store, root, &id, "controller", &m.descriptor().unwrap()).unwrap();
        chunk(
            store,
            root,
            &m.project_id,
            &id,
            "controller",
            None,
            0,
            &URL_SAFE_NO_PAD.encode(m.canonical_bytes().unwrap()),
        )
        .unwrap();
        id
    }

    fn upload(
        store: &StateStore,
        root: &Path,
        m: &ProjectArtifactManifest,
        bytes: &[u8],
    ) -> ArtifactTransferStatus {
        let id = start(store, root, m);
        chunk(
            store,
            root,
            &m.project_id,
            &id,
            "controller",
            Some(0),
            0,
            &URL_SAFE_NO_PAD.encode(bytes),
        )
        .unwrap();
        commit(store, root, &m.project_id, &id, "controller").unwrap()
    }

    fn configure_budget(root: &Path, values: &str) {
        let path = root.join("agent.env");
        if path.exists() {
            std::fs::write(path, values).unwrap();
        } else {
            vault::write_new_private(&path, values.as_bytes()).unwrap();
        }
    }

    fn variant(
        m: &ProjectArtifactManifest,
        project: &str,
        bytes: &[u8],
    ) -> ProjectArtifactManifest {
        let mut m = m.clone();
        m.project_id = project.into();
        m.files[0].path = format!("apps/{project}/manifest.app");
        m.files[0].size = bytes.len() as u64;
        m.files[0].sha256 = artifact_sha256(bytes);
        m
    }

    #[test]
    fn committed_revision_budget_survives_expiry_restart_and_missing_transfer_rows() {
        let (dir, store, m) = setup();
        configure_budget(
            dir.path(),
            "FLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=1\nFLOW_LIKE_DEVICE_ARTIFACT_REVISIONS=2\n",
        );
        let committed = upload(&store, dir.path(), &m, b"hello");
        store
            .connection
            .execute("UPDATE project_artifact_transfers SET expires_at=0", [])
            .unwrap();
        drop(store);
        let store = StateStore::open(&dir.path().join("management.sqlite")).unwrap();
        let next = variant(&m, "project", b"next!");
        let error = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "another-shared-deployer",
            &next.descriptor().unwrap(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("budget exceeded for project"),
            "{error:#}"
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM project_artifact_transfers",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
            0
        );
        assert!(
            managed_revision(dir.path(), "project", &committed.descriptor.manifest_sha256).is_ok()
        );

        // A different project has its own allowance, but cannot bypass the
        // device's aggregate limit by selecting another project or principal.
        let other = variant(&m, "other", b"hello");
        upload(&store, dir.path(), &other, b"hello");
        let third = variant(&m, "third", b"hello");
        let error = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "third-deployer",
            &third.descriptor().unwrap(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("budget exceeded for device"),
            "{error:#}"
        );
    }

    #[test]
    fn file_and_byte_budgets_include_receiving_reservations_and_metadata() {
        let (dir, store, m) = setup();
        let descriptor = m.descriptor().unwrap();
        let charge = ArtifactUsage::reservation(&descriptor);
        configure_budget(
            dir.path(),
            &format!("FLOW_LIKE_DEVICE_ARTIFACT_BYTES={}\n", charge.bytes - 1),
        );
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &descriptor
            )
            .unwrap_err()
            .to_string()
            .contains("charged bytes")
        );
        configure_budget(
            dir.path(),
            &format!("FLOW_LIKE_PROJECT_ARTIFACT_FILES={}\n", charge.files - 1),
        );
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &descriptor
            )
            .unwrap_err()
            .to_string()
            .contains("budget exceeded for project")
        );
        configure_budget(
            dir.path(),
            &format!("FLOW_LIKE_DEVICE_ARTIFACT_FILES={}\n", charge.files + 8),
        );
        let id = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "controller",
            &descriptor,
        )
        .unwrap()
        .transfer_id;
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "other",
                &descriptor
            )
            .is_err()
        );
        abort(&store, dir.path(), "project", &id, "controller").unwrap();
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "other",
                &descriptor
            )
            .is_ok()
        );
    }

    #[test]
    fn duplicate_revision_does_not_leave_a_second_revision_charge() {
        let (dir, store, m) = setup();
        let first = upload(&store, dir.path(), &m, b"hello");
        upload(&store, dir.path(), &m, b"hello");
        store
            .connection
            .execute("DELETE FROM project_artifact_transfers", [])
            .unwrap();
        // Simulate legacy transfer expiry, including its staging cleanup.
        std::fs::remove_dir_all(dir.path().join("artifact-transfers")).unwrap();
        configure_budget(dir.path(), "FLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=2\n");
        let next = variant(&m, "project", b"next!");
        upload(&store, dir.path(), &next, b"next!");
        assert!(managed_revision(dir.path(), "project", &first.descriptor.manifest_sha256).is_ok());
    }

    #[test]
    fn interrupted_begin_reservation_is_charged_recoverable_and_expires() {
        let (dir, store, m) = setup();
        configure_budget(dir.path(), "FLOW_LIKE_DEVICE_ARTIFACT_REVISIONS=1\n");
        let id = uuid::Uuid::new_v4().to_string();
        store.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        begin(
            &store,
            dir.path(),
            &id,
            "controller",
            &m.descriptor().unwrap(),
        )
        .unwrap();
        store.connection.execute_batch("ROLLBACK").unwrap();
        // A new connection sees the reservation even though no SQL row survived.
        drop(store);
        let store = StateStore::open(&dir.path().join("management.sqlite")).unwrap();
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &m.descriptor().unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("budget exceeded for device")
        );
        assert!(
            begin(
                &store,
                dir.path(),
                &id,
                "intruder",
                &m.descriptor().unwrap()
            )
            .is_err()
        );
        begin(
            &store,
            dir.path(),
            &id,
            "controller",
            &m.descriptor().unwrap(),
        )
        .unwrap();
        abort(&store, dir.path(), "project", &id, "controller").unwrap();

        let abandoned_id = uuid::Uuid::new_v4().to_string();
        let abandoned = transfer_dir(dir.path(), &abandoned_id).unwrap();
        let expired = ArtifactReservation {
            version: 1,
            principal: "controller".into(),
            descriptor: m.descriptor().unwrap(),
            created_at: 0,
            expires_at: PROJECT_ARTIFACT_TTL_SECONDS,
        };
        vault::write_new_private(
            &abandoned.join("reservation.json"),
            &serde_json::to_vec(&expired).unwrap(),
        )
        .unwrap();
        vault::write_new_private(&abandoned.join("partial"), b"old partial data").unwrap();
        begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "controller",
            &m.descriptor().unwrap(),
        )
        .unwrap();
        assert!(!abandoned.exists());
    }

    #[test]
    fn tiny_deep_files_reserve_directory_entries_before_any_file_is_written() {
        let (dir, store, mut m) = setup();
        let path = format!(
            "apps/project/{}/empty",
            (0..29)
                .map(|index| format!("d{index}"))
                .collect::<Vec<_>>()
                .join("/")
        );
        m.files.push(ProjectArtifactFile {
            path,
            size: 0,
            sha256: artifact_sha256(b""),
        });
        m.files.sort_by(|left, right| left.path.cmp(&right.path));
        let descriptor = m.descriptor().unwrap();
        configure_budget(dir.path(), "FLOW_LIKE_PROJECT_ARTIFACT_FILES=32\n");
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &descriptor
            )
            .unwrap_err()
            .to_string()
            .contains("file/directory entries")
        );
        assert!(!dir.path().join("artifact-transfers").exists());
        configure_budget(dir.path(), "FLOW_LIKE_PROJECT_ARTIFACT_FILES=128\n");
        let id = start(&store, dir.path(), &m);
        for (index, file) in m.files.iter().enumerate() {
            let data: &[u8] = if file.size == 0 { b"" } else { b"hello" };
            chunk(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(index as u32),
                0,
                &URL_SAFE_NO_PAD.encode(data),
            )
            .unwrap();
        }
        let uploaded = commit(&store, dir.path(), "project", &id, "controller").unwrap();
        let mut accounting = ArtifactAccounting {
            device: ArtifactUsage::default(),
            projects: Default::default(),
            scanned: 0,
        };
        let actual = accounting
            .tree(Path::new(uploaded.project_path.as_deref().unwrap()), 0)
            .unwrap();
        let reserved = ArtifactUsage::reservation(&descriptor);
        assert!(actual.files <= reserved.files && actual.bytes <= reserved.bytes);
    }

    #[test]
    fn receipt_before_rename_crash_expires_without_deleting_complete_revisions() {
        let (dir, store, m) = setup();
        configure_budget(dir.path(), "FLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=1\n");
        let id = uuid::Uuid::new_v4().to_string();
        let staging = transfer_dir(dir.path(), &id).unwrap();
        let pending = ArtifactReservation {
            version: 1,
            principal: "controller".into(),
            descriptor: m.descriptor().unwrap(),
            created_at: 0,
            expires_at: PROJECT_ARTIFACT_TTL_SECONDS,
        };
        vault::write_new_private(
            &staging.join("reservation.json"),
            &serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        let revisions = directory(
            &managed_project_root(dir.path(), "project")
                .unwrap()
                .join("revisions"),
        )
        .unwrap();
        let receipt = revisions.join(format!(
            "{}.manifest.json",
            pending.descriptor.manifest_sha256
        ));
        vault::write_new_private(&receipt, &m.canonical_bytes().unwrap()).unwrap();
        // Receipt publication was durable, but data rename and SQL commit never happened.
        let completed = upload(&store, dir.path(), &m, b"hello");
        assert!(!staging.exists());
        assert!(
            managed_revision(dir.path(), "project", &completed.descriptor.manifest_sha256).is_ok()
        );
        // The same expired marker cannot remove a revision that did commit.
        remove_expired_orphan_receipt(dir.path(), &pending).unwrap();
        assert!(receipt.exists());
    }

    #[test]
    fn legacy_secrets_count_but_missing_receipts_and_untracked_uploads_fail_closed() {
        let (dir, store, m) = setup();
        let first = upload(&store, dir.path(), &m, b"hello");
        let root = PathBuf::from(first.project_path.unwrap());
        let secrets =
            directory(&directory(&root.join(".secrets")).unwrap().join("placement")).unwrap();
        vault::write_new_private(&secrets.join("TOKEN"), b"private").unwrap();
        let next = variant(&m, "project", b"next!");
        let id = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "controller",
            &next.descriptor().unwrap(),
        )
        .unwrap()
        .transfer_id;
        abort(&store, dir.path(), "project", &id, "controller").unwrap();

        let orphan = transfer_dir(dir.path(), &uuid::Uuid::new_v4().to_string()).unwrap();
        vault::write_new_private(&orphan.join("reservation.json"), b"pending transaction").unwrap();
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &next.descriptor().unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("Untracked artifact staging")
        );
        std::fs::remove_dir_all(orphan).unwrap();
        std::fs::remove_file(root.parent().unwrap().join(format!(
            "{}.manifest.json",
            first.descriptor.manifest_sha256
        )))
        .unwrap();
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &next.descriptor().unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("no receipt")
        );
        assert_eq!(
            std::fs::read(root.join("apps/project/manifest.app")).unwrap(),
            b"hello"
        );
    }
    #[test]
    fn online_dependency_revision_cannot_be_used_as_an_offline_project() {
        let (dir, store, mut manifest) = setup();
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":1,"project_id":"project","source":"online"}),
        )
        .unwrap();
        manifest.source = flow_like_device_protocol::ProjectArtifactSource::Online;
        manifest.files[0] = ProjectArtifactFile {
            path: "apps/project/online-source.json".into(),
            size: bytes.len() as u64,
            sha256: artifact_sha256(&bytes),
        };
        let transfer = start(&store, dir.path(), &manifest);
        chunk(
            &store,
            dir.path(),
            "project",
            &transfer,
            "controller",
            Some(0),
            0,
            &URL_SAFE_NO_PAD.encode(&bytes),
        )
        .unwrap();
        let committed = commit(&store, dir.path(), "project", &transfer, "controller").unwrap();
        let digest = &committed.descriptor.manifest_sha256;
        assert!(
            managed_revision_for_source(
                dir.path(),
                "project",
                digest,
                flow_like_device_protocol::ProjectArtifactSource::Online
            )
            .is_ok()
        );
        assert!(managed_revision(dir.path(), "project", digest).is_err());
        assert!(commit(&store, dir.path(), "project", &transfer, "controller").is_ok());
        assert!(
            status(
                &store,
                dir.path(),
                "project",
                &transfer,
                "controller",
                Some(0)
            )
            .unwrap()
            .complete
        );
        let again = upload(&store, dir.path(), &manifest, &bytes);
        assert_eq!(again.descriptor.manifest_sha256, *digest);
    }
    #[test]
    fn managed_project_ids_reject_case_aliases_before_import_or_cache_access() {
        let (dir, store, manifest) = setup();
        let upper = managed_project_root(dir.path(), "Project").unwrap();
        assert_eq!(managed_project_root(dir.path(), "Project").unwrap(), upper);
        assert!(prepare_online_cache(dir.path(), "Project").is_ok());
        assert!(prepare_online_cache(dir.path(), "project").is_err());
        assert!(managed_revision(dir.path(), "project", &artifact_sha256(b"x")).is_err());
        let id = uuid::Uuid::new_v4().to_string();
        assert!(
            begin(
                &store,
                dir.path(),
                &id,
                "controller",
                &manifest.descriptor().unwrap()
            )
            .is_err()
        );
        let count: u64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM project_artifact_transfers",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert!(upper.join("online-cache").is_dir());
    }

    #[test]
    fn resumes_exact_chunks_rejects_other_principals_and_commits_complete_snapshot() {
        let (dir, store, m) = setup();
        let id = start(&store, dir.path(), &m);
        assert!(status(&store, dir.path(), "project", &id, "intruder", None).is_err());
        assert!(status(&store, dir.path(), "other", &id, "controller", None).is_err());
        let send = |offset, data: &[u8]| {
            chunk(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                offset,
                &URL_SAFE_NO_PAD.encode(data),
            )
        };
        assert_eq!(send(0, b"he").unwrap().offset, 2);
        assert_eq!(send(0, b"he").unwrap().offset, 2);
        assert!(send(0, b"xx").is_err());
        assert!(send(3, b"lo").is_err());
        assert!(commit(&store, dir.path(), "project", &id, "controller").is_err());
        assert!(send(2, b"llo").unwrap().complete);
        let result = commit(&store, dir.path(), "project", &id, "controller").unwrap();
        let path = PathBuf::from(result.project_path.unwrap());
        assert_eq!(
            std::fs::read(path.join("apps/project/manifest.app")).unwrap(),
            b"hello"
        );
        assert!(commit(&store, dir.path(), "project", &id, "controller").is_ok());
        assert!(send(0, b"hello").is_err());
    }
    #[test]
    fn corrupt_file_resets_only_that_file_and_enforces_transfer_quota() {
        let (dir, store, m) = setup();
        let id = start(&store, dir.path(), &m);
        assert!(
            chunk(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &URL_SAFE_NO_PAD.encode(b"wrong")
            )
            .is_err()
        );
        assert_eq!(
            status(&store, dir.path(), "project", &id, "controller", Some(0))
                .unwrap()
                .offset,
            0
        );
        for _ in 0..3 {
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &m.descriptor().unwrap(),
            )
            .unwrap();
        }
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &m.descriptor().unwrap()
            )
            .is_err()
        );
        abort(&store, dir.path(), "project", &id, "controller").unwrap();
        assert!(
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                "controller",
                &m.descriptor().unwrap()
            )
            .is_ok()
        );
    }
    #[test]
    fn local_import_copies_only_selected_verified_model_and_package_assets() {
        let (dir, store, _) = setup();
        let source = tempfile::tempdir().unwrap();
        for path in [
            "apps/project",
            "bits/metadata",
            "bits/hash/vision_encoder",
            "bits/unselected",
            "packages/package/1.0.0",
        ] {
            std::fs::create_dir_all(source.path().join(path)).unwrap();
        }
        std::fs::write(source.path().join("apps/project/manifest.app"), b"project").unwrap();
        std::fs::write(
            source.path().join("bits/hash/vision_encoder/weights.bin"),
            b"weights",
        )
        .unwrap();
        std::fs::write(
            source.path().join("bits/unselected/private.bin"),
            b"private",
        )
        .unwrap();
        std::fs::write(
            source.path().join("packages/package/1.0.0/module.wasm"),
            b"wasm",
        )
        .unwrap();
        std::fs::write(
            source.path().join("packages/package/1.0.0/manifest.json"),
            b"manifest",
        )
        .unwrap();
        let metadata=serde_json::to_vec(&serde_json::json!({"bit":{"id":"model","hash":"hash","file_name":"vision_encoder/weights.bin","size":7},"dependencies":[],"artifacts":[{"path":"bits/hash/vision_encoder/weights.bin","size":7,"sha256":artifact_sha256(b"weights")}]})).unwrap();
        std::fs::write(source.path().join("bits/metadata/model.json"), &metadata).unwrap();
        let assets = ProjectArtifactAssets {
            bit_pins: vec![flow_like_device_protocol::ProjectBitPin {
                bit_id: "model".into(),
                metadata_sha256: artifact_sha256(&metadata),
            }],
            package_pins: vec![flow_like_device_protocol::ProjectPackagePin {
                package_id: "package".into(),
                version: "1.0.0".into(),
                wasm_sha256: artifact_sha256(b"wasm"),
                manifest_sha256: artifact_sha256(b"manifest"),
            }],
        };
        let result =
            import_local_selected(&store, dir.path(), "project", source.path(), &assets).unwrap();
        let path = PathBuf::from(result.project_path.unwrap());
        assert_eq!(
            std::fs::read(path.join("bits/hash/vision_encoder/weights.bin")).unwrap(),
            b"weights"
        );
        assert!(!path.join("bits/unselected").exists());
        assert_eq!(
            std::fs::read(path.join("packages/package/1.0.0/module.wasm")).unwrap(),
            b"wasm"
        );
        std::fs::write(
            source.path().join("bits/hash/vision_encoder/weights.bin"),
            b"WRONG!!",
        )
        .unwrap();
        assert!(
            import_local_selected(&store, dir.path(), "project", source.path(), &assets).is_err()
        );
    }

    #[test]
    fn bit_metadata_rejects_nested_traversal_even_when_digest_is_pinned() {
        for name in [
            "vision_encoder/../weights.bin",
            "vision_encoder//weights.bin",
            "vision_encoder/.secrets/key",
            "vision_encoder\\weights.bin",
        ] {
            let metadata = serde_json::to_vec(&serde_json::json!({
                "bit": {"id": "model", "hash": "hash", "file_name": name, "size": 7},
                "dependencies": [],
                "artifacts": [{"path": format!("bits/hash/{name}"), "size": 7, "sha256": artifact_sha256(b"weights")}]
            })).unwrap();
            let pin = flow_like_device_protocol::ProjectBitPin {
                bit_id: "model".into(),
                metadata_sha256: artifact_sha256(&metadata),
            };
            assert!(metadata_assets(&metadata, &pin).is_err(), "{name}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_staging_file_and_cross_project_import_are_rejected() {
        let (dir, store, m) = setup();
        let id = start(&store, dir.path(), &m);
        let outside = dir.path().join("outside");
        vault::write_new_private(&outside, b"untouched").unwrap();
        let staging = transfer_dir(dir.path(), &id).unwrap();
        let destination = data_path(&staging, "apps/project/manifest.app").unwrap();
        std::os::unix::fs::symlink(&outside, &destination).unwrap();
        assert!(
            chunk(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &URL_SAFE_NO_PAD.encode(b"hello")
            )
            .is_err()
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
        abort(&store, dir.path(), "project", &id, "controller").unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(source.path().join("apps/project")).unwrap();
        std::fs::create_dir_all(source.path().join("apps/other")).unwrap();
        std::fs::write(source.path().join("apps/project/manifest.app"), b"hello").unwrap();
        std::fs::write(source.path().join("apps/other/manifest.app"), b"private").unwrap();
        let receipt = import_local(&store, dir.path(), "project", source.path()).unwrap();
        let path = PathBuf::from(receipt.project_path.unwrap());
        assert!(!path.join("apps/other").exists());
    }
}
