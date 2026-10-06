//! Signed runtime packs (llama.cpp, MLX), installed on demand into
//! `<state>/runtimes/<runtime>/<build>-<backend>/`. The archive is an ordinary model asset, so
//! it is fetched download-first with the push fallback and verified against its signed sha256.

use super::{
    acquire::AcquisitionManager,
    db::{AssetOwner, OwnerKind, RuntimeRecord},
};
use crate::{enrollment::unix_time, release::ReleaseTrust, vault};
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    DigestAlgorithm, Ed25519PublicKey, MAX_COMPACT_JWS_BYTES, MODEL_MAX_RUNTIMES,
    ModelAssetDescriptor, ModelAssetDigest, ModelAssetState, ModelAssetStatus, ModelBackend,
    ModelRuntime, RUNTIME_PACK_FALLBACK_DIR, RUNTIME_PACK_LISTING, ReleaseTarget, RuntimeInfo,
    RuntimeInstalled, RuntimeManifest, RuntimePack, RuntimePackFile, verify_runtime_manifest,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

const MANIFEST_FLOOR_KEY: &str = "runtime_manifest_floor";
const MANIFEST_KEY: &str = "runtime_manifest";
const MANIFEST_JWS_KEY: &str = "runtime_manifest_jws";
const MANIFEST_FLOOR_FILE: &str = "runtime-manifest-floor.json";
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(60);
const LISTING_MAX_BYTES: u64 = 1024 * 1024;
const START_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const COPY_BUFFER_BYTES: usize = 256 * 1024;

type Slot = (ModelRuntime, ModelBackend);
type Outcome = Option<std::result::Result<(), String>>;

struct PendingInstall {
    pack: RuntimePack,
    outcome: watch::Receiver<Outcome>,
}

/// Where the signed runtime manifest of this device's target lives, and who may sign it.
pub struct RuntimeSource {
    manifest_url: String,
    keys: Vec<Ed25519PublicKey>,
    client: reqwest::Client,
    minimum_sequence: u64,
}

impl RuntimeSource {
    /// `{release base}/runtimes/<target>.jws`, next to the trusted `release.jws`.
    pub fn from_trust(state_dir: &Path, target: ReleaseTarget) -> Result<Self> {
        let trust = ReleaseTrust::load(&state_dir.join("release-trust.json")).context(
            "Runtime packs need this agent's release trust; development builds have none",
        )?;
        let base = trust
            .manifest_url
            .strip_suffix("/release.jws")
            .with_context(|| {
                format!(
                    "Release manifest URL {} does not end in /release.jws",
                    trust.manifest_url
                )
            })?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(MANIFEST_TIMEOUT)
            .build()
            .context("Build the runtime manifest HTTP client")?;
        let mut source = Self::new(
            format!("{base}/runtimes/{}.jws", target.triple()),
            trust.keys()?,
            client,
        );
        source.minimum_sequence = trust.minimum_sequence;
        Ok(source)
    }

    pub fn new(manifest_url: String, keys: Vec<Ed25519PublicKey>, client: reqwest::Client) -> Self {
        Self {
            manifest_url,
            keys,
            client,
            minimum_sequence: 0,
        }
    }

    async fn fetch(&self) -> Result<String> {
        let url = &self.manifest_url;
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Download the runtime manifest {url}"))?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Download the runtime manifest {url}: the server answered {}",
            response.status()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("Download the runtime manifest {url}"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= MAX_COMPACT_JWS_BYTES,
                "Runtime manifest {url} exceeds {MAX_COMPACT_JWS_BYTES} bytes"
            );
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).with_context(|| format!("Runtime manifest {url} is not text"))
    }
}

/// An installed pack; engines spawn its entrypoint.
#[derive(Clone, Debug)]
pub struct InstalledRuntime {
    pub record: RuntimeRecord,
    pub dir: PathBuf,
}

impl InstalledRuntime {
    pub fn entrypoint(&self) -> PathBuf {
        self.dir.join(&self.record.entrypoint)
    }

    /// Adds `fallback/` to the loader path only when the entrypoint cannot load without it,
    /// so GPU drivers keep the host's C++ runtime.
    pub fn env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        if self.record.needs_fallback {
            vec![(
                "LD_LIBRARY_PATH".into(),
                self.dir.join(RUNTIME_PACK_FALLBACK_DIR).into_os_string(),
            )]
        } else {
            Vec::new()
        }
    }

    pub fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(self.entrypoint());
        command.envs(self.env());
        command
    }

    pub fn info(&self) -> RuntimeInfo {
        RuntimeInfo {
            runtime: self.record.runtime,
            backend: self.record.backend,
            build: self.record.build.clone(),
            installed: true,
            size: self.record.size,
        }
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PackListing {
    version: u32,
    runtime: ModelRuntime,
    build: String,
    target: ReleaseTarget,
    backend: ModelBackend,
    entrypoint: String,
    files: Vec<RuntimePackFile>,
}

fn wire<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn slot_name((runtime, backend): Slot) -> String {
    format!("{}/{}", wire(&runtime), wire(&backend))
}

fn archive_descriptor(pack: &RuntimePack) -> ModelAssetDescriptor {
    ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Sha256,
            hex: pack.sha256.clone(),
        },
        size: pack.size,
        file_name: format!(
            "{}-{}-{}-{}.tar.gz",
            wire(&pack.runtime),
            pack.build,
            pack.target.triple(),
            wire(&pack.backend)
        ),
        sources: vec![pack.url.clone()],
    }
}

pub struct RuntimeInstaller {
    root: PathBuf,
    acquisition: AcquisitionManager,
    source: Option<RuntimeSource>,
    target: ReleaseTarget,
    installs: Mutex<HashMap<Slot, PendingInstall>>,
    install_lock: tokio::sync::Mutex<()>,
    manifest_lock: tokio::sync::Mutex<()>,
}

