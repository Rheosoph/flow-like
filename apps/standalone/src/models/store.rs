use super::db::{AssetOwner, AssetRecord, ModelsDb, OwnerKind, digest_from_key};
use crate::enrollment::unix_time;
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{ModelAssetDescriptor, ModelAssetDigest};
use std::{
    collections::HashMap,
    fs::{File, Metadata, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

pub const DEFAULT_GC_GRACE: Duration = Duration::from_secs(24 * 60 * 60);
const PARTIAL_SUFFIX: &str = ".part";
/// The `disk_low` recommendation fires below the same share of the volume.
#[cfg(not(test))]
const KEEP_FREE_PERCENT: u8 = 10;
/// Lib tests write a few bytes and must pass on a nearly full developer volume.
#[cfg(test)]
const KEEP_FREE_PERCENT: u8 = 0;

#[derive(Clone, Debug)]
pub struct ModelStoreConfig {
    /// Caps stored, reserved and staged bytes. `None` admits the free space on the volume
    /// down to `keep_free_percent` of the volume's size.
    pub max_bytes: Option<u64>,
    pub gc_grace: Duration,
    /// Without `max_bytes`, a volume with less free space than this share of its size is
    /// full for the store: nothing more is admitted, and collection skips the grace period.
    pub keep_free_percent: u8,
}

impl Default for ModelStoreConfig {
    fn default() -> Self {
        Self {
            max_bytes: None,
            gc_grace: DEFAULT_GC_GRACE,
            keep_free_percent: KEEP_FREE_PERCENT,
        }
    }
}

/// Retrying cannot succeed until space is freed or the budget is raised.
#[derive(Debug)]
pub struct DiskBudgetExceeded {
    pub digest: String,
    pub needed: u64,
    pub available: u64,
}

impl std::fmt::Display for DiskBudgetExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "Model asset {} needs {} bytes, but the model store budget admits {}",
            self.digest, self.needed, self.available
        )
    }
}

impl std::error::Error for DiskBudgetExceeded {}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed: Vec<ModelAssetDigest>,
    pub freed_bytes: u64,
    pub partials_removed: usize,
}

type Reserved = HashMap<ModelAssetDigest, u64>;

/// Bytes counted against the budget while an asset is acquired; released on drop.
pub struct Reservation {
    reserved: Arc<Mutex<Reserved>>,
    digest: ModelAssetDigest,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        lock!(self.reserved).remove(&self.digest);
    }
}

#[derive(Clone, Copy, Debug)]
struct Volume {
    available: u64,
    total: u64,
}

impl Volume {
    /// The free bytes a default budget never hands out: a share of the volume's size, so
    /// installs one after another cannot creep toward a full volume.
    fn floor(self, keep_free_percent: u8) -> u64 {
        let floor = u128::from(self.total) * u128::from(keep_free_percent) / 100;
        u64::try_from(floor).unwrap_or(u64::MAX)
    }
}

/// What the budget is measured against. `pending` is what reserved assets still have to
/// write, and `staging` what `incoming/` holds that no reservation counts, such as the bytes
/// of a failed download or of a push that a restart cut off.
#[derive(Clone, Copy, Debug)]
struct Usage {
    volume: Volume,
    stored: u64,
    reserved: u64,
    pending: u64,
    staging: u64,
}

impl Usage {
    /// After an asset is collected: its record leaves the stored bytes, and only a blob that
    /// is still on disk frees room on the volume.
    fn collecting(self, size: u64, on_disk: bool) -> Self {
        let freed = if on_disk { size } else { 0 };
        Self {
            volume: Volume {
                available: self.volume.available.saturating_add(freed),
                ..self.volume
            },
            stored: self.stored.saturating_sub(size),
            ..self
        }
    }
}

/// Room for the bytes still missing on the volume, and for whole assets under `max_bytes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Room {
    volume: u64,
    budget: u64,
}

impl Room {
    fn admits(self, size: u64, staged: u64) -> bool {
        size.saturating_sub(staged) <= self.volume && size <= self.budget
    }

    fn available(self) -> u64 {
        self.volume.min(self.budget)
    }
}

/// The default budget sees staged bytes in the volume's free space; a cap counts them.
fn room(config: &ModelStoreConfig, usage: Usage) -> Room {
    match config.max_bytes {
        Some(max) => Room {
            volume: usage.volume.available.saturating_sub(usage.pending),
            budget: max.saturating_sub(
                usage
                    .stored
                    .saturating_add(usage.reserved)
                    .saturating_add(usage.staging),
            ),
        },
        None => Room {
            volume: usage
                .volume
                .available
                .saturating_sub(usage.volume.floor(config.keep_free_percent))
                .saturating_sub(usage.pending),
            budget: u64::MAX,
        },
    }
}

