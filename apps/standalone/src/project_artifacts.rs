//! Imported revisions bind a complete initial project snapshot. Runtime databases may change later.
use crate::models::acquire::AcquisitionManager;
use crate::{enrollment::unix_time, state::StateStore, supervisor, vault};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    ArtifactTransferState, ArtifactTransferStatus, PROJECT_ARTIFACT_CHUNK_BYTES,
    PROJECT_ARTIFACT_TTL_SECONDS, ProjectArtifactDescriptor, ProjectArtifactManifest,
    artifact_sha256, validate_artifact_digest, validate_artifact_project_id,
    validate_artifact_prune,
};
use flow_like_device_protocol::{
    MODEL_ASSET_MAX_SOURCES, MODEL_ENSURE_MAX_PINS, MODEL_MAX_PENDING_ASSETS, ModelAssetDescriptor,
    ModelAssetDigest, ModelAssetState, ModelAssetSummary, PACKAGED_BIT_METADATA_MAX_BYTES,
    PackagedBitMetadata, ProjectBitPin,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// Stays below the controller's 15 second response deadline. A timeout keeps its
/// `WouldBlock` cause, so the management plane reports it as retryable.
const ARTIFACT_LOCK_WAIT: Duration = Duration::from_secs(10);
const MAX_RECEIVING_PER_PRINCIPAL: u64 = 4;
const MAX_TRANSFERS_PER_PRINCIPAL: u64 = 256;
const MAX_RECEIVING_BYTES_PER_PRINCIPAL: u64 = 16 * 1024 * 1024 * 1024;
const MAX_RECEIVING: u64 = 64;
const MAX_TRANSFERS: u64 = 4096;
pub const RAW_ARTIFACT_CHUNK_BYTES: usize = 256 * 1024;

fn artifact_lock(root: &Path) -> Result<File> {
    supervisor::lock_file_within(&root.join("artifact.lock"), ARTIFACT_LOCK_WAIT)
}

/// A staging or storage budget refusal. Retrying cannot succeed until uploads
/// finish, expire or an operator changes the budget, so it is not retryable.
#[derive(Debug)]
pub struct ArtifactLimitExceeded(String);

impl std::fmt::Display for ArtifactLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ArtifactLimitExceeded {}

/// Why listed revisions stay on the device. Nothing was removed.
#[derive(Debug)]
pub enum PruneRefused {
    /// An upload or a restart is under way; the same request can succeed shortly.
    Busy(String),
    /// A placement or an update in progress still uses a listed revision.
    InUse(String),
}

impl std::fmt::Display for PruneRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::Busy(message) | Self::InUse(message)) = self;
        f.write_str(message)
    }
}

impl std::error::Error for PruneRefused {}

type FileStamp = (u64, u64, u64, i64, i64, i64, i64);