impl RuntimeInstaller {
    pub fn open(
        state_dir: &Path,
        acquisition: AcquisitionManager,
        source: Option<RuntimeSource>,
        target: ReleaseTarget,
    ) -> Result<Arc<Self>> {
        let root = state_dir.join("runtimes");
        private_directory(&root)?;
        Ok(Arc::new(Self {
            root,
            acquisition,
            source,
            target,
            installs: Mutex::default(),
            install_lock: tokio::sync::Mutex::new(()),
            manifest_lock: tokio::sync::Mutex::new(()),
        }))
    }

    fn slot_dir(&self, runtime: ModelRuntime) -> PathBuf {
        self.root.join(wire(&runtime))
    }

    pub fn manifest_url(&self) -> Option<&str> {
        self.source
            .as_ref()
            .map(|source| source.manifest_url.as_str())
    }

    fn install_dir(&self, runtime: ModelRuntime, build: &str, backend: ModelBackend) -> PathBuf {
        self.slot_dir(runtime)
            .join(format!("{build}-{}", wire(&backend)))
    }

    pub fn installed(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<Option<InstalledRuntime>> {
        let Some(record) = self
            .acquisition
            .store()
            .with_db(|db| db.runtime(runtime, backend))?
        else {
            return Ok(None);
        };
        let installed = InstalledRuntime {
            dir: self.install_dir(runtime, &record.build, backend),
            record,
        };
        Ok(installed.entrypoint().is_file().then_some(installed))
    }

    /// Installed packs of `runtime`, the best backend first: GPU backends before the CPU.
    pub fn installed_of(&self, runtime: ModelRuntime) -> Result<Vec<InstalledRuntime>> {
        let records = self.acquisition.store().with_db(|db| db.runtimes())?;
        let mut installed: Vec<_> = records
            .into_iter()
            .filter(|record| record.runtime == runtime)
            .filter_map(|record| {
                self.installed(record.runtime, record.backend)
                    .ok()
                    .flatten()
            })
            .collect();
        installed.sort_by_key(|runtime| runtime.record.backend == ModelBackend::Cpu);
        Ok(installed)
    }

    /// Installed packs, then the packs of the last verified manifest still valid now.
    pub fn infos(&self) -> Result<Vec<RuntimeInfo>> {
        let store = self.acquisition.store();
        let mut infos: Vec<RuntimeInfo> = store
            .with_db(|db| db.runtimes())?
            .into_iter()
            .filter_map(|record| {
                self.installed(record.runtime, record.backend)
                    .ok()
                    .flatten()
            })
            .map(|installed| installed.info())
            .collect();
        for pack in self.available()? {
            if !infos
                .iter()
                .any(|info| info.runtime == pack.runtime && info.backend == pack.backend)
            {
                infos.push(RuntimeInfo {
                    runtime: pack.runtime,
                    backend: pack.backend,
                    build: pack.build,
                    installed: false,
                    size: pack.size,
                });
            }
        }
        infos.truncate(MODEL_MAX_RUNTIMES);
        Ok(infos)
    }

    /// Packs of this target in the last verified manifest, while it is valid.
    pub fn available(&self) -> Result<Vec<RuntimePack>> {
        let now = unix_time()?;
        let manifest: Option<RuntimeManifest> = self
            .acquisition
            .store()
            .with_db(|db| db.setting(MANIFEST_KEY))?
            .and_then(|value| serde_json::from_value(value).ok());
        Ok(manifest
            .filter(|manifest| manifest.expires_at > now)
            .map(|manifest| {
                manifest
                    .packs
                    .into_iter()
                    .filter(|pack| pack.target == self.target)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The release floor and highest accepted runtime sequence survive a model-store reset.
    fn manifest_floor(&self) -> Result<u64> {
        let stored = self
            .acquisition
            .store()
            .with_db(|db| db.setting(MANIFEST_FLOOR_KEY))?;
        let mut floor = stored.and_then(|value| value.as_u64()).unwrap_or(0).max(
            self.source
                .as_ref()
                .map_or(0, |source| source.minimum_sequence),
        );
        let state = self
            .root
            .parent()
            .context("Runtime directory has no state directory")?;
        let path = state.join(MANIFEST_FLOOR_FILE);
        if path.try_exists()? {
            floor = floor.max(serde_json::from_slice::<u64>(&vault::read_private(&path)?)?);
        }
        let trust = state.join("release-trust.json");
        if trust.try_exists()? {
            floor = floor.max(ReleaseTrust::load(&trust)?.minimum_sequence);
        }
        Ok(floor)
    }

    fn remember_manifest(
        &self,
        manifest: &RuntimeManifest,
        compact: &str,
        floor: u64,
        now: i64,
    ) -> Result<()> {
        let floor = manifest.sequence.max(floor);
        let state = self
            .root
            .parent()
            .context("Runtime directory has no state directory")?;
        let temporary = state.join(format!(
            ".{MANIFEST_FLOOR_FILE}.{}.tmp",
            uuid::Uuid::new_v4()
        ));
        vault::write_new_private(&temporary, &serde_json::to_vec(&floor)?)?;
        let written = std::fs::rename(&temporary, state.join(MANIFEST_FLOOR_FILE))
            .and_then(|()| File::open(state)?.sync_all());
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.context("Persist the runtime manifest rollback floor")?;
        let value = serde_json::to_value(manifest)?;
        self.acquisition.store().with_db(|db| {
            db.set_setting(MANIFEST_FLOOR_KEY, &floor.into(), now)?;
            db.set_setting(
                MANIFEST_JWS_KEY,
                &serde_json::Value::String(compact.to_owned()),
                now,
            )?;
            db.set_setting(MANIFEST_KEY, &value, now)
        })
    }

    /// A controller may supply the signed bytes when the device cannot reach the CDN.
    /// A failed fetch may reuse a signed cache entry, checked against current keys, time and floor.
    async fn manifest(&self, supplied: Option<&str>) -> Result<RuntimeManifest> {
        let _guard = self.manifest_lock.lock().await;
        let source = self
            .source
            .as_ref()
            .context("Install a runtime: this agent has no release trust to verify packs with")?;
        let compact = match supplied {
            Some(compact) => compact.to_owned(),
            None => match source.fetch().await {
                Ok(compact) => compact,
                Err(error) => self
                    .acquisition
                    .store()
                    .with_db(|db| db.setting(MANIFEST_JWS_KEY))?
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .with_context(|| {
                        format!(
                            "Fetch the runtime manifest and no signed cache is available: {error:#}"
                        )
                    })?,
            },
        };
        let floor = self.manifest_floor()?;
        let now = unix_time()?;
        let manifest = verify_runtime_manifest(&compact, &source.keys, floor, now)
            .with_context(|| format!("Verify the runtime manifest {}", source.manifest_url))?;
        self.remember_manifest(&manifest, &compact, floor, now)?;
        Ok(manifest)
    }

    async fn pack(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
        supplied: Option<&str>,
    ) -> Result<RuntimePack> {
        let manifest = self.manifest(supplied).await?;
        let pack = manifest
            .find(runtime, self.target, backend)
            .with_context(|| {
                format!(
                    "Install runtime {}: the signed manifest has no pack for {}",
                    slot_name((runtime, backend)),
                    self.target.triple()
                )
            })?;
        Ok(pack.clone())
    }

    /// The answer for a slot that already runs this build.
    fn current(
        &self,
        pack: &RuntimePack,
        descriptor: &ModelAssetDescriptor,
    ) -> Result<Option<RuntimeInstalled>> {
        let installed = self.installed(pack.runtime, pack.backend)?;
        Ok(installed
            .filter(|installed| installed.record.build == pack.build)
            .map(|installed| RuntimeInstalled {
                runtime: installed.info(),
                asset: ModelAssetStatus {
                    digest: descriptor.digest.clone(),
                    job_id: None,
                    state: ModelAssetState::Present,
                },
            }))
    }

    /// Starts or joins the install of a slot. The answer comes before the archive is
    /// present; `wait` follows the install to its end.
    pub async fn install(
        self: &Arc<Self>,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<RuntimeInstalled> {
        self.install_with_manifest(runtime, backend, None).await
    }

    pub async fn install_with_manifest(
        self: &Arc<Self>,
        runtime: ModelRuntime,
        backend: ModelBackend,
        manifest_jws: Option<&str>,
    ) -> Result<RuntimeInstalled> {
        let _guard = self.install_lock.lock().await;
        let pack = self.pack(runtime, backend, manifest_jws).await?;
        let pending = {
            let installs = lock!(self.installs);
            let pending = installs
                .get(&(runtime, backend))
                .filter(|pending| pending.outcome.borrow().is_none());
            ensure!(
                pending.is_none_or(|pending| pending.pack == pack),
                "Another runtime build is still installing; cancel its archive job before changing builds"
            );
            pending.is_some()
        };
        let descriptor = archive_descriptor(&pack);
        if let Some(current) = self.current(&pack, &descriptor)? {
            return Ok(current);
        }
        let owner = AssetOwner::new(OwnerKind::Runtime, slot_name((runtime, backend)))?;
        self.acquisition
            .store()
            .add_ref(&descriptor.digest, &owner)?;
        let asset = match self.acquisition.ensure(&descriptor, None) {
            Ok(asset) => asset,
            Err(error) => {
                if !pending {
                    self.acquisition
                        .store()
                        .remove_ref(&descriptor.digest, &owner)?;
                }
                return Err(error);
            }
        };
        let runtime = RuntimeInfo {
            runtime,
            backend,
            build: pack.build.clone(),
            installed: false,
            size: pack.size,
        };
        if !pending {
            self.start_install(pack, descriptor, owner);
        }
        Ok(RuntimeInstalled { runtime, asset })
    }

    fn start_install(
        self: &Arc<Self>,
        pack: RuntimePack,
        descriptor: ModelAssetDescriptor,
        owner: AssetOwner,
    ) {
        let slot = (pack.runtime, pack.backend);
        let mut installs = lock!(self.installs);
        let (sender, receiver) = watch::channel(None);
        installs.insert(
            slot,
            PendingInstall {
                pack: pack.clone(),
                outcome: receiver,
            },
        );
        drop(installs);
        let installer = Arc::clone(self);
        tokio::spawn(async move {
            let result = installer.finish_install(&pack, &descriptor).await;
            if let Err(error) = installer
                .acquisition
                .store()
                .remove_ref(&descriptor.digest, &owner)
            {
                tracing::warn!(
                    "Release the runtime archive {}: {error:#}",
                    descriptor.file_name
                );
            }
            if let Err(error) = &result {
                tracing::warn!("Install runtime {}: {error:#}", slot_name(slot));
            }
            let _ = sender.send(Some(result.map_err(|error| format!("{error:#}"))));
        });
    }

    async fn finish_install(
        &self,
        pack: &RuntimePack,
        descriptor: &ModelAssetDescriptor,
    ) -> Result<()> {
        self.acquisition
            .wait_for_present(&descriptor.digest)
            .await?;
        let archive = self
            .acquisition
            .store()
            .path_of(&descriptor.digest)?
            .context("Install a runtime: the verified archive left the model store")?;
        let staging = self.root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
        let result = self.unpack(pack, &archive, &staging).await;
        if staging.exists()
            && let Err(error) = std::fs::remove_dir_all(&staging)
        {
            tracing::warn!("Remove runtime staging {}: {error}", staging.display());
        }
        result
    }

    async fn unpack(&self, pack: &RuntimePack, archive: &Path, staging: &Path) -> Result<()> {
        let unpack = {
            let (pack, archive, staging) = (pack.clone(), archive.to_owned(), staging.to_owned());
            tokio::task::spawn_blocking(move || extract(&archive, &pack, &staging))
        };
        let size = unpack.await.context("Unpack a runtime pack")??;
        // A newer manifest may arrive while this archive waits for a client push.
        let _guard = self.manifest_lock.lock().await;
        self.check_current_pack(pack)?;
        let fallback = pack.runtime == ModelRuntime::Llamacpp
            && needs_fallback(staging, &pack.entrypoint).await?;
        self.place(pack, staging)?;
        self.record(pack, size, fallback)
    }

    fn check_current_pack(&self, pack: &RuntimePack) -> Result<()> {
        let source = self
            .source
            .as_ref()
            .context("Runtime release trust is missing")?;
        let compact = self
            .acquisition
            .store()
            .with_db(|db| db.setting(MANIFEST_JWS_KEY))?
            .and_then(|value| value.as_str().map(str::to_owned))
            .context("The signed runtime manifest is missing")?;
        let manifest =
            verify_runtime_manifest(&compact, &source.keys, self.manifest_floor()?, unix_time()?)?;
        ensure!(
            manifest.find(pack.runtime, self.target, pack.backend) == Some(pack),
            "The pending runtime pack was replaced by a newer signed manifest; retry the install"
        );
        Ok(())
    }

    /// Records the installed build and removes the build it replaced.
    fn record(&self, pack: &RuntimePack, size: u64, needs_fallback: bool) -> Result<()> {
        let store = self.acquisition.store();
        let previous = store.with_db(|db| db.runtime(pack.runtime, pack.backend))?;
        let record = RuntimeRecord {
            runtime: pack.runtime,
            backend: pack.backend,
            build: pack.build.clone(),
            entrypoint: pack.entrypoint.clone(),
            size,
            needs_fallback,
            installed_at: unix_time()?,
        };
        store.with_db(|db| db.put_runtime(&record))?;
        if let Some(previous) = previous.filter(|previous| previous.build != pack.build) {
            self.discard(pack.runtime, &previous.build, pack.backend);
        }
        Ok(())
    }

    /// Renames the verified staging directory into the slot, replacing a broken copy.
    fn place(&self, pack: &RuntimePack, staging: &Path) -> Result<()> {
        let target = self.install_dir(pack.runtime, &pack.build, pack.backend);
        private_directory(&self.slot_dir(pack.runtime))?;
        if target.exists() {
            let trash = self.root.join(format!(".trash-{}", uuid::Uuid::new_v4()));
            std::fs::rename(&target, &trash)?;
            std::fs::remove_dir_all(&trash)?;
        }
        std::fs::rename(staging, &target)
            .with_context(|| format!("Move the runtime pack into {}", target.display()))?;
        File::open(self.slot_dir(pack.runtime))?
            .sync_all()
            .context("Sync the runtime directory")
    }

    fn discard(&self, runtime: ModelRuntime, build: &str, backend: ModelBackend) {
        let old = self.install_dir(runtime, build, backend);
        if let Err(error) = std::fs::remove_dir_all(&old) {
            tracing::warn!("Remove the replaced runtime {}: {error}", old.display());
        }
    }

    /// Follows a running or finished install of the slot to its end.
    pub async fn wait(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<InstalledRuntime> {
        let receiver = lock!(self.installs)
            .get(&(runtime, backend))
            .map(|pending| pending.outcome.clone());
        if let Some(mut receiver) = receiver {
            let outcome = receiver
                .wait_for(Option::is_some)
                .await
                .map(|outcome| outcome.clone())
                .ok()
                .flatten();
            if let Some(Err(error)) = outcome {
                bail!("{error}");
            }
        }
        self.installed(runtime, backend)?
            .with_context(|| format!("Runtime {} is not installed", slot_name((runtime, backend))))
    }

    /// Callers stop the engines of the slot first.
    pub fn remove(&self, runtime: ModelRuntime, backend: ModelBackend) -> Result<RuntimeInfo> {
        let record = self
            .acquisition
            .store()
            .with_db(|db| db.runtime(runtime, backend))?
            .with_context(|| {
                format!(
                    "Remove runtime {}: it is not installed",
                    slot_name((runtime, backend))
                )
            })?;
        self.acquisition
            .store()
            .with_db(|db| db.delete_runtime(runtime, backend))?;
        let dir = self.install_dir(runtime, &record.build, backend);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("Remove the runtime files {}", dir.display()))?;
        }
        Ok(RuntimeInfo {
            runtime,
            backend,
            build: record.build,
            installed: false,
            size: record.size,
        })
    }
}

/// Created private and owned by the agent, never through a symlink.
pub(super) fn private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("Create directory {}", path.display()))?;
        let metadata = std::fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let owner = unsafe { libc::geteuid() };
        ensure!(
            metadata.is_dir() && metadata.uid() == owner,
            "Directory {} must be a directory owned by the agent",
            path.display()
        );
        Ok(())
    }
    #[cfg(not(unix))]
    bail!("Directory {} needs Unix file permissions", path.display())
}

fn expected_mode(file: &RuntimePackFile) -> u32 {
    if file.executable { 0o755 } else { 0o644 }
}

/// Unpacks the listing and exactly the signed files in listing order; any other entry,
/// a link, a directory or a byte that differs refuses the pack. Answers the unpacked size.
fn extract(archive: &Path, pack: &RuntimePack, staging: &Path) -> Result<u64> {
    let file = File::open(archive)
        .with_context(|| format!("Open the runtime archive {}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file)));
    private_staging(staging)?;
    let count = each_entry(&mut tar, |index, entry| {
        unpack_entry(index, entry, pack, staging)
    })?;
    ensure!(
        count == pack.files.len() + 1,
        "Runtime archive ends after {count} of its {} entries",
        pack.files.len() + 1
    );
    Ok(pack.files.iter().map(|file| file.size).sum())
}

/// Entry 0 is the listing, entry `n` the signed file `n - 1`.
fn unpack_entry<R: Read>(
    index: usize,
    entry: &mut tar::Entry<R>,
    pack: &RuntimePack,
    staging: &Path,
) -> Result<()> {
    let Some(expected) = index.checked_sub(1) else {
        check_entry(entry, RUNTIME_PACK_LISTING)?;
        return check_listing(entry, pack);
    };
    let file = pack
        .files
        .get(expected)
        .context("Runtime archive holds entries beyond its signed files")?;
    check_entry(entry, &file.path)?;
    check_header(entry.header(), file)?;
    write_file(entry, file, &staging.join(&file.path))
}

fn each_entry<R: Read>(
    archive: &mut tar::Archive<R>,
    mut visit: impl FnMut(usize, &mut tar::Entry<R>) -> Result<()>,
) -> Result<usize> {
    let mut count = 0;
    for entry in archive.entries().context("Read the runtime archive")? {
        let mut entry = entry.context("Read a runtime archive entry")?;
        visit(count, &mut entry)?;
        count += 1;
    }
    Ok(count)
}

/// The archive's own listing must describe exactly the signed pack.
fn check_listing(listing: &mut impl Read, pack: &RuntimePack) -> Result<()> {
    let mut text = Vec::new();
    listing
        .take(LISTING_MAX_BYTES + 1)
        .read_to_end(&mut text)
        .context("Read the runtime pack listing")?;
    ensure!(
        text.len() as u64 <= LISTING_MAX_BYTES,
        "Runtime pack listing exceeds {LISTING_MAX_BYTES} bytes"
    );
    let listing: PackListing =
        serde_json::from_slice(&text).context("Read the runtime pack listing")?;
    let signed = PackListing {
        version: 1,
        runtime: pack.runtime,
        build: pack.build.clone(),
        target: pack.target,
        backend: pack.backend,
        entrypoint: pack.entrypoint.clone(),
        files: pack.files.clone(),
    };
    ensure!(
        listing == signed,
        "Runtime pack listing differs from the signed manifest"
    );
    Ok(())
}

fn check_header(header: &tar::Header, expected: &RuntimePackFile) -> Result<()> {
    let size = header.size()?;
    ensure!(
        size == expected.size,
        "Runtime pack file {} holds {size} bytes, not the signed {}",
        expected.path,
        expected.size
    );
    let mode = header.mode()? & 0o7777;
    ensure!(
        mode == expected_mode(expected),
        "Runtime pack file {} has mode {mode:o}, not {:o}",
        expected.path,
        expected_mode(expected)
    );
    Ok(())
}

fn check_entry<R: Read>(entry: &tar::Entry<R>, expected: &str) -> Result<()> {
    ensure!(
        entry.header().entry_type() == tar::EntryType::Regular,
        "Runtime archive entry {expected} is not a regular file"
    );
    ensure!(
        entry.path_bytes().as_ref() == expected.as_bytes(),
        "Runtime archive holds {} where {expected} belongs",
        String::from_utf8_lossy(&entry.path_bytes())
    );
    Ok(())
}

fn private_staging(staging: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(staging)
            .with_context(|| format!("Create runtime staging {}", staging.display()))
    }
    #[cfg(not(unix))]
    bail!("Runtime packs need Unix file permissions")
}

fn write_file(source: &mut impl Read, expected: &RuntimePackFile, path: &Path) -> Result<()> {
    let mut file = create_file(path, expected_mode(expected))
        .with_context(|| format!("Create runtime pack file {}", expected.path))?;
    let (written, sha256) = copy_hashed(source, &mut file, expected)?;
    ensure!(
        written == expected.size && sha256 == expected.sha256,
        "Runtime pack file {} differs from its signed size or sha256",
        expected.path
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(expected_mode(expected)))?;
    }
    file.sync_all()
        .with_context(|| format!("Sync runtime pack file {}", expected.path))
}

/// A new file, parents included, never through a symlink.
fn create_file(path: &Path, mode: u32) -> Result<File> {
    let parent = path.parent().context("Runtime pack file has no parent")?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(parent)?;
        options.mode(mode).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = (parent, mode);
    Ok(options.open(path)?)
}

/// Copies at most the signed size; answers the bytes copied and their sha256.
fn copy_hashed(
    source: &mut impl Read,
    file: &mut File,
    expected: &RuntimePackFile,
) -> Result<(u64, String)> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
    let mut written = 0u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            return Ok((written, format!("{:x}", hash.finalize())));
        }
        written += read as u64;
        ensure!(
            written <= expected.size,
            "Runtime pack file {} exceeds its signed size",
            expected.path
        );
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
    }
}