/// Disk pressure: the store at its cap with its staged bytes, or the volume below the floor
/// of the default budget.
fn pressured(config: &ModelStoreConfig, usage: Usage) -> bool {
    let volume = usage.volume;
    match config.max_bytes {
        Some(max) => volume.available == 0 || usage.stored.saturating_add(usage.staging) >= max,
        None => volume.available < volume.floor(config.keep_free_percent),
    }
}

/// How many of the oldest `candidates`, each its size and whether its blob is on disk, must
/// be collected before `size` fits; `None` when collecting all of them would not make room.
fn collect_to_fit(
    config: &ModelStoreConfig,
    mut usage: Usage,
    candidates: &[(u64, bool)],
    size: u64,
    staged: u64,
) -> Option<usize> {
    for (index, &(bytes, on_disk)) in candidates.iter().enumerate() {
        usage = usage.collecting(bytes, on_disk);
        if room(config, usage).admits(size, staged) {
            return Some(index + 1);
        }
    }
    None
}

/// Content-addressed blobs at `<state>/models/blobs/<algo>/<hex>`. A blob appears only by
/// renaming a complete, verified file out of `incoming/` and is never written again.
pub struct ModelStore {
    root: PathBuf,
    config: ModelStoreConfig,
    db: Mutex<ModelsDb>,
    /// Also serializes publishing with garbage collection.
    reserved: Arc<Mutex<Reserved>>,
    /// Woken by every released reference.
    released: Arc<Notify>,
}