#[cfg(unix)]
fn file_stamp(metadata: &std::fs::Metadata) -> FileStamp {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[cfg(not(unix))]
fn file_stamp(metadata: &std::fs::Metadata) -> FileStamp {
    (0, 0, metadata.len(), 0, 0, 0, 0)
}

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
            ArtifactLimitExceeded(format!(
                "Artifact storage budget exceeded for {scope}: {} / {} charged bytes, {} / {} file/directory entries, {} / {} revisions. Each in-flight file reserves 34 entries for maximum path depth until commit. An operator must remove unused revisions or increase the limit in agent.env",
                self.bytes, limit.bytes, self.files, limit.files, self.revisions, limit.revisions
            ))
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

#[derive(Default)]
struct ArtifactAccounting {
    device: ArtifactUsage,
    projects: std::collections::BTreeMap<String, ArtifactUsage>,
    scanned: usize,
    /// A project whose retained revisions are also charged one by one, by manifest digest.
    detail: Option<(String, std::collections::BTreeMap<String, ArtifactUsage>)>,
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
    let mut accounting = ArtifactAccounting::default();
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

    /// Charges part of a retained revision to its project.
    fn retain(&mut self, project: &str, digest: &str, usage: ArtifactUsage) -> Result<()> {
        if let Some((detailed, revisions)) = &mut self.detail
            && detailed == project
        {
            revisions.entry(digest.to_owned()).or_default().add(usage)?;
        }
        self.add(project, usage)
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

/// Admits one more upload within the device and project budgets. Existing revision
/// reads do not need this scan or a budget increase.
fn admit_artifact(
    store: &StateStore,
    root: &Path,
    descriptor: &ProjectArtifactDescriptor,
    recovered: Option<&str>,
) -> Result<()> {
    let budgets = crate::isolation::artifact_budgets(root)?;
    crate::config::ensure_unaliased_child(
        &directory(&root.join("projects"))?,
        &descriptor.project_id,
    )?;
    let (mut accounting, resumed) = charged(store, root, recovered, None)?;
    accounting.add(
        &descriptor.project_id,
        resumed.unwrap_or_else(|| ArtifactUsage::reservation(descriptor)),
    )?;
    accounting.device.enforce(budgets.device, "device")?;
    accounting.projects[&descriptor.project_id].enforce(budgets.project, "project")
}

/// Reconstruct what admission charges from durable receipts and directories, rather
/// than expiring transfer rows. The artifact lock serializes it with commit/abort.
/// `recovered` names the staging directory of a transfer that resumes: its charge is
/// returned instead of added. `detail` names a project whose revisions are also charged
/// one by one.
fn charged(
    store: &StateStore,
    root: &Path,
    recovered: Option<&str>,
    detail: Option<&str>,
) -> Result<(ArtifactAccounting, Option<ArtifactUsage>)> {
    reconcile_expired_reservations(store, root)?;
    let mut accounting = ArtifactAccounting {
        detail: detail.map(|project| (project.to_owned(), Default::default())),
        ..Default::default()
    };
    accounting.charge_projects(root)?;
    let tracked = accounting.charge_transfers(store, root)?;
    let resumed = accounting.charge_staging(root, &tracked, recovered)?;
    Ok((accounting, resumed))
}

impl ArtifactAccounting {
    /// Every managed project's directory and what it retains.
    fn charge_projects(&mut self, root: &Path) -> Result<()> {
        let projects = directory(&root.join("projects"))?;
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
            let (metadata, usage) = self.entry(&project.path())?;
            ensure!(metadata.is_dir(), "Managed project is not a directory");
            self.add(&id, usage)?;
            let revisions = project.path().join("revisions");
            if !entry_exists(&revisions)? {
                continue;
            }
            let (metadata, usage) = self.entry(&revisions)?;
            ensure!(metadata.is_dir(), "Managed revisions must be a directory");
            self.add(&id, usage)?;
            self.charge_revisions(&id, &revisions)?;
        }
        Ok(())
    }

    /// One project's receipts and the revision directories they vouch for.
    fn charge_revisions(&mut self, id: &str, revisions: &Path) -> Result<()> {
        let mut revision_count = 0;
        for revision in std::fs::read_dir(revisions)? {
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
                let (metadata, mut usage) = self.entry(&revision.path())?;
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
                self.retain(id, digest, usage)?;
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
                let usage = self.tree(&revision.path(), 0)?;
                self.retain(id, &name, usage)?;
            }
        }
        Ok(())
    }

    /// Every transfer the device tracks, by its staging directory; an upload in flight
    /// holds its whole reservation. Returns the tracked transfer IDs.
    fn charge_transfers(
        &mut self,
        store: &StateStore,
        root: &Path,
    ) -> Result<std::collections::HashSet<String>> {
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
                self.tree(&path, 0)?
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
            self.add(&project, usage)?;
        }
        Ok(known)
    }

    /// Staging directories without a transfer row. Returns the charge of the one that
    /// `recovered` names instead of adding it.
    fn charge_staging(
        &mut self,
        root: &Path,
        tracked: &std::collections::HashSet<String>,
        recovered: Option<&str>,
    ) -> Result<Option<ArtifactUsage>> {
        let mut resumed = None;
        let staging = root.join("artifact-transfers");
        if !entry_exists(&staging)? {
            return Ok(resumed);
        }
        let (metadata, usage) = self.entry(&staging)?;
        ensure!(metadata.is_dir(), "Artifact staging must be a directory");
        self.device.add(usage)?;
        for entry in std::fs::read_dir(&staging)? {
            let entry = entry?;
            if tracked.contains(&entry.file_name().to_string_lossy().into_owned()) {
                continue;
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid untracked transfer ID"))?;
            uuid(&id)?;
            ensure!(
                self.entry(&entry.path())?.0.is_dir(),
                "Untracked artifact staging must be a directory"
            );
            let actual = self.tree(&entry.path(), 0)?;
            if std::fs::read_dir(entry.path())?.next().is_none() {
                self.device.add(actual)?;
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
                resumed = Some(usage);
            } else {
                self.add(&pending.descriptor.project_id, usage)?;
            }
        }
        Ok(resumed)
    }
}

/// A transfer past its lifetime no longer holds staging or budget.
fn drop_expired_transfers(store: &StateStore, root: &Path, now: i64) -> Result<()> {
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
    Ok(())
}

fn budget_json(used: ArtifactUsage, limit: crate::isolation::ArtifactLimits) -> Value {
    json!({
        "bytes": {"used": used.bytes, "max": limit.bytes},
        "entries": {"used": used.files, "max": limit.files},
        "revisions": {"used": used.revisions, "max": limit.revisions},
    })
}

pub(crate) struct StorageUse {
    /// What the whole device is charged, next to its budget.
    pub device: Value,
    /// The same for the requested project, and what each of its retained revisions is
    /// charged, in digest order.
    pub project: Option<(Value, Vec<(String, u64)>)>,
}

/// What uploads are charged right now, exactly as the next admission counts it, so
/// transfers past their lifetime are dropped first. It walks every retained file: ask on
/// demand only.
pub(crate) fn usage(store: &StateStore, root: &Path, project: Option<&str>) -> Result<StorageUse> {
    project.map(validate_artifact_project_id).transpose()?;
    private_root(root)?;
    let _lock = artifact_lock(root)?;
    drop_expired_transfers(store, root, unix_time()?)?;
    let budgets = crate::isolation::artifact_budgets(root)?;
    let (accounting, _) = charged(store, root, None, project)?;
    let project = project.map(|project| {
        let used = accounting
            .projects
            .get(project)
            .copied()
            .unwrap_or_default();
        let revisions = accounting.detail.map_or_else(Vec::new, |(_, revisions)| {
            revisions
                .into_iter()
                .map(|(digest, charge)| (digest, charge.bytes))
                .collect()
        });
        (budget_json(used, budgets.project), revisions)
    });
    Ok(StorageUse {
        device: budget_json(accounting.device, budgets.device),
        project,
    })
}

/// A project path that a placement runs from, or that an update in progress may switch
/// to or back to.
struct Pin {
    placement: Option<String>,
    path: PathBuf,
    resolved: Option<PathBuf>,
}

impl Pin {
    fn new(placement: Option<String>, path: String) -> Self {
        let path = PathBuf::from(path);
        Self {
            placement,
            resolved: path.canonicalize().ok(),
            path,
        }
    }

    /// At or below the directory, by its spelling or by where the filesystem resolves it.
    fn within(&self, directory: &Path, resolved: Option<&Path>) -> bool {
        self.path.starts_with(directory)
            || self
                .resolved
                .as_deref()
                .zip(resolved)
                .is_some_and(|(pin, directory)| pin.starts_with(directory))
    }
}

fn pins(store: &StateStore) -> Result<Vec<Pin>> {
    let mut placements = store.connection.prepare(
        "SELECT id,json_extract(config_json,'$.project_path') FROM placements ORDER BY id",
    )?;
    let mut pins = placements
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .filter_map(|placement| {
            placement
                .map(|(id, path)| path.map(|path| Pin::new(Some(id), path)))
                .transpose()
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let updating = store.active_rollout_project_paths()?;
    pins.extend(updating.into_iter().map(|path| Pin::new(None, path)));
    Ok(pins)
}

/// The placements named for one revision; more can use it.
const NAMED_PLACEMENTS: usize = 32;

#[derive(serde::Serialize)]
pub(crate) struct RevisionUse {
    pub revision: String,
    pub bytes: u64,
    /// Placements whose configuration runs from this revision.
    pub referenced_by: Vec<String>,
    /// An update in progress may still switch to it or back to it.
    pub rollout: bool,
}

impl RevisionUse {
    fn new(pins: &[Pin], revisions: &Path, revision: &str, bytes: u64) -> Self {
        let directory = revisions.join(revision);
        let resolved = directory.canonicalize().ok();
        let mut used = Self {
            revision: revision.to_owned(),
            bytes,
            referenced_by: Vec::new(),
            rollout: false,
        };
        for pin in pins
            .iter()
            .filter(|pin| pin.within(&directory, resolved.as_deref()))
        {
            match &pin.placement {
                Some(placement) if used.referenced_by.len() < NAMED_PLACEMENTS => {
                    used.referenced_by.push(placement.clone())
                }
                Some(_) => (),
                None => used.rollout = true,
            }
        }
        used
    }

    fn pinned(&self) -> bool {
        self.rollout || !self.referenced_by.is_empty()
    }
}

/// Who still uses each listed revision of a project. The caller holds the management write
/// transaction when the answer has to stay true.
pub(crate) fn revision_uses(
    store: &StateStore,
    root: &Path,
    project: &str,
    revisions: &[(String, u64)],
) -> Result<Vec<RevisionUse>> {
    validate_artifact_project_id(project)?;
    let directory = root.join("projects").join(project).join("revisions");
    let pins = pins(store)?;
    Ok(revisions
        .iter()
        .map(|(revision, bytes)| RevisionUse::new(&pins, &directory, revision, *bytes))
        .collect())
}

/// Where removed revisions wait until their files are deleted. It lies outside
/// `revisions`, whose entries every agent version accounts for by name.
const PRUNED: &str = ".pruned";

/// An existing private directory; unlike `directory`, never creates one.
fn existing_directory(path: &Path) -> Result<Option<PathBuf>> {
    if entry_exists(path)? {
        directory(path).map(Some)
    } else {
        Ok(None)
    }
}

/// Deletes what a removal moved aside. A crash can leave it behind.
fn discard_pruned(home: &Path) -> Result<()> {
    if let Some(pruned) = existing_directory(&home.join(PRUNED))? {
        std::fs::remove_dir_all(pruned)?;
        File::open(home)?.sync_all()?;
    }
    Ok(())
}

/// What a retained revision is charged: its receipt and its directory, whichever exist.
fn retained_bytes(revisions: &Path, revision: &str) -> Result<Option<u64>> {
    let mut accounting = ArtifactAccounting::default();
    let mut charge = None::<ArtifactUsage>;
    for path in [
        revisions.join(format!("{revision}.manifest.json")),
        revisions.join(revision),
    ] {
        if entry_exists(&path)? {
            charge
                .get_or_insert_default()
                .add(accounting.tree(&path, 0)?)?;
        }
    }
    Ok(charge.map(|charge| charge.bytes))
}

/// A project's own directory when it has one. Never creates it, and refuses a spelling
/// that the filesystem would resolve to another project.
fn project_home(root: &Path, project: &str) -> Result<Option<PathBuf>> {
    validate_artifact_project_id(project)?;
    let projects = directory(&root.join("projects"))?;
    crate::config::ensure_unaliased_child(&projects, project)?;
    existing_directory(&projects.join(project))
}

/// The listed revisions a project still retains, with what each is charged. What an
/// interrupted removal left behind is deleted first.
fn retained(home: &Path, revisions: &[String]) -> Result<Vec<(String, u64)>> {
    discard_pruned(home)?;
    let Some(directory) = existing_directory(&home.join("revisions"))? else {
        return Ok(Vec::new());
    };
    let mut retained = Vec::new();
    for revision in revisions {
        if let Some(bytes) = retained_bytes(&directory, revision)? {
            retained.push((revision.clone(), bytes));
        }
    }
    Ok(retained)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

/// Listed revisions of one project that are held for removal under the artifact lock,
/// which keeps uploads from committing or measuring in between.
pub(crate) struct Prune {
    _lock: File,
    project: String,
    home: Option<PathBuf>,
    retained: Vec<(String, u64)>,
}

impl Prune {
    /// Takes the artifact lock and measures the listed revisions that are still retained.
    /// Call it before the management write transaction begins, so that uploads never wait
    /// for the lock behind an open transaction.
    pub(crate) fn prepare(root: &Path, project: &str, revisions: &[String]) -> Result<Self> {
        validate_artifact_prune(revisions)?;
        private_root(root)?;
        let lock = artifact_lock(root)?;
        let home = project_home(root, project)?;
        let retained = home
            .as_deref()
            .map(|home| retained(home, revisions))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            _lock: lock,
            project: project.to_owned(),
            home,
            retained,
        })
    }

    /// Inside the management write transaction: nothing can pin a revision between this
    /// check and the removal. Refuses as a whole while an upload of the project is in
    /// flight, while a process still runs a superseded configuration, or when a listed
    /// revision is in use. Otherwise every listed revision leaves the project; revisions
    /// that were already gone are skipped. Returns what was removed and the bytes it was
    /// charged.
    pub(crate) fn remove(&self, store: &StateStore, now: i64) -> Result<(Vec<String>, u64)> {
        let Some(home) = self.home.as_deref().filter(|_| !self.retained.is_empty()) else {
            return Ok((Vec::new(), 0));
        };
        self.refuse_while_busy(store, now)?;
        let revisions = home.join("revisions");
        self.refuse_in_use(store, &revisions)?;
        self.set_aside(home, &revisions)?;
        self.remove_receipts(&revisions)?;
        Ok((
            self.retained
                .iter()
                .map(|(revision, _)| revision.clone())
                .collect(),
            self.retained.iter().map(|(_, bytes)| bytes).sum(),
        ))
    }

    /// An upload in flight may be about to reuse a listed revision, and a process that
    /// still runs a superseded configuration may still run from one.
    fn refuse_while_busy(&self, store: &StateStore, now: i64) -> Result<()> {
        let project = &self.project;
        let receiving: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_artifact_transfers WHERE project_id=?1 AND state='receiving' AND expires_at>?2)",
            params![project, now],
            |row| row.get(0),
        )?;
        ensure!(
            !receiving,
            PruneRefused::Busy(format!(
                "An upload for project {project} is in progress; wait for it to finish or cancel it before removing revisions"
            ))
        );
        let replacing: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM placement_replicas r JOIN placements p ON p.id=r.placement_id WHERE json_extract(p.config_json,'$.project_id')=?1 AND r.process_id IS NOT NULL AND r.config_revision<>p.config_revision)",
            [project],
            |row| row.get(0),
        )?;
        ensure!(
            !replacing,
            PruneRefused::Busy(format!(
                "A service of project {project} still replaces processes that run an earlier configuration; retry shortly"
            ))
        );
        Ok(())
    }

    fn refuse_in_use(&self, store: &StateStore, revisions: &Path) -> Result<()> {
        let pins = pins(store)?;
        for (revision, bytes) in &self.retained {
            let used = RevisionUse::new(&pins, revisions, revision, *bytes);
            let user = if used.referenced_by.is_empty() {
                "an update in progress".to_owned()
            } else {
                format!("placement {}", used.referenced_by.join(", "))
            };
            ensure!(
                !used.pinned(),
                PruneRefused::InUse(format!(
                    "Revision {revision} of project {} is still used by {user}; nothing was removed",
                    self.project
                ))
            );
        }
        Ok(())
    }

    /// A directory is durably gone before its receipt is: a revision directory without a
    /// receipt would stop every upload until an operator reconciles it.
    fn set_aside(&self, home: &Path, revisions: &Path) -> Result<()> {
        let pruned = directory(&home.join(PRUNED))?;
        for (revision, _) in &self.retained {
            let retained = revisions.join(revision);
            if entry_exists(&retained)? {
                std::fs::rename(retained, pruned.join(revision))?;
            }
        }
        sync_directory(&pruned)?;
        sync_directory(revisions)
    }

    fn remove_receipts(&self, revisions: &Path) -> Result<()> {
        for (revision, _) in &self.retained {
            let receipt = revisions.join(format!("{revision}.manifest.json"));
            if entry_exists(&receipt)? {
                std::fs::remove_file(receipt)?;
            }
        }
        sync_directory(revisions)
    }

    /// Deletes the removed files once the transaction has ended. The next removal in the
    /// project deletes what this one could not.
    pub(crate) fn discard(self) {
        if let Some(home) = &self.home
            && let Err(error) = discard_pruned(home)
        {
            tracing::warn!(project_id = %self.project, "Removed project revisions could not be deleted yet: {error:#}");
        }
    }
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
    stamp: FileStamp,
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
    let stamp = file_stamp(&metadata);
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