/// Starts the entrypoint once without `fallback/`; only a loader failure retries with it.
/// Answers whether starts need it.
async fn needs_fallback(dir: &Path, entrypoint: &str) -> Result<bool> {
    let (status, stderr) = start_probe(dir, entrypoint, false).await?;
    if status.success() {
        return Ok(false);
    }
    let fallback = dir.join(RUNTIME_PACK_FALLBACK_DIR);
    if failed_to_load(&status) && fallback.is_dir() {
        let (status, stderr) = start_probe(dir, entrypoint, true).await?;
        ensure!(
            status.success(),
            "Runtime entrypoint {entrypoint} cannot start even with its fallback libraries ({status}): {stderr}"
        );
        return Ok(true);
    }
    bail!("Runtime entrypoint {entrypoint} cannot start ({status}): {stderr}")
}

/// glibc's loader exits with 127 when a library is missing or too old.
fn failed_to_load(status: &std::process::ExitStatus) -> bool {
    cfg!(target_os = "linux") && status.code() == Some(127)
}

async fn start_probe(
    dir: &Path,
    entrypoint: &str,
    fallback: bool,
) -> Result<(std::process::ExitStatus, String)> {
    let mut command = tokio::process::Command::new(dir.join(entrypoint));
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if fallback {
        command.env("LD_LIBRARY_PATH", dir.join(RUNTIME_PACK_FALLBACK_DIR));
    }
    let output = tokio::time::timeout(START_PROBE_TIMEOUT, command.output())
        .await
        .with_context(|| format!("Start runtime entrypoint {entrypoint}: no exit within 30 s"))?
        .with_context(|| format!("Start runtime entrypoint {entrypoint}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = stderr.lines().rev().take(4).collect();
    Ok((
        output.status,
        tail.into_iter().rev().collect::<Vec<_>>().join(" | "),
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::{
        store::{ModelStore, ModelStoreConfig},
        test_server::Origin,
    };
    use axum::{Router, routing::get};
    use flow_like_device_protocol::{SigningKey, sign_runtime_manifest};
    use std::os::unix::fs::PermissionsExt;

    pub(crate) const ENTRYPOINT: &str = "llama-server";
    const SCRIPT: &[u8] = b"#!/bin/sh\necho llama-server 0.0.1-test >&2\n";

    pub(crate) struct Pack {
        pub archive: Vec<u8>,
        pub files: Vec<RuntimePackFile>,
    }

    fn file(path: &str, bytes: &[u8], executable: bool) -> RuntimePackFile {
        RuntimePackFile {
            path: path.into(),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            executable,
        }
    }

    /// `extra` entries follow the signed files, `tamper` replaces the first file's bytes.
    pub(crate) fn pack_archive(extra: &[(&str, &[u8])], tamper: bool) -> Pack {
        let contents: Vec<(&str, Vec<u8>, bool)> = vec![
            ("lib/libllama.so", b"library bytes".to_vec(), false),
            (ENTRYPOINT, SCRIPT.to_vec(), true),
        ];
        let files: Vec<RuntimePackFile> = contents
            .iter()
            .map(|(path, bytes, executable)| file(path, bytes, *executable))
            .collect();
        let target = ReleaseTarget::current().expect("a release target");
        let listing = serde_json::json!({
            "version": 1, "runtime": "llamacpp", "build": "b1", "target": target,
            "backend": "cpu", "entrypoint": ENTRYPOINT, "files": files,
        });
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut add = |path: &str, bytes: &[u8], mode: u32| {
            let mut header = tar::Header::new_ustar();
            header.set_size(bytes.len() as u64);
            header.set_mode(mode);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, path, bytes)
                .expect("a tar entry");
        };
        add(RUNTIME_PACK_LISTING, listing.to_string().as_bytes(), 0o644);
        for (index, (path, bytes, executable)) in contents.iter().enumerate() {
            let mut bytes = bytes.clone();
            if tamper && index == 0 {
                bytes[0] ^= 1;
            }
            add(path, &bytes, if *executable { 0o755 } else { 0o644 });
        }
        for (path, bytes) in extra {
            add(path, bytes, 0o644);
        }
        let archive = builder
            .into_inner()
            .expect("a tar stream")
            .finish()
            .expect("a gzip stream");
        Pack { archive, files }
    }

    pub(crate) fn manifest_for(origin_url: &str, pack: &Pack, sequence: u64) -> RuntimeManifest {
        let now = unix_time().expect("a clock");
        RuntimeManifest {
            version: 1,
            sequence,
            issued_at: now - 10,
            expires_at: now + 86_400,
            packs: vec![RuntimePack {
                runtime: ModelRuntime::Llamacpp,
                build: "b1".into(),
                target: ReleaseTarget::current().expect("a release target"),
                backend: ModelBackend::Cpu,
                url: origin_url.into(),
                size: pack.archive.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&pack.archive)),
                entrypoint: ENTRYPOINT.into(),
                files: pack.files.clone(),
            }],
        }
    }

    pub(crate) struct Fixture {
        pub _directory: tempfile::TempDir,
        pub installer: Arc<RuntimeInstaller>,
        pub manifest: Arc<Mutex<String>>,
        pub origin: Origin,
        pub key: SigningKey,
    }

    /// Serves whatever manifest was published last, and the pack.
    async fn origin(pack: &Pack, manifest: &Arc<Mutex<String>>) -> Origin {
        let served = Arc::clone(manifest);
        let router = Router::new()
            .route(
                "/runtimes/manifest.jws",
                get(move || {
                    let served = Arc::clone(&served);
                    async move { lock!(served).clone() }
                }),
            )
            .route(
                "/pack.tar.gz",
                crate::models::test_server::serving(Arc::new(pack.archive.clone())),
            );
        Origin::start(router).await
    }

    fn trusting(origin: &Origin) -> reqwest::Client {
        let certificate =
            reqwest::Certificate::from_pem(origin.certificate.as_bytes()).expect("a certificate");
        reqwest::Client::builder()
            .add_root_certificate(certificate)
            .build()
            .expect("a client")
    }

    pub(crate) async fn fixture(pack: &Pack) -> Fixture {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let manifest = Arc::new(Mutex::new(String::new()));
        let origin = origin(pack, &manifest).await;
        let store = Arc::new(
            ModelStore::open(directory.path(), ModelStoreConfig::default()).expect("a store"),
        );
        let acquisition = AcquisitionManager::start(
            store,
            origin.fetcher(),
            crate::models::acquire::AcquisitionConfig {
                attempts_per_source: 1,
                ..Default::default()
            },
        )
        .expect("acquisition");
        let client = trusting(&origin);
        let key = SigningKey::generate();
        let source = RuntimeSource::new(
            origin.url("/runtimes/manifest.jws"),
            vec![key.public_key()],
            client,
        );
        let installer = RuntimeInstaller::open(
            directory.path(),
            acquisition,
            Some(source),
            ReleaseTarget::current().expect("a release target"),
        )
        .expect("an installer");
        Fixture {
            _directory: directory,
            installer,
            manifest,
            origin,
            key,
        }
    }

    impl Fixture {
        pub(crate) fn publish(&self, manifest: &RuntimeManifest) {
            *lock!(self.manifest) =
                sign_runtime_manifest(manifest, &self.key).expect("a signed manifest");
        }

        pub(crate) async fn install(&self) -> Result<InstalledRuntime> {
            self.installer
                .install(ModelRuntime::Llamacpp, ModelBackend::Cpu)
                .await?;
            self.installer
                .wait(ModelRuntime::Llamacpp, ModelBackend::Cpu)
                .await
        }
    }

    #[tokio::test]
    async fn forwarded_signed_manifest_and_client_push_install_without_device_egress() -> Result<()>
    {
        let pack = pack_archive(&[], false);
        let mut fixture = fixture(&pack).await;
        Arc::get_mut(&mut fixture.installer)
            .unwrap()
            .source
            .as_mut()
            .unwrap()
            .manifest_url = fixture.origin.url("/unavailable.jws");
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let archive_url = format!("https://{}/pack.tar.gz", listener.local_addr()?);
        drop(listener);
        let manifest = manifest_for(&archive_url, &pack, 3);
        let forged = sign_runtime_manifest(&manifest, &SigningKey::generate())?;
        assert!(
            fixture
                .installer
                .install_with_manifest(ModelRuntime::Llamacpp, ModelBackend::Cpu, Some(&forged))
                .await
                .is_err()
        );
        assert!(fixture.installer.acquisition.jobs().is_empty());
        let signed = sign_runtime_manifest(&manifest, &fixture.key)?;
        let reply = fixture
            .installer
            .install_with_manifest(ModelRuntime::Llamacpp, ModelBackend::Cpu, Some(&signed))
            .await?;
        let acquisition = &fixture.installer.acquisition;
        let state = tokio::time::timeout(
            Duration::from_secs(5),
            acquisition.settled(&reply.asset.digest),
        )
        .await??;
        assert!(
            matches!(
                state,
                ModelAssetState::Failed {
                    reason: flow_like_device_protocol::ModelAssetFailure::EgressBlocked,
                    ..
                }
            ),
            "{state:?}"
        );
        assert_eq!(
            acquisition.begin_push(&reply.asset.digest, false).await?,
            ModelAssetState::AwaitingPush { bytes: 0 }
        );
        assert!(
            fixture
                .installer
                .installed(ModelRuntime::Llamacpp, ModelBackend::Cpu)?
                .is_none()
        );
        assert_eq!(
            acquisition
                .push_chunk(&reply.asset.digest, 0, &pack.archive)
                .await?,
            ModelAssetState::Present
        );
        let installed = tokio::time::timeout(
            Duration::from_secs(5),
            fixture
                .installer
                .wait(ModelRuntime::Llamacpp, ModelBackend::Cpu),
        )
        .await??;
        assert_eq!(installed.record.build, "b1");
        assert_eq!(std::fs::read(installed.entrypoint())?, SCRIPT);
        assert_eq!(
            fixture.origin.hits.total(),
            0,
            "the supplied manifest bypasses the device's unavailable source"
        );
        acquisition.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_pending_archive_cannot_be_replaced_or_install_after_its_manifest_changes()
    -> Result<()> {
        let pack = pack_archive(&[], false);
        let fixture = fixture(&pack).await;
        let manifest = manifest_for(&fixture.origin.url("/missing.tar.gz"), &pack, 1);
        let signed = sign_runtime_manifest(&manifest, &fixture.key)?;
        let install = || {
            fixture.installer.install_with_manifest(
                ModelRuntime::Llamacpp,
                ModelBackend::Cpu,
                Some(&signed),
            )
        };
        let first = install().await?;
        let acquisition = &fixture.installer.acquisition;
        acquisition.settled(&first.asset.digest).await?;
        acquisition.begin_push(&first.asset.digest, false).await?;
        let duplicate = install().await?;
        assert_eq!(duplicate.asset.job_id, first.asset.job_id);
        assert_eq!(acquisition.jobs().len(), 1);
        assert_eq!(acquisition.store().refs(&first.asset.digest)?.len(), 1);
        let mut next = manifest.clone();
        next.sequence = 2;
        next.packs[0].build = "b2".into();
        next.packs[0].sha256 = "b".repeat(64);
        let next_signed = sign_runtime_manifest(&next, &fixture.key)?;
        let refused = fixture
            .installer
            .install_with_manifest(
                ModelRuntime::Llamacpp,
                ModelBackend::Cpu,
                Some(&next_signed),
            )
            .await
            .unwrap_err();
        assert!(format!("{refused:#}").contains("cancel its archive job"));
        let next_digest = archive_descriptor(&next.packs[0]).digest;
        assert!(acquisition.state(&next_digest)?.is_none());
        assert!(acquisition.store().refs(&next_digest)?.is_empty());
        acquisition
            .push_chunk(&first.asset.digest, 0, &pack.archive)
            .await?;
        let refused = tokio::time::timeout(
            Duration::from_secs(5),
            fixture
                .installer
                .wait(ModelRuntime::Llamacpp, ModelBackend::Cpu),
        )
        .await?
        .unwrap_err();
        assert!(format!("{refused:#}").contains("replaced by a newer signed manifest"));
        assert!(
            fixture
                .installer
                .installed(ModelRuntime::Llamacpp, ModelBackend::Cpu)?
                .is_none()
        );
        assert!(acquisition.store().refs(&first.asset.digest)?.is_empty());
        acquisition.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn cached_manifest_is_reverified_after_restart_against_time_signature_and_floor()
    -> Result<()> {
        let pack = pack_archive(&[], false);
        let fixture = fixture(&pack).await;
        let manifest = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 3);
        let signed = sign_runtime_manifest(&manifest, &fixture.key)?;
        fixture.installer.manifest(Some(&signed)).await?;
        let restarted = RuntimeInstaller::open(
            fixture._directory.path(),
            fixture.installer.acquisition.clone(),
            Some(RuntimeSource::new(
                fixture.origin.url("/unavailable.jws"),
                vec![fixture.key.public_key()],
                trusting(&fixture.origin),
            )),
            ReleaseTarget::current().unwrap(),
        )?;
        assert_eq!(restarted.manifest(None).await?.sequence, 3);
        let mut expired = manifest.clone();
        expired.issued_at = unix_time()? - 100;
        expired.expires_at = unix_time()? - 1;
        let mut rolled_back = manifest.clone();
        rolled_back.sequence = 2;
        for invalid in [
            sign_runtime_manifest(&expired, &fixture.key)?,
            sign_runtime_manifest(&rolled_back, &fixture.key)?,
            sign_runtime_manifest(&manifest, &SigningKey::generate())?,
        ] {
            restarted
                .acquisition
                .store()
                .with_db(|db| db.set_setting(MANIFEST_JWS_KEY, &invalid.into(), unix_time()?))?;
            assert!(restarted.manifest(None).await.is_err());
        }
        assert!(restarted.acquisition.jobs().is_empty());
        restarted.acquisition.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_floor_uses_release_trust_and_survives_a_fresh_model_database() -> Result<()> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let pack = pack_archive(&[], false);
        let mut fixture = fixture(&pack).await;
        let state = fixture._directory.path();
        let trust = ReleaseTrust {
            manifest_url: fixture.origin.url("/release.jws"),
            public_keys: vec![URL_SAFE_NO_PAD.encode(fixture.key.public_key().to_bytes()?)],
            minimum_sequence: 7,
        };
        vault::write_new_private(
            &state.join("release-trust.json"),
            &serde_json::to_vec(&trust)?,
        )?;
        Arc::get_mut(&mut fixture.installer).unwrap().source = Some(RuntimeSource::from_trust(
            state,
            ReleaseTarget::current().unwrap(),
        )?);
        let old = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 6);
        let error = fixture
            .installer
            .manifest(Some(&sign_runtime_manifest(&old, &fixture.key)?))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("rollback"));
        let current = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 8);
        fixture
            .installer
            .manifest(Some(&sign_runtime_manifest(&current, &fixture.key)?))
            .await?;
        let replacement = tempfile::tempdir()?;
        let acquisition = AcquisitionManager::start(
            Arc::new(ModelStore::open(
                replacement.path(),
                ModelStoreConfig::default(),
            )?),
            fixture.origin.fetcher(),
            Default::default(),
        )?;
        let restarted = RuntimeInstaller::open(
            state,
            acquisition,
            Some(RuntimeSource::from_trust(
                state,
                ReleaseTarget::current().unwrap(),
            )?),
            ReleaseTarget::current().unwrap(),
        )?;
        assert!(
            restarted
                .acquisition
                .store()
                .with_db(|db| db.setting(MANIFEST_FLOOR_KEY))?
                .is_none()
        );
        assert_eq!(restarted.manifest_floor()?, 8);
        let older = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 7);
        let error = restarted
            .manifest(Some(&sign_runtime_manifest(&older, &fixture.key)?))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("rollback"));
        fixture.installer.acquisition.shutdown().await;
        restarted.acquisition.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_or_stopping_acquisition_ends_a_runtime_waiting_for_push() -> Result<()> {
        for shutdown in [false, true] {
            let pack = pack_archive(&[], false);
            let fixture = fixture(&pack).await;
            let signed = sign_runtime_manifest(
                &manifest_for(&fixture.origin.url("/missing.tar.gz"), &pack, 1),
                &fixture.key,
            )?;
            let reply = fixture
                .installer
                .install_with_manifest(ModelRuntime::Llamacpp, ModelBackend::Cpu, Some(&signed))
                .await?;
            let acquisition = &fixture.installer.acquisition;
            acquisition.settled(&reply.asset.digest).await?;
            if shutdown {
                acquisition.shutdown().await;
            } else {
                acquisition.cancel(&reply.asset.digest).await?;
            }
            assert!(
                tokio::time::timeout(
                    Duration::from_secs(5),
                    fixture
                        .installer
                        .wait(ModelRuntime::Llamacpp, ModelBackend::Cpu)
                )
                .await?
                .is_err()
            );
            acquisition.shutdown().await;
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_signed_pack_installs_its_listed_files_with_their_modes() -> Result<()> {
        let pack = pack_archive(&[], false);
        let fixture = fixture(&pack).await;
        fixture.publish(&manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 3));
        let installed = fixture.install().await?;
        assert_eq!(installed.record.build, "b1");
        assert!(!installed.record.needs_fallback);
        let mode = |path: &str| {
            std::fs::metadata(installed.dir.join(path))
                .expect("an installed file")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(ENTRYPOINT), 0o755);
        assert_eq!(mode("lib/libllama.so"), 0o644);
        assert!(!installed.dir.join(RUNTIME_PACK_LISTING).exists());
        let infos = fixture.installer.infos()?;
        assert_eq!(infos.len(), 1);
        assert!(infos[0].installed);
        assert_eq!(infos[0].size, 13 + SCRIPT.len() as u64);

        let mut rolled_back = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 2);
        rolled_back.packs[0].build = "b0".into();
        fixture.publish(&rolled_back);
        let refused = fixture
            .installer
            .install(ModelRuntime::Llamacpp, ModelBackend::Cpu)
            .await;
        assert!(format!("{:#}", refused.unwrap_err()).contains("rollback"));

        let removed = fixture
            .installer
            .remove(ModelRuntime::Llamacpp, ModelBackend::Cpu)?;
        assert!(!removed.installed);
        assert!(!installed.dir.exists());
        Ok(())
    }

    #[tokio::test]
    async fn tampered_or_padded_packs_are_refused() -> Result<()> {
        for pack in [
            pack_archive(&[], true),
            pack_archive(&[("extra.so", b"more")], false),
        ] {
            let signed = pack_archive(&[], false);
            let fixture = fixture(&pack).await;
            let mut manifest = manifest_for(&fixture.origin.url("/pack.tar.gz"), &pack, 1);
            manifest.packs[0].files = signed.files.clone();
            fixture.publish(&manifest);
            let error = fixture.install().await.unwrap_err();
            assert!(
                format!("{error:#}").contains("differs")
                    || format!("{error:#}").contains("beyond its signed files"),
                "{error:#}"
            );
            assert!(
                fixture
                    .installer
                    .installed(ModelRuntime::Llamacpp, ModelBackend::Cpu)?
                    .is_none()
            );
        }
        Ok(())
    }
}