impl ModelStore {
    pub fn open(state_dir: &Path, config: ModelStoreConfig) -> Result<Self> {
        let root = state_dir.join("models");
        private_directory(&root)?;
        for directory in ["blobs", "blobs/sha256", "blobs/blake3", "incoming"] {
            private_directory(&root.join(directory))?;
        }
        let db = ModelsDb::open(&root.join("models.sqlite"))?;
        Ok(Self {
            root,
            config,
            db: Mutex::new(db),
            reserved: Arc::default(),
            released: Arc::default(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs `read` on the model database; the lock never outlives the call.
    pub fn with_db<R>(&self, read: impl FnOnce(&ModelsDb) -> R) -> R {
        read(&lock!(self.db))
    }

    fn blob_path(&self, digest: &ModelAssetDigest) -> PathBuf {
        self.root.join("blobs").join(digest.store_key())
    }

    pub fn partial_path(&self, digest: &ModelAssetDigest) -> PathBuf {
        self.root.join("incoming").join(format!(
            "{}-{}{PARTIAL_SUFFIX}",
            digest.algorithm.as_str(),
            digest.hex
        ))
    }

    pub fn contains(&self, digest: &ModelAssetDigest, size: u64) -> Result<bool> {
        digest.validate()?;
        let Some(record) = lock!(self.db).asset(digest)? else {
            return Ok(false);
        };
        Ok(record.size == size && self.blob_intact(digest, size)?)
    }

    /// The blob of a present asset; callers keep it alive with a reference.
    pub fn path_of(&self, digest: &ModelAssetDigest) -> Result<Option<PathBuf>> {
        digest.validate()?;
        let Some(record) = lock!(self.db).asset(digest)? else {
            return Ok(None);
        };
        Ok(self
            .blob_intact(digest, record.size)?
            .then(|| self.blob_path(digest)))
    }

    fn blob_intact(&self, digest: &ModelAssetDigest, size: u64) -> Result<bool> {
        match std::fs::symlink_metadata(self.blob_path(digest)) {
            Ok(metadata) if private_file(&metadata) && metadata.len() == size => Ok(true),
            Ok(_) => {
                tracing::warn!(
                    "Model blob {} is not a private regular file of {size} bytes",
                    digest.store_key()
                );
                Ok(false)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => {
                Err(error).with_context(|| format!("Inspect model blob {}", digest.store_key()))
            }
        }
    }

    /// References may name assets that are not present yet, so a deploy can pin first.
    pub fn add_ref(&self, digest: &ModelAssetDigest, owner: &AssetOwner) -> Result<()> {
        digest.validate()?;
        let now = unix_time()?;
        let db = lock!(self.db);
        db.add_ref(digest, owner, now)?;
        db.touch_asset(digest, now)
    }

    /// The grace period of an unreferenced blob counts from its last release. A release
    /// wakes [`ModelStore::releases`], so a collection under disk pressure frees it soon.
    pub fn remove_ref(&self, digest: &ModelAssetDigest, owner: &AssetOwner) -> Result<bool> {
        let now = unix_time()?;
        let removed = {
            let db = lock!(self.db);
            let removed = db.remove_ref(digest, owner)?;
            db.touch_asset(digest, now)?;
            removed
        };
        if removed {
            self.released.notify_one();
        }
        Ok(removed)
    }

    /// Notified whenever a reference goes.
    pub fn releases(&self) -> Arc<Notify> {
        Arc::clone(&self.released)
    }

    /// Holds an asset for a deploy until its placement references it. Every `Models.Ensure`
    /// renews the lease, the placement's start ends it, and it lapses one grace period
    /// after the last renewal when no placement comes.
    pub fn lease(&self, digest: &ModelAssetDigest, deploy: &str) -> Result<()> {
        digest.validate()?;
        let owner = AssetOwner::new(OwnerKind::Deploy, deploy)?;
        let now = unix_time()?;
        let db = lock!(self.db);
        db.renew_ref(digest, &owner, now)?;
        db.touch_asset(digest, now)
    }

    pub fn end_lease(&self, digest: &ModelAssetDigest, deploy: &str) -> Result<bool> {
        self.remove_ref(digest, &AssetOwner::new(OwnerKind::Deploy, deploy)?)
    }

    fn expire_leases(&self, db: &ModelsDb, now: i64) -> Result<()> {
        let grace = i64::try_from(self.config.gc_grace.as_secs()).unwrap_or(i64::MAX);
        db.expire_refs(OwnerKind::Deploy, now.saturating_sub(grace))?;
        Ok(())
    }

    pub fn refs(&self, digest: &ModelAssetDigest) -> Result<Vec<AssetOwner>> {
        lock!(self.db).refs(digest)
    }

    pub fn touch(&self, digest: &ModelAssetDigest) -> Result<()> {
        lock!(self.db).touch_asset(digest, unix_time()?)
    }

    fn volume(&self) -> Result<Volume> {
        let stats = fs2::statvfs(&self.root)
            .with_context(|| format!("Read free space of {}", self.root.display()))?;
        Ok(Volume {
            available: stats.available_space(),
            total: stats.total_space(),
        })
    }

    /// `reserving` is the asset a reservation is being made for: it counts its own staged bytes.
    fn usage(&self, reserved: &Reserved, reserving: Option<&ModelAssetDigest>) -> Result<Usage> {
        let mut total = 0u64;
        let mut pending = 0u64;
        for (digest, size) in reserved {
            total = total.saturating_add(*size);
            pending = pending.saturating_add(size.saturating_sub(self.partial_len(digest)?));
        }
        let counted =
            |digest: &ModelAssetDigest| reserved.contains_key(digest) || reserving == Some(digest);
        Ok(Usage {
            volume: self.volume()?,
            stored: lock!(self.db).stored_bytes()?,
            reserved: total,
            pending,
            staging: self.staging_bytes(counted)?,
        })
    }

    /// Bytes of the regular files in `incoming/`, except the staging files of `counted` assets.
    fn staging_bytes(&self, counted: impl Fn(&ModelAssetDigest) -> bool) -> Result<u64> {
        let incoming = self.root.join("incoming");
        let entries = std::fs::read_dir(&incoming)
            .with_context(|| format!("List model staging files in {}", incoming.display()))?;
        let mut bytes = 0u64;
        for entry in entries {
            let entry = entry?;
            let digest = entry.file_name().to_str().and_then(partial_digest);
            if !digest.is_some_and(|digest| counted(&digest)) {
                bytes = bytes.saturating_add(regular_file_len(&entry)?);
            }
        }
        Ok(bytes)
    }

    /// Bytes a new asset may still take.
    pub fn available_bytes(&self) -> Result<u64> {
        let reserved = lock!(self.reserved);
        Ok(room(&self.config, self.usage(&reserved, None)?).available())
    }

    fn under_pressure(&self) -> Result<bool> {
        let reserved = lock!(self.reserved);
        Ok(pressured(&self.config, self.usage(&reserved, None)?))
    }

    /// Counts `size` against the budget before a job writes. When the asset fits only once
    /// unreferenced blobs go, the oldest ones that make room are collected at once, grace
    /// period or not; otherwise nothing is deleted and the reservation is refused.
    pub fn reserve(&self, digest: &ModelAssetDigest, size: u64) -> Result<Reservation> {
        let mut reserved = lock!(self.reserved);
        ensure!(
            !reserved.contains_key(digest),
            "Model asset {} already holds a disk reservation",
            digest.store_key()
        );
        let staged = self.partial_len(digest)?;
        let usage = self.usage(&reserved, Some(digest))?;
        if !room(&self.config, usage).admits(size, staged) {
            self.collect_to_admit(&reserved, usage, digest, (size, staged))?;
        }
        reserved.insert(digest.clone(), size);
        Ok(Reservation {
            reserved: Arc::clone(&self.reserved),
            digest: digest.clone(),
        })
    }

    /// Deletes the oldest unreferenced blobs that make room for an asset of `size` with
    /// `staged` bytes, or refuses without deleting any when all of them would not.
    fn collect_to_admit(
        &self,
        reserved: &Reserved,
        usage: Usage,
        digest: &ModelAssetDigest,
        (size, staged): (u64, u64),
    ) -> Result<()> {
        let db = lock!(self.db);
        let candidates = self.collectable(&db, reserved)?;
        let sizes: Vec<_> = candidates
            .iter()
            .map(|(asset, on_disk)| (asset.size, *on_disk))
            .collect();
        let Some(count) = collect_to_fit(&self.config, usage, &sizes, size, staged) else {
            return Err(DiskBudgetExceeded {
                digest: digest.store_key(),
                needed: size.saturating_sub(staged),
                available: room(&self.config, usage).available(),
            }
            .into());
        };
        for (asset, _) in candidates.into_iter().take(count) {
            remove_if_present(&self.blob_path(&asset.digest))?;
            db.delete_asset(&asset.digest)?;
        }
        Ok(())
    }

    /// Unreferenced assets nothing reserves, oldest first, each with whether its blob is
    /// still on disk. Lapsed deploy leases go first.
    fn collectable(&self, db: &ModelsDb, reserved: &Reserved) -> Result<Vec<(AssetRecord, bool)>> {
        self.expire_leases(db, unix_time()?)?;
        let mut candidates = Vec::new();
        for asset in db.unreferenced_assets(i64::MAX)? {
            if !reserved.contains_key(&asset.digest) {
                let on_disk = self.blob_intact(&asset.digest, asset.size)?;
                candidates.push((asset, on_disk));
            }
        }
        Ok(candidates)
    }

    /// The staging file of an asset in `incoming/`, created when missing.
    pub fn open_partial(&self, digest: &ModelAssetDigest) -> Result<File> {
        digest.validate()?;
        open_private_file(&self.partial_path(digest), true)
    }

    pub fn partial_len(&self, digest: &ModelAssetDigest) -> Result<u64> {
        match std::fs::symlink_metadata(self.partial_path(digest)) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(metadata.len()),
            Ok(_) => bail!(
                "Staged model asset {} is not a regular file",
                digest.store_key()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error)
                .with_context(|| format!("Inspect staged model asset {}", digest.store_key())),
        }
    }

    pub fn remove_partial(&self, digest: &ModelAssetDigest) -> Result<()> {
        remove_if_present(&self.partial_path(digest))
    }

    /// Moves a complete staged file into the store. The caller has verified its digest.
    pub fn publish(&self, asset: &ModelAssetDescriptor) -> Result<()> {
        let key = asset.digest.store_key();
        let partial = self.partial_path(&asset.digest);
        let file = open_private_file(&partial, false)?;
        let length = file.metadata()?.len();
        ensure!(
            length == asset.size,
            "Publish model asset {key}: the staged file holds {length} of {} bytes",
            asset.size
        );
        file.sync_all()?;
        self.install(asset, &partial)?;
        if let Err(error) = read_only(&file) {
            tracing::warn!("Model blob {key} stays writable by the agent: {error}");
        }
        Ok(())
    }

    /// Records the asset before renaming its blob into place, so a crash in between
    /// leaves a record without a blob, which reads as absent, and never an untracked blob.
    fn install(&self, asset: &ModelAssetDescriptor, partial: &Path) -> Result<()> {
        let _layout = lock!(self.reserved);
        let now = unix_time()?;
        lock!(self.db).put_asset(&AssetRecord {
            digest: asset.digest.clone(),
            size: asset.size,
            file_name: asset.file_name.clone(),
            present_at: now,
            last_used_at: now,
        })?;
        if let Err(error) = std::fs::rename(partial, self.blob_path(&asset.digest)) {
            lock!(self.db).delete_asset(&asset.digest)?;
            return Err(error)
                .with_context(|| format!("Publish model asset {}", asset.digest.store_key()));
        }
        sync_directory(
            &self
                .root
                .join("blobs")
                .join(asset.digest.algorithm.as_str()),
        )?;
        sync_directory(&self.root.join("incoming"))
    }

    /// Removes unreferenced blobs whose last use is older than the grace period, or every
    /// unreferenced blob under disk pressure, plus staging files without a job and lapsed
    /// deploy leases. Callers add a reference or a lease before they rely on a blob, since
    /// pressure skips the grace period.
    pub fn collect_garbage(&self) -> Result<GcReport> {
        let immediate = self.under_pressure()?;
        self.collect(unix_time()?, immediate)
    }

    fn collect(&self, now: i64, immediate: bool) -> Result<GcReport> {
        let reserved = lock!(self.reserved);
        let grace = if immediate {
            0
        } else {
            i64::try_from(self.config.gc_grace.as_secs()).unwrap_or(i64::MAX)
        };
        let db = lock!(self.db);
        self.expire_leases(&db, now)?;
        let mut report = self.remove_unreferenced(&db, &reserved, now.saturating_sub(grace))?;
        self.forget_missing(&db)?;
        report.partials_removed = self.sweep_incoming(&db)?;
        Ok(report)
    }

    fn remove_unreferenced(
        &self,
        db: &ModelsDb,
        reserved: &Reserved,
        unused_since: i64,
    ) -> Result<GcReport> {
        let mut report = GcReport::default();
        for asset in db.unreferenced_assets(unused_since)? {
            if reserved.contains_key(&asset.digest) {
                continue;
            }
            remove_if_present(&self.blob_path(&asset.digest))?;
            db.delete_asset(&asset.digest)?;
            report.freed_bytes += asset.size;
            report.removed.push(asset.digest);
        }
        Ok(report)
    }

    /// Records whose blob vanished would otherwise count against the budget forever.
    fn forget_missing(&self, db: &ModelsDb) -> Result<()> {
        for asset in db.assets()? {
            if !self.blob_intact(&asset.digest, asset.size)? {
                db.delete_asset(&asset.digest)?;
            }
        }
        Ok(())
    }

    fn sweep_incoming(&self, db: &ModelsDb) -> Result<usize> {
        let mut removed = 0;
        for entry in std::fs::read_dir(self.root.join("incoming"))? {
            let entry = entry?;
            if stray_staging(db, &entry)? {
                remove_if_present(&entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// A staging entry that belongs to no job; directories are left for an operator.
fn stray_staging(db: &ModelsDb, entry: &std::fs::DirEntry) -> Result<bool> {
    if let Some(digest) = entry.file_name().to_str().and_then(partial_digest)
        && db.job(&digest)?.is_some()
    {
        return Ok(false);
    }
    if entry.file_type()?.is_dir() {
        tracing::warn!(
            "Model store staging holds an unexpected directory {}",
            entry.path().display()
        );
        return Ok(false);
    }
    Ok(true)
}

/// The length of a regular file in a listing; 0 for anything else or for a file that went.
fn regular_file_len(entry: &std::fs::DirEntry) -> Result<u64> {
    match entry.metadata() {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Inspect {}", entry.path().display()))
        }
        _ => Ok(0),
    }
}

fn partial_digest(name: &str) -> Option<ModelAssetDigest> {
    let (algorithm, hex) = name.strip_suffix(PARTIAL_SUFFIX)?.split_once('-')?;
    digest_from_key(&format!("{algorithm}/{hex}")).ok()
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Remove {}", path.display())),
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("Sync model store directory {}", path.display()))
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn create_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(error).with_context(|| format!("Create model store directory {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Mirrors the runtime store: created private, opened without following a symlink,
/// owned by the agent, and narrowed to the owner when it is wider.
fn private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        create_directory(path)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)
            .with_context(|| {
                format!(
                    "Open model store directory {} without symlinks",
                    path.display()
                )
            })?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.is_dir() && metadata.uid() == effective_uid(),
            "Model store directory {} must be a directory owned by the agent",
            path.display()
        );
        if metadata.mode() & 0o077 != 0 {
            directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    bail!(
        "Model store directory {} needs Unix file permissions",
        path.display()
    )
}

fn private_file(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.file_type().is_file()
            && metadata.uid() == effective_uid()
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn open_private_file(path: &Path, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Open staged model asset {}", path.display()))?;
    ensure!(
        private_file(&file.metadata()?),
        "Staged model asset {} must be a private regular file with one link",
        path.display()
    );
    Ok(file)
}

fn read_only(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o400))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::db::OwnerKind;
    use flow_like_device_protocol::DigestAlgorithm;
    use std::io::Write;

    fn descriptor(bytes: &[u8]) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Blake3,
                hex: blake3::hash(bytes).to_hex().to_string(),
            },
            size: bytes.len() as u64,
            file_name: "model.gguf".into(),
            sources: Vec::new(),
        }
    }

    /// Stages and publishes `bytes`; setup failures panic.
    fn put(store: &ModelStore, bytes: &[u8]) -> ModelAssetDescriptor {
        let asset = descriptor(bytes);
        let mut staged = store.open_partial(&asset.digest).expect("a staging file");
        staged.write_all(bytes).expect("staged bytes");
        store.publish(&asset).expect("a published asset");
        asset
    }

    fn open(directory: &Path, max_bytes: Option<u64>) -> Result<ModelStore> {
        ModelStore::open(
            directory,
            ModelStoreConfig {
                max_bytes,
                gc_grace: Duration::from_secs(100),
                ..ModelStoreConfig::default()
            },
        )
    }

    /// A store with a 100 s grace period in a fresh state directory.
    fn open_store(max_bytes: Option<u64>) -> (tempfile::TempDir, ModelStore) {
        let directory = tempfile::tempdir().expect("a state directory");
        let store = open(directory.path(), max_bytes).expect("a model store");
        (directory, store)
    }

    #[cfg(unix)]
    fn link(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("a symbolic link");
    }

    #[cfg(unix)]
    #[test]
    fn published_blobs_are_private_and_read_only() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let (_directory, store) = open_store(None);
        let mode = |path: PathBuf| {
            let metadata = std::fs::metadata(path).expect("store metadata");
            metadata.permissions().mode() & 0o777
        };
        for path in ["", "blobs", "blobs/blake3", "incoming"] {
            assert_eq!(mode(store.root().join(path)), 0o700, "{path}");
        }
        let asset = put(&store, b"weights");
        let blob = store.path_of(&asset.digest)?.context("published blob")?;
        assert_eq!(std::fs::read(&blob)?, b"weights");
        assert_eq!(mode(blob), 0o400);
        assert!(store.contains(&asset.digest, 7)?);
        assert!(!store.contains(&asset.digest, 8)?);
        assert_eq!(store.partial_len(&asset.digest)?, 0);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_blobs_and_staging_files_are_refused() -> Result<()> {
        let (directory, store) = open_store(None);
        let asset = put(&store, b"weights");
        let blob = store.path_of(&asset.digest)?.context("published blob")?;
        let elsewhere = directory.path().join("elsewhere");
        std::fs::write(&elsewhere, b"weights").expect("a decoy file");
        std::fs::remove_file(&blob).expect("a removed blob");
        link(&elsewhere, &blob);
        assert!(!store.contains(&asset.digest, 7)?);
        assert_eq!(store.path_of(&asset.digest)?, None);
        let other = descriptor(b"other");
        link(&elsewhere, &store.partial_path(&other.digest));
        assert!(store.open_partial(&other.digest).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_store_directory_is_refused() {
        let (directory, store) = open_store(None);
        drop(store);
        let incoming = directory.path().join("models/incoming");
        std::fs::remove_dir_all(&incoming).expect("a removed staging directory");
        link(directory.path(), &incoming);
        assert!(open(directory.path(), None).is_err());
    }

    #[test]
    fn unreferenced_blobs_go_after_the_grace_period() -> Result<()> {
        let (_directory, store) = open_store(None);
        let kept = put(&store, b"kept");
        let dropped = put(&store, b"dropped");
        let owner = AssetOwner::new(OwnerKind::Placement, "placement-1")?;
        store.add_ref(&kept.digest, &owner)?;
        let published = unix_time()?;
        assert_eq!(store.collect(published + 50, false)?, GcReport::default());
        let report = store.collect(published + 101, false)?;
        assert_eq!(report.removed, vec![dropped.digest.clone()]);
        assert_eq!(report.freed_bytes, 7);
        assert!(!store.contains(&dropped.digest, 7)?);
        assert!(store.contains(&kept.digest, 4)?);
        Ok(())
    }

    #[test]
    fn releasing_the_last_reference_restarts_the_grace_period() -> Result<()> {
        let (_directory, store) = open_store(None);
        let kept = put(&store, b"kept");
        let owner = AssetOwner::new(OwnerKind::Placement, "placement-1")?;
        store.add_ref(&kept.digest, &owner)?;
        let released = unix_time()?;
        assert!(store.remove_ref(&kept.digest, &owner)?);
        assert!(store.collect(released + 95, false)?.removed.is_empty());
        let removed = store.collect(released + 105, false)?.removed;
        assert_eq!(removed, vec![kept.digest.clone()]);
        assert_eq!(store.with_db(ModelsDb::stored_bytes)?, 0);
        Ok(())
    }

    #[test]
    fn disk_pressure_skips_the_grace_period() -> Result<()> {
        let (_directory, store) = open_store(Some(8));
        let referenced = put(&store, b"ref");
        let unreferenced = put(&store, b"unref");
        store.add_ref(
            &referenced.digest,
            &AssetOwner::new(OwnerKind::User, "pin")?,
        )?;
        let report = store.collect_garbage()?;
        assert_eq!(report.removed, vec![unreferenced.digest]);
        assert!(store.contains(&referenced.digest, 3)?);
        Ok(())
    }

    #[test]
    fn budget_refuses_what_does_not_fit_and_releases_on_drop() -> Result<()> {
        let (_directory, store) = open_store(Some(100));
        let first = descriptor(b"first");
        let second = descriptor(b"second");
        let held = store.reserve(&first.digest, 60)?;
        let Err(refused) = store.reserve(&second.digest, 50) else {
            bail!("A reservation beyond the budget was admitted");
        };
        let budget = refused
            .downcast_ref::<DiskBudgetExceeded>()
            .context("typed budget refusal")?;
        assert_eq!((budget.needed, budget.available), (50, 40));
        assert!(store.reserve(&first.digest, 1).is_err());
        drop(held);
        drop(store.reserve(&second.digest, 50)?);
        assert_eq!(store.available_bytes()?, 100);
        Ok(())
    }

    #[test]
    fn reserving_collects_unreferenced_blobs_only_when_that_makes_room() -> Result<()> {
        let (_directory, store) = open_store(Some(10));
        let idle = put(&store, b"idle123");
        let wanted = descriptor(b"wanted");
        assert!(store.reserve(&wanted.digest, 20).is_err());
        assert!(store.contains(&idle.digest, 7)?);
        drop(store.reserve(&wanted.digest, 5)?);
        assert!(!store.contains(&idle.digest, 7)?);
        Ok(())
    }

    #[test]
    fn reserving_collects_only_the_oldest_blobs_that_make_room() -> Result<()> {
        let (_directory, store) = open_store(Some(12));
        let oldest = put(&store, b"aa");
        let middle = put(&store, b"bbb");
        let newest = put(&store, b"cccc");
        let now = unix_time()?;
        store.with_db(|db| db.touch_asset(&middle.digest, now + 10))?;
        store.with_db(|db| db.touch_asset(&newest.digest, now + 20))?;
        drop(store.reserve(&descriptor(b"wanted").digest, 5)?);
        assert!(!store.contains(&oldest.digest, 2)?);
        assert!(store.contains(&middle.digest, 3)?);
        assert!(store.contains(&newest.digest, 4)?);
        Ok(())
    }

    /// A 1000-byte volume measured with the production floor.
    fn usage(available: u64, stored: u64, reserved: u64, pending: u64) -> Usage {
        Usage {
            volume: Volume {
                available,
                total: 1000,
            },
            stored,
            reserved,
            pending,
            staging: 0,
        }
    }

    fn budget(max_bytes: Option<u64>) -> ModelStoreConfig {
        ModelStoreConfig {
            max_bytes,
            keep_free_percent: 10,
            ..ModelStoreConfig::default()
        }
    }

    #[test]
    fn the_default_budget_keeps_a_tenth_of_the_volume_free() {
        let auto = room(&budget(None), usage(300, 0, 40, 30));
        assert_eq!(auto.volume, 170);
        assert!(auto.admits(200, 30));
        assert!(!auto.admits(201, 30));
        assert_eq!(room(&budget(None), usage(200, 0, 0, 0)).volume, 100);
        assert_eq!(room(&budget(None), usage(100, 0, 0, 0)).volume, 0);
        assert!(!pressured(&budget(None), usage(100, 0, 0, 0)));
        assert!(pressured(&budget(None), usage(99, 0, 0, 0)));
        let capped = room(&budget(Some(500)), usage(300, 350, 100, 30));
        assert_eq!((capped.volume, capped.budget), (270, 50));
        assert!(!capped.admits(60, 60));
    }

    #[test]
    fn a_cap_counts_staged_bytes_no_reservation_covers() {
        let staged = |staging| Usage {
            staging,
            ..usage(300, 350, 100, 30)
        };
        let capped = room(&budget(Some(500)), staged(40));
        assert_eq!((capped.volume, capped.budget), (270, 10));
        assert_eq!(room(&budget(None), staged(40)).volume, 170);
        assert!(!pressured(&budget(Some(500)), staged(149)));
        assert!(pressured(&budget(Some(500)), staged(150)));
    }

    #[test]
    fn staged_bytes_of_a_failed_download_count_against_the_cap() -> Result<()> {
        let (_directory, store) = open_store(Some(100));
        let failed = descriptor(b"failed");
        store.open_partial(&failed.digest)?.write_all(&[7; 60])?;
        assert_eq!(store.available_bytes()?, 40);
        let Err(refused) = store.reserve(&descriptor(b"next").digest, 50) else {
            bail!("A reservation beside 60 staged bytes was admitted under a cap of 100");
        };
        let budget = refused
            .downcast_ref::<DiskBudgetExceeded>()
            .context("typed budget refusal")?;
        assert_eq!((budget.needed, budget.available), (50, 40));
        let resumed = store.reserve(&failed.digest, 90)?;
        assert_eq!(store.available_bytes()?, 10);
        drop(resumed);
        assert_eq!(store.available_bytes()?, 40);
        Ok(())
    }

    #[test]
    fn staged_bytes_put_a_capped_store_under_pressure() -> Result<()> {
        let (_directory, store) = open_store(Some(10));
        let idle = put(&store, b"idle");
        assert!(store.collect_garbage()?.removed.is_empty());
        store
            .open_partial(&descriptor(b"failed").digest)?
            .write_all(&[7; 6])?;
        assert_eq!(store.collect_garbage()?.removed, vec![idle.digest]);
        Ok(())
    }

    #[test]
    fn collection_counts_only_blobs_still_on_disk_as_freed_room() {
        let config = budget(None);
        let full = usage(150, 0, 0, 0);
        let vanished_then_small = [(10, false), (3, true)];
        assert_eq!(
            collect_to_fit(&config, full, &vanished_then_small, 53, 0),
            Some(2)
        );
        assert_eq!(
            collect_to_fit(&config, full, &vanished_then_small, 54, 0),
            None
        );
        let oldest_first = [(2, true), (3, true), (4, true)];
        assert_eq!(collect_to_fit(&config, full, &oldest_first, 52, 0), Some(1));
        let capped = budget(Some(20));
        let vanished = [(10, false)];
        assert_eq!(
            collect_to_fit(&capped, usage(150, 15, 0, 0), &vanished, 10, 0),
            Some(1)
        );
    }

    /// A blob a deploy leased in a store over its cap, so every collection is under pressure.
    fn leased_under_pressure() -> (tempfile::TempDir, ModelStore, ModelAssetDescriptor) {
        let (directory, store) = open_store(Some(4));
        let leased = put(&store, b"leased");
        store.lease(&leased.digest, "project").expect("a lease");
        (directory, store, leased)
    }

    #[test]
    fn a_deploy_lease_holds_a_blob_under_pressure_until_it_lapses() -> Result<()> {
        let (_directory, store, leased) = leased_under_pressure();
        assert!(store.collect_garbage()?.removed.is_empty());
        let renewed = unix_time()?;
        store.lease(&leased.digest, "project")?;
        assert!(store.collect(renewed + 50, true)?.removed.is_empty());
        let lapsed = store.collect(renewed + 101, true)?.removed;
        assert_eq!(lapsed, vec![leased.digest]);
        Ok(())
    }

    #[test]
    fn an_ended_deploy_lease_frees_the_blob() -> Result<()> {
        let (_directory, store, leased) = leased_under_pressure();
        assert!(store.end_lease(&leased.digest, "project")?);
        assert_eq!(store.collect_garbage()?.removed, vec![leased.digest]);
        Ok(())
    }

    #[tokio::test]
    async fn a_released_reference_wakes_collection_once() -> Result<()> {
        let (_directory, store) = open_store(None);
        let asset = put(&store, b"weights");
        let owner = AssetOwner::new(OwnerKind::Placement, "placement-1")?;
        store.add_ref(&asset.digest, &owner)?;
        let releases = store.releases();
        assert!(store.remove_ref(&asset.digest, &owner)?);
        assert!(!store.remove_ref(&asset.digest, &owner)?);
        let wait =
            |millis| tokio::time::timeout(Duration::from_millis(millis), releases.notified());
        assert!(wait(1_000).await.is_ok());
        assert!(wait(50).await.is_err());
        Ok(())
    }

    #[test]
    fn sweep_removes_staging_files_without_a_job() -> Result<()> {
        let (_directory, store) = open_store(None);
        let orphan = descriptor(b"orphan");
        store.open_partial(&orphan.digest)?.write_all(b"orph")?;
        std::fs::write(store.root().join("incoming/stray.tmp"), b"x")?;
        assert_eq!(store.collect(unix_time()?, false)?.partials_removed, 2);
        assert_eq!(store.partial_len(&orphan.digest)?, 0);
        Ok(())
    }
}