/// Staging slots are partitioned per controller principal, so one principal's
/// abandoned uploads cannot block another. The device-wide bounds only cap
/// bookkeeping, and the device owner is exempt from them so grantees can never
/// lock it out; its own per-principal bounds still apply. Storage is charged by
/// admission.
fn enforce_staging_limits(
    store: &StateStore,
    owner: &str,
    descriptor: &ProjectArtifactDescriptor,
    device_owner: bool,
) -> Result<()> {
    let (receiving, transfers, own_receiving, own_transfers, own_bytes): (u64, u64, u64, u64, u64) =
        store.connection.query_row(
            "SELECT COALESCE(SUM(state='receiving'),0),COUNT(*),COALESCE(SUM(state='receiving' AND principal=?1),0),COALESCE(SUM(principal=?1),0),COALESCE(SUM(CASE WHEN state='receiving' AND principal=?1 THEN json_extract(descriptor_json,'$.total_bytes')+json_extract(descriptor_json,'$.manifest_size') ELSE 0 END),0) FROM project_artifact_transfers",
            [owner],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    ensure!(
        own_receiving < MAX_RECEIVING_PER_PRINCIPAL,
        ArtifactLimitExceeded(format!(
            "This controller already has {own_receiving} of {MAX_RECEIVING_PER_PRINCIPAL} artifact uploads in progress on the device; finish or abort one first"
        ))
    );
    ensure!(
        own_transfers < MAX_TRANSFERS_PER_PRINCIPAL,
        ArtifactLimitExceeded(format!(
            "This controller started {own_transfers} of {MAX_TRANSFERS_PER_PRINCIPAL} artifact transfers allowed per day; older transfers expire after 24 hours"
        ))
    );
    ensure!(
        own_bytes
            .checked_add(descriptor.total_bytes + descriptor.manifest_size)
            .is_some_and(|bytes| bytes <= MAX_RECEIVING_BYTES_PER_PRINCIPAL),
        ArtifactLimitExceeded(format!(
            "This controller's in-progress artifact uploads would exceed {} GiB of staging",
            MAX_RECEIVING_BYTES_PER_PRINCIPAL / 1024 / 1024 / 1024
        ))
    );
    ensure!(
        device_owner || (receiving < MAX_RECEIVING && transfers < MAX_TRANSFERS),
        ArtifactLimitExceeded(format!(
            "Device artifact staging is full: {receiving} of {MAX_RECEIVING} uploads in progress and {transfers} of {MAX_TRANSFERS} transfer records"
        ))
    );
    Ok(())
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
    begin_transfer(store, root, transfer_id, owner, descriptor, false)
}

/// Starts a transfer for the device owner, which the device-wide staging bounds
/// never refuse.
pub fn begin_as_device_owner(
    store: &StateStore,
    root: &Path,
    transfer_id: &str,
    owner: &str,
    descriptor: &ProjectArtifactDescriptor,
) -> Result<ArtifactTransferStatus> {
    begin_transfer(store, root, transfer_id, owner, descriptor, true)
}

fn begin_transfer(
    store: &StateStore,
    root: &Path,
    transfer_id: &str,
    owner: &str,
    descriptor: &ProjectArtifactDescriptor,
    device_owner: bool,
) -> Result<ArtifactTransferStatus> {
    descriptor.validate()?;
    uuid(transfer_id)?;
    principal(owner)?;
    private_root(root)?;
    let _lock = artifact_lock(root)?;
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
    drop_expired_transfers(store, root, now)?;
    enforce_staging_limits(store, owner, descriptor, device_owner)?;
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
    let _lock = artifact_lock(root)?;
    status_inner(
        root,
        transfer_id,
        load(store, transfer_id, project, owner, true)?,
        index,
    )
}

/// A complete file whose digest is checked without holding the device-wide
/// artifact lock. The stamp detects any change before the result is recorded.
struct PendingVerification {
    file: File,
    stamp: FileStamp,
    path: PathBuf,
    size: u64,
    digest: String,
    marker: Option<PathBuf>,
    dir: PathBuf,
}

fn file_sha256(file: &mut File, size: u64) -> Result<Option<String>> {
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
    Ok((received == size).then(|| format!("{:x}", hash.finalize())))
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
    chunk_bytes(
        store,
        root,
        project,
        transfer_id,
        owner,
        index,
        offset,
        &bytes,
    )
}

/// Tunnel uploads use raw bytes and retain the same durable offsets and checksum checks.
pub fn chunk_bytes(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
    index: Option<u32>,
    offset: u64,
    bytes: &[u8],
) -> Result<ArtifactTransferStatus> {
    ensure!(
        bytes.len() <= RAW_ARTIFACT_CHUNK_BYTES,
        "Artifact chunk exceeds its bound"
    );
    let mut pending = {
        let _lock = artifact_lock(root)?;
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
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        let verified = match &marker {
            Some(marker) => checked_marker(marker, &digest)?,
            None => value.manifest_ready,
        };
        let metadata = file.metadata()?;
        if metadata.len() != size || verified {
            return status_inner(root, transfer_id, value, index);
        }
        PendingVerification {
            file,
            stamp: file_stamp(&metadata),
            path,
            size,
            digest,
            marker,
            dir,
        }
    };
    // Hashing up to 4 GiB must not stall other controllers' transfers.
    let matches =
        file_sha256(&mut pending.file, pending.size)?.as_deref() == Some(pending.digest.as_str());
    let _lock = artifact_lock(root)?;
    let value = load(store, transfer_id, project, owner, false)?;
    ensure!(
        file_stamp(&open(&pending.path, false, false)?.metadata()?) == pending.stamp,
        "Artifact file changed while it was verified; retry its final chunk"
    );
    if !matches {
        pending.file.set_len(0)?;
        pending.file.sync_all()?;
        if let Some(marker) = &pending.marker {
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
    if let Some(parent) = pending.path.parent() {
        File::open(parent)?.sync_all()?;
    }
    if let Some(marker) = &pending.marker {
        if !checked_marker(marker, &pending.digest)? {
            vault::write_new_private(marker, pending.digest.as_bytes())?;
        }
    } else {
        manifest(&pending.dir, &value.descriptor)?;
        store.connection.execute(
            "UPDATE project_artifact_transfers SET manifest_ready=1 WHERE transfer_id=?1",
            [transfer_id],
        )?;
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
    commit_with(
        store,
        root,
        project,
        transfer_id,
        owner,
        model_store_supported(),
    )
}

fn commit_with(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: &str,
    model_store: bool,
) -> Result<ArtifactTransferStatus> {
    let _lock = artifact_lock(root)?;
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
    validate_selected_assets(&verification_root, &manifest, model_store)?;
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
    abort_transfer(store, root, project, transfer_id, Some(owner))
}

/// The device owner may release any principal's transfer in a project, so an
/// abandoned upload never holds staging or budget until it expires.
pub fn abort_as_device_owner(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
) -> Result<ArtifactTransferStatus> {
    abort_transfer(store, root, project, transfer_id, None)
}

fn abort_transfer(
    store: &StateStore,
    root: &Path,
    project: &str,
    transfer_id: &str,
    owner: Option<&str>,
) -> Result<ArtifactTransferStatus> {
    uuid(transfer_id)?;
    let _lock = artifact_lock(root)?;
    let owner = match owner {
        Some(owner) => owner.to_owned(),
        None => store
            .connection
            .query_row(
                "SELECT principal FROM project_artifact_transfers WHERE transfer_id=?1",
                [transfer_id],
                |row| row.get(0),
            )
            .optional()?
            .context("Unknown artifact transfer")?,
    };
    let value = load(store, transfer_id, project, &owner, true)?;
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
        load(store, transfer_id, project, &owner, true)?,
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
/// The metadata a pin names, checked against its digest and against the files it lists.
pub(crate) fn packaged_metadata(bytes: &[u8], pin: &ProjectBitPin) -> Result<PackagedBitMetadata> {
    ensure!(
        bytes.len() as u64 <= PACKAGED_BIT_METADATA_MAX_BYTES
            && artifact_sha256(bytes) == pin.metadata_sha256,
        "Selected Bit metadata digest differs"
    );
    let metadata: PackagedBitMetadata = serde_json::from_slice(bytes)
        .with_context(|| format!("Read the metadata of Bit {}", pin.bit_id))?;
    metadata
        .validate(&pin.bit_id)
        .with_context(|| format!("Check the metadata of Bit {}", pin.bit_id))?;
    Ok(metadata)
}

/// The agent advertises its model store once placements can use it; only then may metadata
/// leave model weights out of the artifact.
fn model_store_supported() -> bool {
    crate::diagnostics::model_host_available()
}

/// The artifact files a pin's metadata lists, by path.
fn metadata_assets(
    bytes: &[u8],
    pin: &ProjectBitPin,
    model_store: bool,
) -> Result<std::collections::BTreeMap<String, (u64, String)>> {
    let metadata = packaged_metadata(bytes, pin)?;
    ensure!(
        model_store || matches!(metadata, PackagedBitMetadata::V1(_)),
        "Bit {} uses metadata version 2, whose model assets need an agent with a model store",
        pin.bit_id
    );
    Ok(metadata
        .artifacts()
        .iter()
        .map(|file| (file.path.clone(), (file.size, file.sha256.clone())))
        .collect())
}
pub(crate) fn read_selected_metadata(
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
/// Every Bit file the pinned metadata lists as an artifact file is in the manifest, and the
/// manifest holds no other Bit file. Model-store assets are not artifact files.
fn validate_selected_assets(
    root: &Path,
    manifest: &ProjectArtifactManifest,
    model_store: bool,
) -> Result<()> {
    if manifest.source == flow_like_device_protocol::ProjectArtifactSource::Online {
        validate_online_marker(root, manifest)?;
    }
    let mut selected = std::collections::BTreeMap::new();
    for pin in &manifest.bit_pins {
        let bytes = read_selected_metadata(root, pin)?;
        for (path, size) in metadata_assets(&bytes, pin, model_store)? {
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

/// An online revision carries its source marker and nothing else names its source.
fn validate_online_marker(root: &Path, manifest: &ProjectArtifactManifest) -> Result<()> {
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
    Ok(())
}

/// `Models.Ensure`, the deploy step after an upload: starts or joins the acquisition of every
/// model-store asset that the pinned Bits name, deduplicated by digest. Each pin must belong to
/// a committed revision of the project; metadata v1 names no assets.
pub async fn ensure_project_models(
    root: &Path,
    acquisition: &AcquisitionManager,
    project_id: &str,
    pins: &[ProjectBitPin],
    operation_id: Option<&str>,
) -> Result<ModelAssetSummary> {
    let assets = {
        let (root, project, pins) = (root.to_owned(), project_id.to_owned(), pins.to_vec());
        tokio::task::spawn_blocking(move || committed_model_assets(&root, &project, &pins))
            .await??
    };
    summarize(acquisition, &assets, project_id, operation_id)
        .with_context(|| format!("Ensure the models of project {project_id}"))
}

/// One job per asset, started or joined; present assets are counted, the others listed.
/// The project leases each asset, so a collection under disk pressure keeps the files a
/// deploy fetched until its placement starts and references them.
fn summarize(
    acquisition: &AcquisitionManager,
    assets: &[ModelAssetDescriptor],
    project_id: &str,
    operation_id: Option<&str>,
) -> Result<ModelAssetSummary> {
    let mut summary = ModelAssetSummary {
        total: u32::try_from(assets.len())?,
        present: 0,
        pending: Vec::new(),
    };
    for asset in assets {
        acquisition.store().lease(&asset.digest, project_id)?;
        let status = acquisition.ensure(asset, operation_id)?;
        if status.state == ModelAssetState::Present {
            summary.present += 1;
        } else if summary.pending.len() < MODEL_MAX_PENDING_ASSETS {
            summary.pending.push(status);
        }
    }
    summary.validate()?;
    Ok(summary)
}

fn committed_model_assets(
    root: &Path,
    project: &str,
    pins: &[ProjectBitPin],
) -> Result<Vec<ModelAssetDescriptor>> {
    validate_artifact_project_id(project)?;
    ensure!(
        (1..=MODEL_ENSURE_MAX_PINS).contains(&pins.len()),
        "Ensure models: {} Bit pins, expected 1 to {MODEL_ENSURE_MAX_PINS}",
        pins.len()
    );
    for pin in pins {
        pin.validate()?;
    }
    let mut assets: Vec<ModelAssetDescriptor> = Vec::new();
    let mut listed = std::collections::HashMap::<ModelAssetDigest, usize>::new();
    for metadata in committed_metadata(root, project, pins)? {
        #[cfg(feature = "runtime")]
        crate::models::router::validate_packaged_model(&metadata)?;
        for asset in metadata.assets() {
            let descriptor = &asset.descriptor;
            let Some(&index) = listed.get(&descriptor.digest) else {
                listed.insert(descriptor.digest.clone(), assets.len());
                assets.push(descriptor.clone());
                continue;
            };
            merge_sources(&mut assets[index], descriptor)?;
        }
    }
    Ok(assets)
}

/// Two Bits naming the same digest name the same bytes; their sources add up.
fn merge_sources(known: &mut ModelAssetDescriptor, other: &ModelAssetDescriptor) -> Result<()> {
    ensure!(
        known.size == other.size,
        "Model asset {} is pinned with {} and with {} bytes",
        known.digest.store_key(),
        known.size,
        other.size
    );
    for source in &other.sources {
        if known.sources.len() < MODEL_ASSET_MAX_SOURCES && !known.sources.contains(source) {
            known.sources.push(source.clone());
        }
    }
    Ok(())
}

/// The metadata of each pin, read from a committed revision whose manifest pins it.
fn committed_metadata(
    root: &Path,
    project: &str,
    pins: &[ProjectBitPin],
) -> Result<Vec<PackagedBitMetadata>> {
    let _lock = artifact_lock(root)?;
    let mut found: Vec<Option<PackagedBitMetadata>> = vec![None; pins.len()];
    if let Some(revisions) = project_revisions(root, project)? {
        scan_revisions(&revisions, project, pins, &mut found)?;
    }
    pins.iter()
        .zip(found)
        .map(|(pin, metadata)| {
            metadata.with_context(|| {
                format!(
                    "Ensure models: no committed revision of project {project} pins Bit {} with metadata {}",
                    pin.bit_id, pin.metadata_sha256
                )
            })
        })
        .collect()
}

fn project_revisions(root: &Path, project: &str) -> Result<Option<PathBuf>> {
    match project_home(root, project)? {
        Some(home) => existing_directory(&home.join("revisions")),
        None => Ok(None),
    }
}

/// Reads the receipts of a project's revisions until every pin is found.
fn scan_revisions(
    revisions: &Path,
    project: &str,
    pins: &[ProjectBitPin],
    found: &mut [Option<PackagedBitMetadata>],
) -> Result<()> {
    for (count, entry) in std::fs::read_dir(revisions)?.enumerate() {
        ensure!(
            count < 32_768,
            "Project {project} exceeds its revision entry bound"
        );
        let name = entry?.file_name();
        if let Some(digest) = name
            .to_str()
            .and_then(|text| text.strip_suffix(".manifest.json"))
        {
            read_pinned_metadata(revisions, project, digest, pins, found)?;
        }
        if found.iter().all(Option::is_some) {
            break;
        }
    }
    Ok(())
}

/// Fills in the pins that one committed revision carries.
fn read_pinned_metadata(
    revisions: &Path,
    project: &str,
    digest: &str,
    pins: &[ProjectBitPin],
    found: &mut [Option<PackagedBitMetadata>],
) -> Result<()> {
    let Some(manifest) = revision_receipt(revisions, project, digest)? else {
        return Ok(());
    };
    let revision = revisions.join(digest);
    for (pin, slot) in pins.iter().zip(found.iter_mut()) {
        if slot.is_none() && manifest.bit_pins.contains(pin) {
            *slot = Some(packaged_metadata(
                &read_selected_metadata(&revision, pin)?,
                pin,
            )?);
        }
    }
    Ok(())
}

/// The manifest of a committed revision, or `None` when only its receipt is left.
fn revision_receipt(
    revisions: &Path,
    project: &str,
    digest: &str,
) -> Result<Option<ProjectArtifactManifest>> {
    validate_artifact_digest(digest)?;
    if !is_revision_directory(&revisions.join(digest)) {
        return Ok(None);
    }
    let bytes = read_receipt(&revisions.join(format!("{digest}.manifest.json")))?;
    parse_receipt(&bytes, project, digest).map(Some)
}

fn parse_receipt(bytes: &[u8], project: &str, digest: &str) -> Result<ProjectArtifactManifest> {
    let manifest: ProjectArtifactManifest = serde_json::from_slice(bytes)
        .with_context(|| format!("Read the receipt of revision {digest}"))?;
    ensure!(
        manifest.project_id == project
            && manifest.canonical_bytes()? == bytes
            && artifact_sha256(bytes) == digest,
        "The receipt of revision {digest} differs from its revision"
    );
    Ok(manifest)
}

fn is_revision_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn read_receipt(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open(path, false, false)?
        .take(flow_like_device_protocol::PROJECT_ARTIFACT_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
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
        selected.extend(metadata_assets(&metadata, pin, model_store_supported())?.into_keys());
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
    validate_selected_assets(&source, &manifest, model_store_supported())?;
    let id = uuid::Uuid::new_v4().to_string();
    let owner = "local-cli";
    begin_as_device_owner(store, root, &id, owner, &descriptor)?;
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
    // Each file is copied and hashed without the device-wide artifact lock, so
    // remote transfers keep progressing during a large local import.
    for (index, (source, relative)) in paths.iter().enumerate() {
        let (destination, size, digest, marker, mut output) = {
            let _lock = artifact_lock(root)?;
            let transfer = load(store, &id, project, owner, false)?;
            let dir = transfer_dir(root, &id)?;
            let (destination, size, digest, marker) =
                file_info(&dir, &transfer, Some(index as u32))?;
            let output = open(&destination, true, true)?;
            ensure!(
                output.metadata()?.len() == 0,
                "Import destination already exists"
            );
            let marker = marker.context("Missing artifact verification path")?;
            (destination, size, digest, marker, output)
        };
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
        let stamp = file_stamp(&output.metadata()?);
        let _lock = artifact_lock(root)?;
        load(store, &id, project, owner, false)?;
        ensure!(
            file_stamp(&open(&destination, false, false)?.metadata()?) == stamp,
            "Imported artifact file {relative} changed while it was copied"
        );
        vault::write_new_private(&marker, digest.as_bytes())?;
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
        assert!(error.downcast_ref::<ArtifactLimitExceeded>().is_some());
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
        let mut accounting = ArtifactAccounting::default();
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
    fn raw_chunks_resume_after_restart_and_keep_transfer_authority_and_bounds() {
        let (dir, store, manifest) = setup();
        let bytes = (0..RAW_ARTIFACT_CHUNK_BYTES + 10003)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let manifest = variant(&manifest, "project", &bytes);
        let id = uuid::Uuid::new_v4().to_string();
        begin(
            &store,
            dir.path(),
            &id,
            "controller",
            &manifest.descriptor().unwrap(),
        )
        .unwrap();
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                None,
                0,
                &manifest.canonical_bytes().unwrap()
            )
            .unwrap()
            .manifest_ready
        );
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "intruder",
                Some(0),
                0,
                &bytes[..16]
            )
            .is_err()
        );
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "other",
                &id,
                "controller",
                Some(0),
                0,
                &bytes[..16]
            )
            .is_err()
        );
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &bytes[..RAW_ARTIFACT_CHUNK_BYTES + 1]
            )
            .is_err()
        );
        assert!(
            chunk(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &URL_SAFE_NO_PAD.encode(&bytes[..PROJECT_ARTIFACT_CHUNK_BYTES + 1])
            )
            .is_err()
        );
        let first = chunk_bytes(
            &store,
            dir.path(),
            "project",
            &id,
            "controller",
            Some(0),
            0,
            &bytes[..RAW_ARTIFACT_CHUNK_BYTES],
        )
        .unwrap();
        assert_eq!(first.offset, RAW_ARTIFACT_CHUNK_BYTES as u64);
        assert!(!first.complete);
        assert_eq!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &bytes[..RAW_ARTIFACT_CHUNK_BYTES]
            )
            .unwrap()
            .offset,
            first.offset
        );
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                first.offset + 1,
                &[1]
            )
            .is_err()
        );
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                first.offset - 1,
                &[1, 2]
            )
            .is_err()
        );
        drop(store);
        let store = StateStore::open(&dir.path().join("management.sqlite")).unwrap();
        assert_eq!(
            status(&store, dir.path(), "project", &id, "controller", Some(0))
                .unwrap()
                .offset,
            first.offset
        );
        let finished = chunk_bytes(
            &store,
            dir.path(),
            "project",
            &id,
            "controller",
            Some(0),
            first.offset,
            &bytes[RAW_ARTIFACT_CHUNK_BYTES..],
        )
        .unwrap();
        assert!(finished.complete);
        let committed = commit(&store, dir.path(), "project", &id, "controller").unwrap();
        assert_eq!(
            std::fs::read(
                PathBuf::from(committed.project_path.unwrap()).join("apps/project/manifest.app")
            )
            .unwrap(),
            bytes
        );
    }

    #[test]
    fn raw_chunks_reset_corrupt_files_before_accepting_a_verified_retry() {
        let (dir, store, manifest) = setup();
        let bytes = vec![42; 64 * 1024];
        let manifest = variant(&manifest, "project", &bytes);
        let id = start(&store, dir.path(), &manifest);
        let mut corrupted = bytes.clone();
        corrupted[bytes.len() - 1] ^= 1;
        let error = chunk_bytes(
            &store,
            dir.path(),
            "project",
            &id,
            "controller",
            Some(0),
            0,
            &corrupted,
        )
        .unwrap_err();
        assert!(error.to_string().contains("SHA256 differs"));
        let reset = status(&store, dir.path(), "project", &id, "controller", Some(0)).unwrap();
        assert_eq!(reset.offset, 0);
        assert!(!reset.complete);
        assert!(reset.manifest_ready);
        assert!(
            chunk_bytes(
                &store,
                dir.path(),
                "project",
                &id,
                "controller",
                Some(0),
                0,
                &bytes
            )
            .unwrap()
            .complete
        );
        assert!(commit(&store, dir.path(), "project", &id, "controller").is_ok());
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
        let error = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "controller",
            &m.descriptor().unwrap(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("uploads in progress"),
            "{error:#}"
        );
        assert!(error.downcast_ref::<ArtifactLimitExceeded>().is_some());
        // One principal's abandoned uploads never block another principal.
        let other = begin(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "other-controller",
            &m.descriptor().unwrap(),
        )
        .unwrap();
        assert!(abort(&store, dir.path(), "project", &id, "other-controller").is_err());
        assert!(abort_as_device_owner(&store, dir.path(), "other", &id).is_err());
        let aborted = abort_as_device_owner(&store, dir.path(), "project", &id).unwrap();
        assert_eq!(aborted.state, ArtifactTransferState::Aborted);
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
        abort(
            &store,
            dir.path(),
            "project",
            &other.transfer_id,
            "other-controller",
        )
        .unwrap();
    }

    #[test]
    fn grantees_filling_device_staging_never_lock_out_the_device_owner() {
        let (dir, store, m) = setup();
        let descriptor = m.descriptor().unwrap();
        let begin_as = |principal: &str| {
            begin(
                &store,
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                principal,
                &descriptor,
            )
        };
        for grantee in 0..MAX_RECEIVING / MAX_RECEIVING_PER_PRINCIPAL {
            for _ in 0..MAX_RECEIVING_PER_PRINCIPAL {
                begin_as(&format!("grantee-{grantee}:grant")).unwrap();
            }
        }
        let error = begin_as("late-grantee:grant").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Device artifact staging is full"),
            "{error:#}"
        );
        assert!(error.downcast_ref::<ArtifactLimitExceeded>().is_some());
        let owned = begin_as_device_owner(
            &store,
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "owner-user:owner",
            &descriptor,
        )
        .unwrap();
        assert_eq!(owned.state, ArtifactTransferState::Receiving);
    }

    #[test]
    fn verified_final_chunk_retry_is_acknowledged_without_rewriting_its_marker() {
        let (dir, store, m) = setup();
        let id = start(&store, dir.path(), &m);
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
        assert!(send(0, b"hello").unwrap().complete);
        let marker = transfer_dir(dir.path(), &id).unwrap().join("verified/0");
        let before = std::fs::metadata(&marker).unwrap().modified().unwrap();
        assert!(send(3, b"lo").unwrap().complete);
        assert_eq!(
            std::fs::metadata(&marker).unwrap().modified().unwrap(),
            before
        );
        assert!(commit(&store, dir.path(), "project", &id, "controller").is_ok());
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
            assert!(metadata_assets(&metadata, &pin, true).is_err(), "{name}");
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

    fn digest(m: &ProjectArtifactManifest) -> String {
        m.descriptor().unwrap().manifest_sha256
    }

    fn retained(used: &StorageUse) -> Vec<String> {
        let (_, revisions) = used.project.as_ref().unwrap();
        revisions.iter().map(|(digest, _)| digest.clone()).collect()
    }

    /// Two committed revisions of `project`, one of `other`, and an upload of `project`
    /// that is still in flight.
    fn stocked() -> (
        tempfile::TempDir,
        StateStore,
        ProjectArtifactManifest,
        [String; 2],
    ) {
        let (dir, store, m) = setup();
        let second = variant(&m, "project", b"next!");
        upload(&store, dir.path(), &m, b"hello");
        upload(&store, dir.path(), &second, b"next!");
        upload(
            &store,
            dir.path(),
            &variant(&m, "other", b"hello"),
            b"hello",
        );
        start(&store, dir.path(), &variant(&m, "project", b"third"));
        let mut committed = [digest(&m), digest(&second)];
        committed.sort();
        (dir, store, m, committed)
    }

    #[test]
    fn usage_is_what_the_next_upload_is_admitted_against() {
        let (dir, store, m, committed) = stocked();
        let root = dir.path();
        let used = usage(&store, root, Some("project")).unwrap();
        assert_eq!(retained(&used), committed);
        let (project, revisions) = used.project.as_ref().unwrap();
        let directory = root.join("projects/project/revisions");
        for (revision, bytes) in revisions {
            assert_eq!(retained_bytes(&directory, revision).unwrap(), Some(*bytes));
            assert!(*bytes > 4096);
        }
        assert_eq!(project["revisions"], json!({"used":3,"max":128}));
        assert_eq!(used.device["revisions"], json!({"used":4,"max":1024}));
        assert_eq!(used.device["bytes"]["max"], 64 * 1024_u64.pow(3));
        assert!(used.device["entries"]["used"].as_u64() > project["entries"]["used"].as_u64());

        let next = variant(&m, "project", b"fourth").descriptor().unwrap();
        let charged =
            project["bytes"]["used"].as_u64().unwrap() + ArtifactUsage::reservation(&next).bytes;
        let attempt = |limit: u64| {
            configure_budget(root, &format!("FLOW_LIKE_PROJECT_ARTIFACT_BYTES={limit}\n"));
            let id = uuid::Uuid::new_v4().to_string();
            begin(&store, root, &id, "controller", &next)
        };
        let refused = attempt(charged - 1).unwrap_err();
        assert!(refused.downcast_ref::<ArtifactLimitExceeded>().is_some());
        attempt(charged).unwrap();
    }

    #[test]
    fn usage_drops_transfers_past_their_lifetime_and_creates_nothing() {
        let (dir, store, _, committed) = stocked();
        let root = dir.path();
        store
            .connection
            .execute("UPDATE project_artifact_transfers SET expires_at=0", [])
            .unwrap();
        let settled = usage(&store, root, Some("project")).unwrap();
        assert_eq!(retained(&settled), committed);
        assert_eq!(settled.project.unwrap().0["revisions"]["used"], 2);
        let tracked: u64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM project_artifact_transfers",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tracked, 0);

        let device = usage(&store, root, None).unwrap();
        assert!(device.project.is_none());
        assert_eq!(device.device["revisions"]["used"], 3);
        let unknown = usage(&store, root, Some("unknown")).unwrap();
        let (budget, revisions) = unknown.project.unwrap();
        assert_eq!(budget["bytes"]["used"], 0);
        assert!(revisions.is_empty() && !root.join("projects/unknown").exists());
        assert!(usage(&store, root, Some("../project")).is_err());
    }

    fn refusal(error: &anyhow::Error) -> (&'static str, String) {
        match error.downcast_ref::<PruneRefused>() {
            Some(PruneRefused::Busy(message)) => ("busy", message.clone()),
            Some(PruneRefused::InUse(message)) => ("in use", message.clone()),
            None => ("other", format!("{error:#}")),
        }
    }

    /// Three revisions of `project`: placement `api` runs from `kept`, an update in progress
    /// may switch it to `staged`, and nothing uses `unused`.
    struct Revisions {
        dir: tempfile::TempDir,
        store: StateStore,
        m: ProjectArtifactManifest,
        kept: String,
        unused: String,
        staged: String,
    }

    impl Revisions {
        fn new() -> Self {
            let (dir, mut store, m) = setup();
            let root = dir.path();
            let (next, third) = (
                variant(&m, "project", b"next!"),
                variant(&m, "project", b"third"),
            );
            let kept = upload(&store, root, &m, b"hello").project_path;
            upload(&store, root, &next, b"next!");
            let staged = upload(&store, root, &third, b"third").project_path;
            let config = |path: &Option<String>| json!({"id":"api","project_id":"project","deployment_id":"deployment","revision":"release-1","source":"offline","project_path":path,"events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}],"variables":{"public-listen-port":8080}});
            store
                .upsert_placement("api", &config(&kept), crate::state::DesiredState::Running)
                .unwrap();
            store.connection.execute(
                "INSERT INTO placement_rollouts(rollout_id,placement_id,project_id,previous_config_json,candidate_config_json,base_revision,base_intent,previous_replicas,candidate_replicas,state,stabilization_seconds,deadline_seconds,created_at,updated_at) VALUES('rollout','api','project',?1,?2,1,1,1,1,'staged',30,600,1,1)",
                params![config(&kept).to_string(), config(&staged).to_string()],
            ).unwrap();
            Self {
                kept: digest(&m),
                unused: digest(&next),
                staged: digest(&third),
                dir,
                store,
                m,
            }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn directory(&self) -> PathBuf {
            self.root().join("projects/project/revisions")
        }

        /// Whether a revision still has its directory and its receipt.
        fn exists(&self, revision: &str) -> (bool, bool) {
            let receipt = format!("{revision}.manifest.json");
            (
                self.directory().join(revision).exists(),
                self.directory().join(receipt).exists(),
            )
        }

        fn remove(&self, revisions: &[&String]) -> Result<(Vec<String>, u64)> {
            let listed: Vec<String> = revisions.iter().map(|digest| (*digest).clone()).collect();
            let prune = Prune::prepare(self.root(), "project", &listed)?;
            let removed = prune.remove(&self.store, unix_time()?);
            prune.discard();
            removed
        }

        fn refused(&self, revisions: &[&String]) -> (&'static str, String) {
            refusal(&self.remove(revisions).unwrap_err())
        }
    }

    #[test]
    fn revisions_in_use_stay_on_the_device_with_everything_listed_next_to_them() {
        let revisions = Revisions::new();
        let (kept, unused, staged) = (&revisions.kept, &revisions.unused, &revisions.staged);
        let all = revisions.remove(&[]).unwrap_err();
        assert!(
            all.downcast_ref::<flow_like_device_protocol::ProtocolError>()
                .is_some()
        );
        let (cause, message) = revisions.refused(&[unused, kept]);
        assert_eq!(cause, "in use", "{message}");
        assert!(message.contains(kept.as_str()) && message.contains("placement api"));
        let (cause, message) = revisions.refused(&[staged]);
        assert_eq!(cause, "in use", "{message}");
        assert!(message.contains("an update in progress"));
        let uses = revision_uses(
            &revisions.store,
            revisions.root(),
            "project",
            &[(kept.clone(), 1), (staged.clone(), 2), (unused.clone(), 3)],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&uses).unwrap(),
            json!([
                {"revision":kept,"bytes":1,"referenced_by":["api"],"rollout":true},
                {"revision":staged,"bytes":2,"referenced_by":[],"rollout":true},
                {"revision":unused,"bytes":3,"referenced_by":[],"rollout":false},
            ])
        );
        assert!(Prune::prepare(revisions.root(), "Project", std::slice::from_ref(unused)).is_err());
        for revision in [kept, unused, staged] {
            assert_eq!(revisions.exists(revision), (true, true));
        }
    }

    #[test]
    fn nothing_is_removed_while_an_upload_or_a_restart_is_under_way() {
        let revisions = Revisions::new();
        let (root, store) = (revisions.root(), &revisions.store);
        let unused = &revisions.unused;
        let flying = start(store, root, &variant(&revisions.m, "project", b"fourth"));
        assert_eq!(revisions.refused(&[unused]).0, "busy");
        abort(store, root, "project", &flying, "controller").unwrap();
        store.connection.execute(
            "INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state,applied_revision,process_id) VALUES('api',0,7,7,'stopping',7,4242)",
            [],
        ).unwrap();
        assert_eq!(revisions.refused(&[unused]).0, "busy");
        assert_eq!(revisions.exists(unused), (true, true));
        store
            .connection
            .execute("UPDATE placement_replicas SET process_id=NULL", [])
            .unwrap();
        assert_eq!(
            revisions.remove(&[unused]).unwrap().0,
            std::slice::from_ref(unused)
        );
    }

    #[test]
    fn listed_unused_revisions_leave_the_project_and_can_be_uploaded_again() {
        let revisions = Revisions::new();
        let (root, store) = (revisions.root(), &revisions.store);
        let (kept, unused, staged) = (&revisions.kept, &revisions.unused, &revisions.staged);
        let bytes = retained_bytes(&revisions.directory(), unused)
            .unwrap()
            .unwrap();
        let left = directory(&root.join("projects/project").join(PRUNED)).unwrap();
        directory(&left.join(staged)).unwrap();
        let unknown = artifact_sha256(b"never uploaded");
        assert_eq!(
            revisions.remove(&[unused, &unknown]).unwrap(),
            (vec![unused.clone()], bytes)
        );
        assert_eq!(revisions.exists(unused), (false, false));
        assert!(!root.join("projects/project").join(PRUNED).exists());
        assert!(managed_revision(root, "project", unused).is_err());
        for revision in [kept, staged] {
            assert!(managed_revision(root, "project", revision).is_ok());
        }
        assert_eq!(revisions.remove(&[unused]).unwrap(), (Vec::new(), 0));
        let mut left = [kept.clone(), staged.clone()];
        left.sort();
        assert_eq!(
            retained(&usage(store, root, Some("project")).unwrap()),
            left
        );
        let again = upload(
            store,
            root,
            &variant(&revisions.m, "project", b"next!"),
            b"next!",
        );
        assert_eq!(again.state, ArtifactTransferState::Committed);
        assert!(managed_revision(root, "project", unused).is_ok());
    }

    #[test]
    fn a_receipt_left_by_an_interrupted_removal_is_listed_and_removable() {
        let revisions = Revisions::new();
        let (root, store) = (revisions.root(), &revisions.store);
        let staged = &revisions.staged;
        store
            .connection
            .execute("UPDATE placement_rollouts SET state='healthy'", [])
            .unwrap();
        std::fs::remove_dir_all(revisions.directory().join(staged)).unwrap();
        let receipt = retained_bytes(&revisions.directory(), staged)
            .unwrap()
            .unwrap();
        let used = usage(store, root, Some("project")).unwrap();
        assert!(retained(&used).contains(staged));
        assert_eq!(
            revisions.remove(&[staged]).unwrap(),
            (vec![staged.clone()], receipt)
        );
        assert_eq!(revisions.exists(staged), (false, false));
        begin(
            store,
            root,
            &uuid::Uuid::new_v4().to_string(),
            "controller",
            &variant(&revisions.m, "project", b"fifth")
                .descriptor()
                .unwrap(),
        )
        .unwrap();
    }

    mod model_store {
        use super::*;
        use crate::models::{
            acquire::{AcquisitionConfig, AcquisitionManager},
            db::{AssetOwner, OwnerKind},
            fetch::{AddressPolicy, Fetcher},
            store::{ModelStore, ModelStoreConfig},
        };
        use flow_like_device_protocol::DigestAlgorithm;

        /// Refused by the production address policy at once, so the job fails without network.
        const UNREACHABLE: &str = "https://127.0.0.1:9/model.gguf";

        fn weights(bytes: &[u8]) -> ModelAssetDescriptor {
            ModelAssetDescriptor {
                digest: ModelAssetDigest {
                    algorithm: DigestAlgorithm::Blake3,
                    hex: blake3::hash(bytes).to_hex().to_string(),
                },
                size: bytes.len() as u64,
                file_name: "model.gguf".into(),
                sources: vec![UNREACHABLE.into()],
            }
        }

        /// Metadata v2 whose weights are a model-store asset and whose tokenizer, when given,
        /// is an artifact file.
        fn metadata(
            bit_id: &str,
            asset: &ModelAssetDescriptor,
            tokenizer: Option<&[u8]>,
        ) -> Vec<u8> {
            let mut value = json!({
                "version": 2,
                "bit": {"id": bit_id, "hash": "weights-hash", "file_name": "model.gguf",
                        "size": asset.size},
                "dependencies": [],
                "assets": [{"bit_id": bit_id, "descriptor": asset}],
            });
            if let Some(tokenizer) = tokenizer {
                value["dependencies"] = json!([{"id": "tokenizer", "hash": "tokenizer-hash",
                    "file_name": "tokenizer.json", "size": tokenizer.len()}]);
                value["artifacts"] = json!([{"path": "bits/tokenizer-hash/tokenizer.json",
                    "size": tokenizer.len(), "sha256": artifact_sha256(tokenizer)}]);
            }
            serde_json::to_vec(&value).unwrap()
        }

        fn v1_metadata() -> Vec<u8> {
            serde_json::to_vec(&json!({
                "bit": {"id": "model", "hash": "hash", "file_name": "model.bin", "size": 7},
                "dependencies": [],
                "artifacts": [{"path": "bits/hash/model.bin", "size": 7,
                               "sha256": artifact_sha256(b"weights")}],
            }))
            .unwrap()
        }

        fn pin(bit_id: &str, metadata: &[u8]) -> ProjectBitPin {
            ProjectBitPin {
                bit_id: bit_id.into(),
                metadata_sha256: artifact_sha256(metadata),
            }
        }

        /// The project's manifest, its Bit metadata and further files, sorted by path.
        fn files_of(
            bits: &[(ProjectBitPin, Vec<u8>)],
            extra: &[(&str, &[u8])],
        ) -> Vec<(String, Vec<u8>)> {
            let mut files = vec![("apps/project/manifest.app".to_owned(), b"hello".to_vec())];
            files.extend(bits.iter().map(|(pin, metadata)| {
                let path = format!("bits/metadata/{}.json", pin.bit_id);
                (path, metadata.clone())
            }));
            files.extend(
                extra
                    .iter()
                    .map(|(path, bytes)| (path.to_string(), bytes.to_vec())),
            );
            files.sort();
            files
        }

        fn manifest_of(
            files: &[(String, Vec<u8>)],
            bits: &[(ProjectBitPin, Vec<u8>)],
        ) -> ProjectArtifactManifest {
            let file = |(path, bytes): &(String, Vec<u8>)| ProjectArtifactFile {
                path: path.clone(),
                size: bytes.len() as u64,
                sha256: artifact_sha256(bytes),
            };
            ProjectArtifactManifest {
                version: 1,
                source: flow_like_device_protocol::ProjectArtifactSource::Offline,
                project_id: "project".into(),
                bit_pins: bits.iter().map(|(pin, _)| pin.clone()).collect(),
                package_pins: vec![],
                files: files.iter().map(file).collect(),
            }
        }

        /// Uploads the project with these Bit metadata and further files, then commits it; a
        /// refused commit is aborted.
        fn commit_bits(
            store: &StateStore,
            root: &Path,
            bits: &[(ProjectBitPin, Vec<u8>)],
            extra: &[(&str, &[u8])],
            model_store: bool,
        ) -> Result<ArtifactTransferStatus> {
            let files = files_of(bits, extra);
            let id = start(store, root, &manifest_of(&files, bits));
            for (index, (_, bytes)) in files.iter().enumerate() {
                let data = URL_SAFE_NO_PAD.encode(bytes);
                let index = Some(index as u32);
                chunk(store, root, "project", &id, "controller", index, 0, &data).unwrap();
            }
            let committed = commit_with(store, root, "project", &id, "controller", model_store);
            if committed.is_err() {
                abort(store, root, "project", &id, "controller").unwrap();
            }
            committed
        }

        fn acquisition(root: &Path) -> AcquisitionManager {
            let store = ModelStore::open(root, ModelStoreConfig::default()).unwrap();
            let fetcher = Fetcher::new(AddressPolicy::global_only()).unwrap();
            AcquisitionManager::start(Arc::new(store), fetcher, AcquisitionConfig::default())
                .unwrap()
        }

        #[test]
        fn only_model_store_agents_take_weights_that_are_not_in_the_artifact() {
            let (dir, store, _) = setup();
            let tokenizer: &[u8] = b"{}";
            let bytes = metadata("model", &weights(b"weights"), Some(tokenizer));
            let bits = [(pin("model", &bytes), bytes)];
            let extra = [("bits/tokenizer-hash/tokenizer.json", tokenizer)];
            let refused = commit_bits(&store, dir.path(), &bits, &extra, false).unwrap_err();
            assert!(
                format!("{refused:#}").contains("model store"),
                "{refused:#}"
            );
            let unlisted = commit_bits(&store, dir.path(), &bits, &[], true).unwrap_err();
            assert!(
                format!("{unlisted:#}").contains("Selected Bit asset is missing"),
                "{unlisted:#}"
            );
            let stray = [
                extra[0],
                ("bits/weights-hash/model.gguf", b"weights".as_slice()),
            ];
            assert!(commit_bits(&store, dir.path(), &bits, &stray, true).is_err());
            let committed = commit_bits(&store, dir.path(), &bits, &extra, true).unwrap();
            assert_eq!(committed.state, ArtifactTransferState::Committed);
            let revision = PathBuf::from(committed.project_path.unwrap());
            assert!(!revision.join("bits/weights-hash").exists());
            let copied = std::fs::read(revision.join("bits/tokenizer-hash/tokenizer.json"));
            assert_eq!(copied.unwrap(), tokenizer);
        }

        #[test]
        fn v1_metadata_commits_whether_or_not_the_agent_has_a_model_store() {
            let (dir, store, _) = setup();
            let bytes = v1_metadata();
            let bits = [(pin("model", &bytes), bytes)];
            let extra = [("bits/hash/model.bin", b"weights".as_slice())];
            for model_store in [false, true] {
                let committed = commit_bits(&store, dir.path(), &bits, &extra, model_store);
                assert_eq!(committed.unwrap().state, ArtifactTransferState::Committed);
            }
        }

        #[tokio::test]
        async fn ensure_starts_or_joins_one_job_per_asset_of_committed_pins() {
            let (dir, store, _) = setup();
            let asset = weights(b"weights");
            let (first, second) = (
                metadata("model", &asset, None),
                metadata("copy", &asset, None),
            );
            let pins = vec![pin("model", &first), pin("copy", &second)];
            let bits = [(pins[0].clone(), first), (pins[1].clone(), second)];
            commit_bits(&store, dir.path(), &bits, &[], true).unwrap();
            let acquisition = acquisition(dir.path());
            let installing = acquisition.ensure(&asset, Some("install")).unwrap();
            assert!(installing.job_id.is_some());
            let root = dir.path();

            let summary =
                ensure_project_models(root, &acquisition, "project", &pins, Some("deploy"))
                    .await
                    .unwrap();
            assert_eq!((summary.total, summary.present), (1, 0));
            assert_eq!(summary.pending.len(), 1);
            assert_eq!(summary.pending[0].job_id, installing.job_id);
            let lease = AssetOwner::new(OwnerKind::Deploy, "project").unwrap();
            let held = acquisition.store().refs(&asset.digest).unwrap();
            assert_eq!(held, vec![lease], "the deploy holds what it fetches");
            let again = ensure_project_models(root, &acquisition, "project", &pins[..1], None)
                .await
                .unwrap();
            assert_eq!(again.pending[0].job_id, installing.job_id);
            assert_eq!(acquisition.jobs().len(), 1);
            assert_eq!(acquisition.jobs()[0].asset.sources, vec![UNREACHABLE]);

            acquisition.begin_push(&asset.digest, true).await.unwrap();
            let pushed = acquisition.push_chunk(&asset.digest, 0, b"weights").await;
            assert_eq!(pushed.unwrap(), ModelAssetState::Present);
            let present = ensure_project_models(root, &acquisition, "project", &pins, None)
                .await
                .unwrap();
            assert_eq!((present.total, present.present), (1, 1));
            assert!(present.pending.is_empty());

            let uncommitted = [pin("model", b"metadata of another revision")];
            let refused = ensure_project_models(root, &acquisition, "project", &uncommitted, None);
            assert!(refused.await.is_err());
            let elsewhere = ensure_project_models(root, &acquisition, "other", &pins, None);
            assert!(elsewhere.await.is_err());
        }

        #[tokio::test]
        async fn ensure_names_no_assets_for_v1_metadata() {
            let (dir, store, _) = setup();
            let bytes = v1_metadata();
            let pins = vec![pin("model", &bytes)];
            let extra = [("bits/hash/model.bin", b"weights".as_slice())];
            let bits = [(pins[0].clone(), bytes)];
            commit_bits(&store, dir.path(), &bits, &extra, false).unwrap();
            let acquisition = acquisition(dir.path());
            let summary = ensure_project_models(dir.path(), &acquisition, "project", &pins, None)
                .await
                .unwrap();
            assert_eq!((summary.total, summary.present), (0, 0));
        }
    }
}
