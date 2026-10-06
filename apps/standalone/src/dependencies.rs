use crate::config::PlacementConfig;
use crate::models::{
    acquire::AcquisitionManager,
    db::{AssetOwner, OwnerKind},
    store::ModelStore,
};
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    ModelAssetDescriptor, ModelAssetDigest, ModelAssetState, PACKAGED_BIT_METADATA_MAX_BYTES,
    PackagedBitAsset, PackagedBitMetadata, ProjectArtifactFile, ProjectBitPin, artifact_sha256,
    validate_artifact_digest, validate_artifact_relative_path,
};
use flow_like_runtime::{
    app::App,
    bit::{Bit, BitPack},
    flow::{
        board::Board,
        execution::{ExecutionEnvironment, context::ExecutionContext},
        node::{Node, NodeLogic},
    },
    profile::{Profile, ProfileCustomBit},
    state::{CompletionModelCapabilities, FlowLikeState},
    utils::compression::compress_to_file_json,
};
use flow_like_wasm::{WasmConfig, WasmEngine, WasmNodeLogic, WasmSecurityConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// How a placement fails when one of its model-store assets is missing and cannot be acquired
/// again; the deployer then offers to send it from the user's computer.
pub const MODEL_ASSET_MISSING: &str = "model_asset_missing";

fn open_bounded(root: &Path, components: &[&str], limit: u64) -> Result<File> {
    ensure!(
        !components.is_empty() && components.iter().all(|part| !part.contains(['/', '\\'])),
        "Invalid dependency path components"
    );
    flow_like_device_protocol::validate_artifact_relative_path(&components.join("/"))?;
    let mut path = root.canonicalize()?;
    for (index, component) in components.iter().enumerate() {
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("Missing packaged dependency {}", path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Dependency must not be a symlink"
        );
        if index + 1 < components.len() {
            ensure!(metadata.is_dir(), "Dependency parent must be a directory");
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Dependency exceeds its file limit"
    );
    Ok(file)
}

pub(crate) fn read_bounded(root: &Path, components: &[&str], limit: u64) -> Result<Vec<u8>> {
    let file = open_bounded(root, components, limit)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Dependency exceeds its file limit"
    );
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagedBit {
    pub bit: Bit,
    pub dependencies: Vec<Bit>,
    pub artifacts: Vec<ProjectArtifactFile>,
    /// Model-store files of metadata v2, which a placement never copies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<PackagedBitAsset>,
}

/// A pin's metadata as a placement reads it.
struct PinnedPack {
    pack: PackagedBit,
    /// Metadata v2, whose model Bits the device's model host serves.
    version2: bool,
    metadata_bytes: usize,
}

impl PinnedPack {
    /// The root runs on the device's model host instead of in the placement.
    fn hosted(&self) -> bool {
        self.version2 && crate::models::router::hosts(&self.pack.bit)
    }
}

fn read_pinned(config: &PlacementConfig, pin: &ProjectBitPin) -> Result<PinnedPack> {
    pin.validate()?;
    let bytes = read_bounded(
        &config.project_path,
        &["bits", "metadata", &format!("{}.json", pin.bit_id)],
        PACKAGED_BIT_METADATA_MAX_BYTES,
    )?;
    let metadata = crate::project_artifacts::packaged_metadata(&bytes, pin)?;
    let typed = |value: &serde_json::Value| {
        serde_json::from_value::<Bit>(value.clone())
            .with_context(|| format!("Read a Bit in the metadata of Bit {}", pin.bit_id))
    };
    let pack = PackagedBit {
        bit: typed(metadata.bit())?,
        dependencies: metadata
            .dependencies()
            .iter()
            .map(typed)
            .collect::<Result<_>>()?,
        artifacts: metadata.artifacts().to_vec(),
        assets: metadata.assets().to_vec(),
    };
    verify_pack_metadata(&pack, &pin.bit_id)?;
    Ok(PinnedPack {
        pack,
        version2: matches!(metadata, PackagedBitMetadata::V2(_)),
        metadata_bytes: bytes.len(),
    })
}

/// Where a Bit's file lives in a project artifact, `bits/<hash>/<file_name>`.
fn bit_location(bit: &Bit) -> Result<Option<String>> {
    let Some(file) = &bit.file_name else {
        ensure!(
            bit.download_link.is_none(),
            "Bit {} has a download without a file",
            bit.id
        );
        return Ok(None);
    };
    crate::config::validate_id("Bit artifact hash", &bit.hash)?;
    ensure!(
        !matches!(bit.hash.as_str(), "metadata" | "deps-cache"),
        "Bit artifact uses a reserved directory"
    );
    let path = format!("bits/{}/{file}", bit.hash);
    validate_artifact_relative_path(&path)?;
    Ok(Some(path))
}

fn bit_asset_path(bit: &Bit) -> Result<Option<String>> {
    let path = bit_location(bit)?;
    ensure!(
        path.is_none()
            || bit
                .size
                .is_some_and(|size| size > 0 && size <= 4 * 1024 * 1024 * 1024),
        "Bit artifact size is missing or exceeds its limit"
    );
    Ok(path)
}

/// The artifact files a pack's Bits name, with their sizes. A Bit whose bytes are model-store
/// assets names a location but no artifact file.
fn artifact_inventory(pack: &PackagedBit) -> Result<BTreeMap<String, u64>> {
    let stored: HashSet<&str> = pack
        .assets
        .iter()
        .map(|asset| asset.bit_id.as_str())
        .collect();
    let mut ids = HashSet::new();
    let mut assets = BTreeMap::new();
    for bit in std::iter::once(&pack.bit).chain(&pack.dependencies) {
        crate::config::validate_id("Bit", &bit.id)?;
        ensure!(ids.insert(&bit.id), "Duplicate Bit dependency");
        if stored.contains(bit.id.as_str()) {
            bit_location(bit)?
                .with_context(|| format!("Bit {} has model assets but no file", bit.id))?;
        } else {
            record_artifact(&mut assets, bit)?;
        }
    }
    Ok(assets)
}

fn record_artifact(assets: &mut BTreeMap<String, u64>, bit: &Bit) -> Result<()> {
    if let Some(path) = bit_asset_path(bit)?
        && let Some(size) = assets.insert(path, bit.size.unwrap())
    {
        ensure!(
            Some(size) == bit.size,
            "Bit artifacts disagree about file size"
        );
    }
    Ok(())
}

fn verify_pack_metadata(pack: &PackagedBit, expected: &str) -> Result<()> {
    ensure!(pack.bit.id == expected, "Bit metadata identity differs");
    ensure!(
        pack.dependencies.len() <= 2048 && pack.artifacts.len() + pack.assets.len() <= 2049,
        "Too many Bit dependencies"
    );
    let mut assets = artifact_inventory(pack)?;
    let ids: HashSet<&String> = std::iter::once(&pack.bit)
        .chain(&pack.dependencies)
        .map(|bit| &bit.id)
        .collect();
    for bit in std::iter::once(&pack.bit).chain(&pack.dependencies) {
        ensure!(
            bit.dependencies.iter().all(|id| ids.contains(id)),
            "Bit {} has an unpackaged dependency",
            bit.id
        );
    }
    // These assets normally materialize at model dispatch. Require their exact
    // identities now so a deployed workload never fetches them from an ambient hub.
    let mut inline = pack.bit.inline_mlx_asset_bits()?;
    inline.extend(pack.bit.projection_bit());
    for bit in inline {
        let included = pack
            .dependencies
            .iter()
            .find(|dependency| dependency.id == bit.id)
            .context("An inline model asset is absent from the package")?;
        ensure!(
            included.hash == bit.hash
                && included.file_name == bit.file_name
                && included.size == bit.size,
            "Inline model asset identity differs"
        );
    }
    ensure!(
        assets.len() == pack.artifacts.len(),
        "Bit package artifact inventory differs"
    );
    for file in &pack.artifacts {
        validate_artifact_digest(&file.sha256)?;
        ensure!(
            assets.remove(&file.path) == Some(file.size),
            "Bit package has an undeclared or duplicate artifact"
        );
    }
    ensure!(assets.is_empty(), "Bit package is missing an artifact");
    Ok(())
}

fn verify_asset(file: &mut File, asset: &ProjectArtifactFile) -> Result<()> {
    ensure!(
        file.metadata()?.len() == asset.size,
        "Bit artifact size differs"
    );
    let mut sha = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(total <= asset.size, "Bit artifact grew during verification");
        sha.update(&buffer[..count]);
    }
    ensure!(
        total == asset.size && format!("{:x}", sha.finalize()) == asset.sha256,
        "Bit artifact digest differs"
    );
    Ok(())
}

fn prepare_bit_assets(
    root: &Path,
    assets: &[ProjectArtifactFile],
    destination: Option<&Path>,
    trust_markers: bool,
) -> Result<()> {
    for asset in assets {
        let parts = asset.path.split('/').collect::<Vec<_>>();
        let mut source = open_bounded(root, &parts, asset.size)?;
        let Some(destination) = destination else {
            verify_asset(&mut source, asset)?;
            continue;
        };
        ensure!(
            parts.first() == Some(&"bits"),
            "Invalid selected Bit asset prefix"
        );
        if root.join("bits").canonicalize()? == destination.canonicalize()? {
            verify_asset(&mut source, asset)?;
            continue;
        }
        materialize_bit_asset(source, destination, &parts[1..], asset, trust_markers)?;
    }
    Ok(())
}

/// Binds a verified digest to the exact published inode. Any write, replacement
/// or rename changes the inode or its ctime, which invalidates the marker.
#[cfg(unix)]
fn verification_stamp(metadata: &std::fs::Metadata, asset: &ProjectArtifactFile) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!(
        "{} {} {} {} {} {} {} {}",
        asset.sha256,
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    ))
}

#[cfg(not(unix))]
fn verification_stamp(_: &std::fs::Metadata, _: &ProjectArtifactFile) -> Option<String> {
    None
}

fn is_verified(marker: Option<&Path>, stamp: Option<&str>) -> bool {
    marker.zip(stamp).is_some_and(|(marker, stamp)| {
        crate::vault::read_private(marker).is_ok_and(|bytes| bytes.as_slice() == stamp.as_bytes())
    })
}

/// The marker only saves later replicas a full hash, so failing to write it,
/// for example on a full placement quota, never fails a verified start.
fn record_verified(controls: &Path, marker: Option<&Path>, stamp: Option<&str>) {
    let Some((marker, stamp)) = marker.zip(stamp) else {
        return;
    };
    let temporary = controls.join(format!(".{}.verified.tmp", uuid::Uuid::new_v4()));
    let result = crate::vault::write_new_private(&temporary, stamp.as_bytes())
        .and_then(|()| Ok(std::fs::rename(&temporary, marker)?));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        tracing::warn!(
            marker = %marker.display(),
            error = %format!("{error:#}"),
            "Cannot record a verified Bit import; later starts will hash it again"
        );
    }
}

/// A sandboxed workload could replace a cached Bit and forge its marker, so its
/// starts pass `trust_markers = false` and re-hash. Process-profile workloads
/// run as the agent account, where a marker grants nothing they lack.
fn materialize_bit_asset(
    mut source: File,
    destination: &Path,
    parts: &[&str],
    asset: &ProjectArtifactFile,
    trust_markers: bool,
) -> Result<()> {
    use fs2::FileExt;
    #[cfg(unix)]
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    ensure!(!parts.is_empty(), "Missing Bit asset path");
    crate::runtime::private_runtime_directory(destination)?;
    let controls = destination
        .parent()
        .context("Bit store has no parent")?
        .join("bit-imports");
    crate::runtime::private_runtime_directory(&controls)?;
    let key = artifact_sha256(asset.path.as_bytes());
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let lock = options.open(controls.join(format!("{key}.lock")))?;
    let metadata = lock.metadata()?;
    ensure!(metadata.is_file(), "Bit import lock must be a regular file");
    #[cfg(unix)]
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1,
        "Invalid private Bit import lock"
    );
    // Multiple replicas can start together. The lock covers only this asset;
    // process exit releases it, and a bounded wait avoids a wedged startup.
    let started = std::time::Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && started.elapsed() < Duration::from_secs(60) =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error).context("Timed out waiting for a selected Bit import"),
        }
    }
    let mut target = destination.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        target.push(part);
        crate::runtime::private_runtime_directory(&target)?;
    }
    target.push(parts[parts.len() - 1]);
    let marker = trust_markers.then(|| controls.join(format!("{key}.verified")));
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Cached Bit must be a regular file"
            );
            #[cfg(unix)]
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o077 == 0
                    && metadata.nlink() == 1,
                "Cached Bit must be private with one link"
            );
            let mut file = open_bounded(destination, parts, asset.size)?;
            let stamp = verification_stamp(&file.metadata()?, asset);
            if is_verified(marker.as_deref(), stamp.as_deref()) {
                return Ok(());
            }
            // A published Bit is never rewritten in place, so replicas verify
            // it concurrently instead of queueing full hashes behind the lock.
            drop(lock);
            verify_asset(&mut file, asset)?;
            ensure!(
                verification_stamp(&file.metadata()?, asset) == stamp
                    && verification_stamp(&std::fs::symlink_metadata(&target)?, asset) == stamp,
                "Cached Bit {} changed during verification",
                asset.path
            );
            record_verified(&controls, marker.as_deref(), stamp.as_deref());
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let partial = controls.join(format!("{key}.partial"));
    match std::fs::symlink_metadata(&partial) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Incomplete Bit import must be a regular file"
            );
            #[cfg(unix)]
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o077 == 0
                    && metadata.nlink() == 1,
                "Invalid incomplete Bit import"
            );
            std::fs::remove_file(&partial)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    struct Partial(PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let mut output = options.open(&partial)?;
    let _partial = Partial(partial.clone());
    let mut sha = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(
            total <= asset.size,
            "Bit artifact grew during materialization"
        );
        output
            .write_all(&buffer[..count])
            .context("Materialize selected Bit within placement disk quota")?;
        sha.update(&buffer[..count]);
    }
    ensure!(
        total == asset.size && format!("{:x}", sha.finalize()) == asset.sha256,
        "Bit artifact digest differs"
    );
    output.sync_all()?;
    std::fs::rename(&partial, &target)?;
    File::open(target.parent().context("Bit target has no parent")?)?.sync_all()?;
    record_verified(
        &controls,
        marker.as_deref(),
        verification_stamp(&std::fs::symlink_metadata(&target)?, asset).as_deref(),
    );
    File::open(&controls)?.sync_all()?;
    Ok(())
}

fn remove_downloads(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("download_link");
            for value in object.values_mut() {
                remove_downloads(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                remove_downloads(value);
            }
        }
        _ => {}
    }
}

fn resolve_pinned_bit(profile: &Profile, reference: &str) -> Result<Bit> {
    ensure!(reference.len() <= 4096, "Bit reference exceeds its limit");
    let (hub, id) = match reference.rsplit_once(':') {
        Some((hub, id)) => (Some(hub), id),
        None => (None, reference),
    };
    let bit = profile
        .custom_bit(id)
        .context("Bit is absent from this placement's selected metadata")?;
    ensure!(
        hub.is_none_or(|hub| hub == bit.hub),
        "Bit reference changes its pinned hub"
    );
    Ok(bit)
}

struct PinnedBitFromString {
    definition: Node,
}

#[flow_like_types::async_trait]
impl NodeLogic for PinnedBitFromString {
    fn get_node(&self) -> Node {
        self.definition.clone()
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let reference: String = context.evaluate_pin("bit_id").await?;
        let bit = resolve_pinned_bit(&context.profile, &reference)?;
        context
            .set_pin_value("output_bit", serde_json::to_value(bit)?)
            .await?;
        context.activate_exec_pin("exec_out").await
    }
}

pub(crate) async fn hydrate_bits(
    config: &PlacementConfig,
    app: &App,
    state: &Arc<FlowLikeState>,
    profile: &mut Profile,
    materialize: bool,
) -> Result<()> {
    let selected: HashSet<&str> = config
        .bit_pins
        .iter()
        .map(|pin| pin.bit_id.as_str())
        .collect();
    ensure!(
        app.bits
            .iter()
            .all(|reference| selected.contains(reference.rsplit(':').next().unwrap_or(reference))),
        "Project Bits require explicit packaged metadata pins"
    );
    let mut bits = BTreeMap::<String, Bit>::new();
    let mut root_dependencies = BTreeMap::new();
    let mut metadata_bytes = 0usize;
    let mut assets = BTreeMap::<String, ProjectArtifactFile>::new();
    let mut stored = Vec::new();
    let mut hosted = HashSet::new();
    for pin in &config.bit_pins {
        let pinned = read_pinned(config, pin)?;
        metadata_bytes += pinned.metadata_bytes;
        ensure!(
            metadata_bytes <= 64 * 1024 * 1024,
            "Selected Bit metadata exceeds 64 MiB"
        );
        stored.extend(stored_assets(&pinned)?);
        hosted.extend(pinned.hosted().then(|| pin.bit_id.clone()));
        let pack = pinned.pack;
        for asset in &pack.artifacts {
            if let Some(previous) = assets.insert(asset.path.clone(), asset.clone()) {
                ensure!(
                    previous == *asset,
                    "Selected packages disagree about a Bit asset"
                );
            }
        }
        ensure!(
            assets.len() <= flow_like_device_protocol::PROJECT_ARTIFACT_MAX_FILES
                && assets.values().map(|asset| asset.size).sum::<u64>()
                    <= flow_like_device_protocol::PROJECT_ARTIFACT_MAX_BYTES,
            "Selected Bit assets exceed their aggregate file or byte bound"
        );
        root_dependencies.insert(
            pin.bit_id.clone(),
            pack.dependencies
                .iter()
                .map(|bit| bit.id.clone())
                .collect::<Vec<_>>(),
        );
        for bit in std::iter::once(pack.bit).chain(pack.dependencies) {
            if let Some(previous) = bits.insert(bit.id.clone(), bit.clone()) {
                ensure!(
                    serde_json::to_value(previous)? == serde_json::to_value(&bit)?,
                    "Selected packages disagree about Bit metadata"
                );
            }
        }
        ensure!(bits.len() <= 4096, "Too many distinct packaged Bits");
    }
    let read: Vec<&StoredAsset> = stored.iter().filter(|asset| !asset.hosted).collect();
    let bit_store = FlowLikeState::bit_store(state).await?;
    let destination = if materialize {
        match &bit_store {
            flow_like_storage::files::store::FlowLikeStore::Local(store) => {
                Some(store.directory_to_filesystem(&flow_like_storage::Path::from(""))?)
            }
            _ if assets.len() + read.len() == 0 => None,
            _ => anyhow::bail!("Selected Bit assets require a private local runtime store"),
        }
    } else {
        None
    };
    if !assets.is_empty() {
        let root = config.project_path.clone();
        let selected = assets.into_values().collect::<Vec<_>>();
        let target = destination.clone();
        let trust_markers = !crate::isolation::sandboxed(config);
        tokio::task::spawn_blocking(move || {
            prepare_bit_assets(&root, &selected, target.as_deref(), trust_markers)
        })
        .await??;
    }
    verify_stored_assets(destination.as_deref(), &read)?;
    let routing = routes_to_model_host(state);
    let capabilities = FlowLikeState::completion_model_capabilities(state).await;
    for bit in bits.values_mut() {
        check_model_runtime(bit, &capabilities, hosted.contains(&bit.id), routing)?;
        // Pins and the imported artifact manifest authorize these local bytes.
        // A source URL is no longer a fallback or a background refresh policy.
        bit.download_link = None;
        remove_downloads(&mut bit.parameters);
    }
    let originals = bits.clone();
    for (id, bit) in &mut bits {
        let mut pending = bit.dependencies.clone();
        // Root packs include inline model assets even when the source Bit's
        // dependency list was empty.
        if let Some(dependencies) = root_dependencies.get(id) {
            pending.extend(dependencies.iter().cloned());
        }
        let mut visited = HashSet::new();
        let mut dependencies = Vec::new();
        while let Some(dependency) = pending.pop() {
            ensure!(dependency != *id, "Cyclic Bit dependency");
            if !visited.insert(dependency.clone()) {
                continue;
            }
            let value = originals
                .get(&dependency)
                .context("Missing Bit dependency")?;
            pending.extend(value.dependencies.iter().cloned());
            if let Some(dependencies) = root_dependencies.get(&dependency) {
                pending.extend(dependencies.iter().cloned());
            }
            dependencies.push(value.clone());
        }
        dependencies.sort_by(|a, b| a.id.cmp(&b.id));
        bit.dependencies = dependencies
            .iter()
            .map(|dependency| dependency.id.clone())
            .collect();
        let pack = BitPack { bits: dependencies };
        let identity = artifact_sha256(&serde_json::to_vec(&(id, &pack))?);
        bit.dependency_tree_hash = format!("standalone-{identity}");
    }
    let store = bit_store.as_generic();
    if materialize && let Some(destination) = &destination {
        crate::runtime::private_runtime_directory(&destination.join("deps-cache"))?;
    }
    for bit in bits.values().filter(|_| materialize) {
        if !bit.dependencies.is_empty() {
            let pack = BitPack {
                bits: bit.dependencies.iter().map(|id| bits[id].clone()).collect(),
            };
            compress_to_file_json(
                store.clone(),
                flow_like_storage::Path::from(format!(
                    "deps-cache/bit-deps-{}.bin",
                    bit.dependency_tree_hash
                )),
                &pack,
            )
            .await?;
        }
    }
    profile.bits = config
        .bit_pins
        .iter()
        .map(|pin| pin.bit_id.clone())
        .collect();
    profile.custom_bits = bits.into_values().map(ProfileCustomBit).collect();
    // Model calls use the broker's resource URL. Discovery and Load Bit remain
    // confined to metadata explicitly packaged for this placement.
    profile.hub.clear();
    profile.hubs.clear();
    let mut registry = state.node_registry.write().await;
    if let Ok(definition) = registry.get_node("bit_from_string") {
        registry.push_node(Arc::new(PinnedBitFromString { definition }));
    }
    Ok(())
}

/// A model-store file of a pinned Bit and its location below a placement's Bit store.
struct StoredAsset {
    bit_id: String,
    location: String,
    descriptor: ModelAssetDescriptor,
    /// Its pinned Bit runs on the device's model host, so the placement never reads the file.
    hosted: bool,
}

impl StoredAsset {
    fn missing(&self, why: &str) -> String {
        format!(
            "{MODEL_ASSET_MISSING}: Bit {} needs model asset {} ({}), which {why}",
            self.bit_id,
            self.descriptor.file_name,
            self.descriptor.digest.store_key()
        )
    }
}

fn stored_assets(pinned: &PinnedPack) -> Result<Vec<StoredAsset>> {
    let pack = &pinned.pack;
    let hosted = pinned.hosted();
    let hashes: HashMap<&str, &str> = std::iter::once(&pack.bit)
        .chain(&pack.dependencies)
        .map(|bit| (bit.id.as_str(), bit.hash.as_str()))
        .collect();
    pack.assets
        .iter()
        .map(|asset| {
            let hash = hashes.get(asset.bit_id.as_str()).with_context(|| {
                format!(
                    "Model asset {} names Bit {}, which is not packaged",
                    asset.descriptor.file_name, asset.bit_id
                )
            })?;
            let location = format!("{hash}/{}", asset.descriptor.file_name);
            validate_artifact_relative_path(&location)?;
            Ok(StoredAsset {
                bit_id: asset.bit_id.clone(),
                location,
                descriptor: asset.descriptor.clone(),
                hosted,
            })
        })
        .collect()
}

/// A supervised placement reaches the model host through its router. The agent validating a
/// placement runs the host that the placement will reach.
fn routes_to_model_host(state: &FlowLikeState) -> bool {
    state.local_model_router.is_some() || crate::models::host::ModelHost::current().is_some()
}

/// A Bit the model host serves needs the host; other Local and MLX Bits need their runtime here.
fn check_model_runtime(
    bit: &Bit,
    capabilities: &CompletionModelCapabilities,
    hosted: bool,
    routing: bool,
) -> Result<()> {
    if hosted {
        ensure!(
            routing,
            "Bit {} runs on the device's model host, which this process cannot reach",
            bit.id
        );
        return Ok(());
    }
    ensure!(
        !bit.is_mlx_model() || capabilities.mlx,
        "Bit {} requires an Apple-silicon MLX runtime",
        bit.id
    );
    ensure!(
        !bit.try_to_provider()
            .is_some_and(|provider| provider.provider_name.eq_ignore_ascii_case("local"))
            || capabilities.local_server,
        "Bit {} requires the local llama-server runtime",
        bit.id
    );
    Ok(())
}

/// Model-store files are never copied into a placement. Before it starts, the agent links each
/// one it reads at its Bit store location, or a Linux sandbox binds it there read-only. Only a
/// starting placement has the store to look in.
fn verify_stored_assets(bit_store: Option<&Path>, read: &[&StoredAsset]) -> Result<()> {
    let Some(bit_store) = bit_store else {
        return Ok(());
    };
    for asset in read {
        let path = bit_store.join(&asset.location);
        let present = std::fs::metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == asset.descriptor.size);
        ensure!(
            present,
            "{}",
            asset.missing(&format!("is not at {}", path.display()))
        );
    }
    Ok(())
}

fn placement_stored_assets(config: &PlacementConfig) -> Result<Vec<StoredAsset>> {
    let mut stored = Vec::new();
    for pin in &config.bit_pins {
        stored.extend(stored_assets(&read_pinned(config, pin)?)?);
    }
    Ok(stored)
}

/// The Bit store a placement's runtime opens, `<data>/bits/<sha256 of its Bit pins>`.
pub(crate) fn placement_bit_store(config: &PlacementConfig, data: &Path) -> Result<PathBuf> {
    Ok(data
        .join("bits")
        .join(artifact_sha256(&serde_json::to_vec(&config.bit_pins)?)))
}

/// Agent side, before a placement starts: the placement references each of its model-store
/// assets, a missing one is acquired again download-first, and each one its workload reads
/// directly appears at its Bit store location without being copied.
pub(crate) fn prepare_model_assets(
    state_dir: &Path,
    config: &PlacementConfig,
    data: &Path,
    cancel: &CancellationToken,
) -> Result<()> {
    let Some(host) = crate::models::host::ModelHost::current() else {
        let needed = placement_stored_assets(config)?.len();
        ensure!(
            needed == 0,
            "{MODEL_ASSET_MISSING}: placement {} needs {needed} model assets, but the device's model store is not running",
            config.id
        );
        return Ok(());
    };
    let live = || -> Result<HashSet<String>> {
        let store = crate::state::StateStore::open(&state_dir.join("management.sqlite"))?;
        Ok(store
            .list_placements()?
            .into_iter()
            .map(|record| record.id)
            .collect())
    };
    tokio::runtime::Handle::try_current()
        .context("Prepare the model assets of a placement outside the agent's runtime")?
        .block_on(prepare_with(host.acquisition(), config, data, live, cancel))
}

async fn prepare_with(
    acquisition: &AcquisitionManager,
    config: &PlacementConfig,
    data: &Path,
    live: impl FnOnce() -> Result<HashSet<String>>,
    cancel: &CancellationToken,
) -> Result<()> {
    let stored = placement_stored_assets(config)?;
    reference_model_assets(acquisition.store(), &config.id, &stored, live)?;
    end_deploy_leases(acquisition.store(), &config.project_id, &stored)?;
    acquire_model_assets(acquisition, &stored, cancel).await?;
    expose_model_assets(acquisition.store(), config, data, &stored)
}

/// The placement's references now hold what its deploy leased until the start.
fn end_deploy_leases(store: &ModelStore, project: &str, stored: &[StoredAsset]) -> Result<()> {
    for asset in stored {
        store.end_lease(&asset.descriptor.digest, project)?;
    }
    Ok(())
}

/// The placement references exactly its assets, so the store keeps them; references of removed
/// placements go. `live` is read after the references, so a placement created meanwhile keeps
/// its own.
fn reference_model_assets(
    store: &ModelStore,
    placement: &str,
    stored: &[StoredAsset],
    live: impl FnOnce() -> Result<HashSet<String>>,
) -> Result<()> {
    let owner = AssetOwner::new(OwnerKind::Placement, placement)?;
    let wanted: HashSet<&ModelAssetDigest> = stored
        .iter()
        .map(|asset| &asset.descriptor.digest)
        .collect();
    for digest in &wanted {
        store.add_ref(digest, &owner)?;
    }
    let held = store.with_db(|db| db.refs_of_kind(OwnerKind::Placement))?;
    release_stale_references(store, &owner, &wanted, held, &live()?)
}

/// Drops this placement's references to digests it no longer pins, and the references of
/// placements that no longer exist.
fn release_stale_references(
    store: &ModelStore,
    owner: &AssetOwner,
    wanted: &HashSet<&ModelAssetDigest>,
    held: Vec<(ModelAssetDigest, AssetOwner)>,
    live: &HashSet<String>,
) -> Result<()> {
    for (digest, holder) in held {
        let stale = if holder == *owner {
            !wanted.contains(&digest)
        } else {
            !live.contains(&holder.id)
        };
        if stale {
            store.remove_ref(&digest, &holder)?;
        }
    }
    Ok(())
}

/// A placement never starts without its assets. A missing one, evicted or lost in a reset, is
/// acquired again from its sources; a push from a controller completes the same job.
async fn acquire_model_assets(
    acquisition: &AcquisitionManager,
    stored: &[StoredAsset],
    cancel: &CancellationToken,
) -> Result<()> {
    let mut requested = HashSet::new();
    let mut waiting = Vec::new();
    for asset in stored
        .iter()
        .filter(|asset| requested.insert(&asset.descriptor.digest))
    {
        let status = acquisition
            .ensure(&asset.descriptor, None)
            .with_context(|| asset.missing("cannot be requested again"))?;
        if status.state != ModelAssetState::Present {
            waiting.push(asset);
        }
    }
    for asset in waiting {
        settle(acquisition, asset, cancel).await?;
    }
    Ok(())
}

/// Waits until the asset's job stops by itself; only a present asset lets the placement start.
async fn settle(
    acquisition: &AcquisitionManager,
    asset: &StoredAsset,
    cancel: &CancellationToken,
) -> Result<()> {
    let state = tokio::select! {
        biased;
        () = cancel.cancelled() => bail!(
            "Placement start cancelled while model asset {} was acquired",
            asset.descriptor.file_name
        ),
        state = acquisition.settled(&asset.descriptor.digest) => state?,
    };
    ensure!(
        state == ModelAssetState::Present,
        "{}",
        asset.missing(&format!(
            "could not be acquired again: {}",
            serde_json::to_string(&state)?
        ))
    );
    Ok(())
}

/// Each asset the workload reads appears at its Bit store location: a symlink to the immutable
/// blob, or for a Linux sandbox an empty mount point the blob is bound over read-only.
fn expose_model_assets(
    store: &ModelStore,
    config: &PlacementConfig,
    data: &Path,
    stored: &[StoredAsset],
) -> Result<()> {
    let read: Vec<&StoredAsset> = stored.iter().filter(|asset| !asset.hosted).collect();
    if read.is_empty() {
        return Ok(());
    }
    let bit_store = placement_bit_store(config, data)?;
    crate::runtime::private_runtime_directory(&data.join("bits"))?;
    crate::runtime::private_runtime_directory(&bit_store)?;
    let sandboxed = crate::isolation::sandboxed(config);
    for asset in read {
        let target = asset_location(&bit_store, &asset.location)?;
        expose(store, asset, &target, sandboxed)?;
    }
    Ok(())
}

fn expose(store: &ModelStore, asset: &StoredAsset, target: &Path, sandboxed: bool) -> Result<()> {
    if sandboxed {
        return mount_point(target);
    }
    link_blob(&stored_blob(store, asset)?, target)
}

fn stored_blob(store: &ModelStore, asset: &StoredAsset) -> Result<PathBuf> {
    store
        .path_of(&asset.descriptor.digest)?
        .with_context(|| asset.missing("left the model store"))
}

/// A location below the Bit store, with its directories created private.
fn asset_location(bit_store: &Path, location: &str) -> Result<PathBuf> {
    validate_artifact_relative_path(location)?;
    let mut path = bit_store.to_path_buf();
    let mut parts = location.split('/').peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() {
            crate::runtime::private_runtime_directory(&path)?;
        }
    }
    Ok(path)
}

#[cfg(not(unix))]
fn link_blob(_: &Path, target: &Path) -> Result<()> {
    bail!(
        "Expose a model asset at {}: placements need Unix symlinks",
        target.display()
    )
}

#[cfg(unix)]
fn link_blob(blob: &Path, target: &Path) -> Result<()> {
    if links_to(target, blob)? {
        return Ok(());
    }
    let temporary = target.with_file_name(format!(".{}.link", uuid::Uuid::new_v4()));
    std::os::unix::fs::symlink(blob, &temporary)?;
    if let Err(error) = std::fs::rename(&temporary, target) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("Expose a model asset at {}", target.display()));
    }
    Ok(())
}

/// Whether `target` links to `blob` already; a directory in its place is refused.
#[cfg(unix)]
fn links_to(target: &Path, blob: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => bail!(
            "Expose a model asset at {}: a directory is in the way",
            target.display()
        ),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Ok(std::fs::read_link(target)? == blob)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// An empty private file the sandbox mounts the blob over; anything else there is replaced.
fn mount_point(target: &Path) -> Result<()> {
    if is_mount_point(target)? {
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(target)
        .with_context(|| format!("Prepare a model asset mount point at {}", target.display()))?;
    Ok(())
}

/// Whether `target` is an empty file already; anything else but a directory is removed.
fn is_mount_point(target: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_file() && metadata.len() == 0 => Ok(true),
        Ok(metadata) if metadata.is_dir() => bail!(
            "Prepare a model asset mount point at {}: a directory is in the way",
            target.display()
        ),
        Ok(_) => std::fs::remove_file(target)
            .map(|()| false)
            .with_context(|| format!("Clear the model asset mount point at {}", target.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// What a Linux sandbox binds read-only into a placement: the model-store blob of each asset its
/// workload reads directly, over the mount point `prepare_model_assets` left for it.
pub fn model_asset_binds(config: &PlacementConfig, data: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let stored = placement_stored_assets(config)?;
    let read: Vec<&StoredAsset> = stored.iter().filter(|asset| !asset.hosted).collect();
    if read.is_empty() {
        return Ok(Vec::new());
    }
    let host = crate::models::host::ModelHost::current().with_context(|| {
        format!(
            "{MODEL_ASSET_MISSING}: placement {} reads model assets, but the device's model store is not running",
            config.id
        )
    })?;
    let bit_store = placement_bit_store(config, data)?;
    read.into_iter()
        .map(|asset| {
            Ok((
                stored_blob(host.store(), asset)?,
                bit_store.join(&asset.location),
            ))
        })
        .collect()
}

pub(crate) async fn register_packages(
    config: &PlacementConfig,
    app: &App,
    state: &Arc<FlowLikeState>,
) -> Result<()> {
    if config.package_pins.is_empty() {
        return Ok(());
    }
    // Only portable WASM bytes enter this engine. Imported AOT files are never
    // deserialized, including through a writable compilation cache.
    // Wasmtime 48's Mach handler aborts on interrupted receives while child
    // processes exit. Unix signal traps support the agent's child supervision.
    let engine = Arc::new(WasmEngine::new(
        WasmConfig::production()
            .without_cache()
            .with_macos_signal_traps(),
    )?);
    engine.start_epoch_ticker();
    let mut names: HashSet<String> = state
        .node_registry
        .read()
        .await
        .get_nodes()?
        .into_iter()
        .map(|node| node.name)
        .collect();
    let mut prepared: Vec<Arc<dyn NodeLogic>> = Vec::new();
    let mut total_bytes = 0usize;
    for pin in &config.package_pins {
        pin.validate()?;
        ensure!(
            app.packages.get(&pin.package_id) == Some(&pin.version),
            "Package {} does not match the project's version",
            pin.package_id
        );
        let manifest_bytes = read_bounded(
            &config.project_path,
            &["packages", &pin.package_id, &pin.version, "manifest.json"],
            1024 * 1024,
        )?;
        ensure!(
            artifact_sha256(&manifest_bytes) == pin.manifest_sha256,
            "Package manifest digest differs"
        );
        let manifest: flow_like_wasm::manifest::PackageManifest =
            serde_json::from_slice(&manifest_bytes).context("Invalid packaged WASM manifest")?;
        ensure!(
            manifest.validate().is_ok(),
            "Invalid packaged WASM manifest"
        );
        ensure!(
            manifest.id == pin.package_id && manifest.version == pin.version,
            "Package manifest identity differs"
        );
        let bytes = read_bounded(
            &config.project_path,
            &["packages", &pin.package_id, &pin.version, "module.wasm"],
            64 * 1024 * 1024,
        )?;
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .context("Package size overflow")?;
        ensure!(
            total_bytes <= 256 * 1024 * 1024,
            "Packaged WASM exceeds 256 MiB"
        );
        ensure!(
            artifact_sha256(&bytes) == pin.wasm_sha256,
            "Package WASM digest differs"
        );
        ensure!(
            bytes.starts_with(b"\0asm"),
            "Package must contain portable WASM bytes"
        );
        let mut security: WasmSecurityConfig = manifest.permissions.to_security_config();
        ensure!(
            security.limits.memory_limit <= 512 * 1024 * 1024
                && security.limits.timeout <= Duration::from_secs(300),
            "Package exceeds standalone memory or invocation time limits"
        );
        security.execution_environment = ExecutionEnvironment::Server;
        let loaded = engine.load_auto(&bytes).await?;
        let definitions = tokio::time::timeout(Duration::from_secs(30), async {
            let mut instance = loaded.instantiate(&engine, security.for_metadata()).await?;
            instance.call_get_nodes().await
        })
        .await
        .context("Package metadata initialization timed out")??;
        ensure!(
            !definitions.is_empty() && definitions.len() <= 512,
            "Invalid package node count"
        );
        for definition in definitions {
            ensure!(
                !definition.name.is_empty()
                    && definition.name.len() <= 256
                    && !definition.name.chars().any(char::is_control)
                    && definition.pins.len() <= 512
                    && names.insert(definition.name.clone()),
                "Package node name collides with another node or exceeds its bounds"
            );
            let mut node_security =
                WasmSecurityConfig::from_node_permissions(&definition.permissions)
                    .with_package_settings(&security);
            node_security.execution_environment = ExecutionEnvironment::Server;
            prepared.push(Arc::new(
                WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    node_security,
                    definition,
                )
                .with_package_id(pin.package_id.clone()),
            ));
        }
    }
    state.node_registry.write().await.push_nodes(prepared);
    Ok(())
}

pub(crate) fn validate_board_packages(config: &PlacementConfig, board: &Board) -> Result<()> {
    for node in board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
    {
        if let Some(package) = &node.wasm {
            ensure!(
                config
                    .package_pins
                    .iter()
                    .any(|pin| pin.package_id == package.package_id),
                "Node {} requires an explicit package pin for {}",
                node.name,
                package.package_id
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::ProjectPackagePin;
    use flow_like_runtime::bit::Metadata;
    use flow_like_wasm::manifest::PackageManifest;

    fn module(name: &str) -> Vec<u8> {
        let definition = serde_json::json!([{
            "name":name,"friendly_name":"Pinned node","description":"",
            "category":"Tests","pins":[]
        }])
        .to_string();
        let escaped = definition
            .as_bytes()
            .iter()
            .map(|b| format!("\\{b:02x}"))
            .collect::<String>();
        wat::parse_str(format!(
            r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{escaped}")
            (func (export "get_nodes") (result i64) i64.const {})
            (func (export "run") (param i32 i32) (result i64) i64.const 0))"#,
            definition.len()
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn packages_verify_both_pins_and_register_without_replacing_native_nodes() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::runtime::offline_state(directory.path(), None)
            .await
            .unwrap();
        let mut app = App::new(
            Some("project".into()),
            Metadata::default(),
            vec![],
            state.clone(),
        )
        .await
        .unwrap();
        app.packages
            .insert("com.example.pinned".into(), "1.0.0".into());
        let package_dir = directory.path().join("packages/com.example.pinned/1.0.0");
        std::fs::create_dir_all(&package_dir).unwrap();
        let manifest = serde_json::to_vec(&PackageManifest::new(
            "com.example.pinned",
            "Pinned",
            "1.0.0",
            "",
        ))
        .unwrap();
        let bytes = module("standalone_pinned_wasm");
        std::fs::write(package_dir.join("manifest.json"), &manifest).unwrap();
        std::fs::write(package_dir.join("module.wasm"), &bytes).unwrap();
        let mut config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"project","deployment_id":"deployment","revision":"r1",
            "source":"offline","project_path":directory.path(),"events":[]
        }))
        .unwrap();
        config.package_pins.push(ProjectPackagePin {
            package_id: "com.example.pinned".into(),
            version: "1.0.0".into(),
            wasm_sha256: artifact_sha256(&bytes),
            manifest_sha256: artifact_sha256(&manifest),
        });
        let mut invalid = config.clone();
        invalid.package_pins[0].manifest_sha256 = "0".repeat(64);
        assert!(register_packages(&invalid, &app, &state).await.is_err());
        invalid = config.clone();
        invalid.package_pins[0].wasm_sha256 = "0".repeat(64);
        assert!(register_packages(&invalid, &app, &state).await.is_err());
        register_packages(&config, &app, &state).await.unwrap();
        let node = state
            .node_registry
            .read()
            .await
            .get_node("standalone_pinned_wasm")
            .unwrap();
        assert_eq!(node.wasm.unwrap().package_id, "com.example.pinned");
        // A duplicate cannot silently change the implementation already bound to a name.
        assert!(register_packages(&config, &app, &state).await.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn dependency_paths_reject_links_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("data"), b"12345").unwrap();
        assert!(read_bounded(directory.path(), &["data"], 4).is_err());
        assert!(read_bounded(directory.path(), &["..", "data"], 10).is_err());
        std::os::unix::fs::symlink("data", directory.path().join("link")).unwrap();
        assert!(read_bounded(directory.path(), &["link"], 10).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn selected_bits_materialize_privately_once_and_recover_partial_copies() -> Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let source = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        std::fs::create_dir_all(source.path().join("bits/hash/nested"))?;
        std::fs::write(source.path().join("bits/hash/nested/model"), b"weights")?;
        let destination = data.path().join("bits");
        crate::runtime::private_runtime_directory(&destination)?;
        let assets = vec![ProjectArtifactFile {
            path: "bits/hash/nested/model".into(),
            size: 7,
            sha256: artifact_sha256(b"weights"),
        }];
        let controls = data.path().join("bit-imports");
        crate::runtime::private_runtime_directory(&controls)?;
        let partial = controls.join(format!(
            "{}.partial",
            artifact_sha256(assets[0].path.as_bytes())
        ));
        crate::vault::write_new_private(&partial, b"interrupted")?;
        // The source is usable without any write permission, including its
        // directories. Only the placement's mutable store receives new files.
        for path in [
            source.path().join("bits/hash/nested"),
            source.path().join("bits/hash"),
            source.path().join("bits"),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o500))?;
        }
        prepare_bit_assets(source.path(), &assets, Some(&destination), true)?;
        assert!(!partial.exists());
        let target = destination.join("hash/nested/model");
        let before = std::fs::metadata(&target)?;
        assert_eq!(before.mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(destination.join("hash/nested"))?.mode() & 0o777,
            0o700
        );
        prepare_bit_assets(source.path(), &assets, Some(&destination), true)?;
        assert_eq!(std::fs::metadata(&target)?.ino(), before.ino());
        assert_eq!(std::fs::read(&target)?, b"weights");
        let marker = controls.join(format!(
            "{}.verified",
            artifact_sha256(assets[0].path.as_bytes())
        ));
        assert!(marker.is_file());
        // An in-place change invalidates the marker and fails verification.
        std::fs::write(&target, b"WEIGHTS")?;
        assert!(prepare_bit_assets(source.path(), &assets, Some(&destination), true).is_err());
        std::fs::write(&target, b"weights")?;
        prepare_bit_assets(source.path(), &assets, Some(&destination), true)?;
        // A workload that can write the store can also forge the marker, so a
        // sandboxed start never trusts one.
        std::fs::write(&target, b"WEIGHTS")?;
        let forged = verification_stamp(&std::fs::metadata(&target)?, &assets[0]).unwrap();
        std::fs::write(&marker, forged)?;
        prepare_bit_assets(source.path(), &assets, Some(&destination), true)?;
        assert!(prepare_bit_assets(source.path(), &assets, Some(&destination), false).is_err());
        std::fs::write(&target, b"weights")?;
        prepare_bit_assets(source.path(), &assets, Some(&destination), false)?;
        std::fs::remove_file(&target)?;
        std::os::unix::fs::symlink(source.path().join("bits/hash/nested/model"), &target)?;
        assert!(prepare_bit_assets(source.path(), &assets, Some(&destination), true).is_err());
        for path in [
            source.path().join("bits/hash/nested"),
            source.path().join("bits/hash"),
            source.path().join("bits"),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn packaged_bits_verify_files_and_resolve_dependencies_without_a_hub() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::runtime::offline_state(directory.path(), None)
            .await
            .unwrap();
        let app = App::new(
            Some("project".into()),
            Metadata::default(),
            vec!["model".into()],
            state.clone(),
        )
        .await
        .unwrap();
        let dependency = Bit {
            id: "weights".into(),
            hash: "source-identity".into(),
            file_name: Some("model.bin".into()),
            size: Some(4),
            download_link: Some("https://unreachable.invalid/model".into()),
            ..Bit::default()
        };
        let mut pack = PackagedBit {
            bit: Bit {
                id: "model".into(),
                dependencies: vec![dependency.id.clone()],
                hub: "unreachable.invalid".into(),
                ..Bit::default()
            },
            dependencies: vec![dependency],
            artifacts: vec![ProjectArtifactFile {
                path: "bits/source-identity/model.bin".into(),
                size: 4,
                sha256: artifact_sha256(b"data"),
            }],
            assets: vec![],
        };
        let mut config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"project","deployment_id":"deployment","revision":"r1",
            "source":"offline","project_path":directory.path(),"events":[]
        }))
        .unwrap();
        std::fs::create_dir_all(directory.path().join("bits/metadata")).unwrap();
        std::fs::create_dir_all(directory.path().join("bits/source-identity")).unwrap();
        let metadata = serde_json::to_vec(&pack).unwrap();
        std::fs::write(directory.path().join("bits/metadata/model.json"), &metadata).unwrap();
        config
            .bit_pins
            .push(flow_like_device_protocol::ProjectBitPin {
                bit_id: "model".into(),
                metadata_sha256: artifact_sha256(&metadata),
            });
        let mut profile = Profile::default();
        profile.hub = "api.flow-like.com".into();
        profile.hubs.push("https://another.invalid".into());
        assert!(
            hydrate_bits(&config, &app, &state, &mut profile, true)
                .await
                .is_err()
        );
        std::fs::write(
            directory.path().join("bits/source-identity/model.bin"),
            b"oops",
        )
        .unwrap();
        assert!(
            hydrate_bits(&config, &app, &state, &mut profile, true)
                .await
                .is_err()
        );
        std::fs::write(
            directory.path().join("bits/source-identity/model.bin"),
            b"data",
        )
        .unwrap();
        hydrate_bits(&config, &app, &state, &mut profile, true)
            .await
            .unwrap();
        assert_eq!(profile.bits, vec!["model"]);
        assert!(profile.hub.is_empty() && profile.hubs.is_empty());
        assert_eq!(resolve_pinned_bit(&profile, "model").unwrap().id, "model");
        assert_eq!(
            resolve_pinned_bit(&profile, "unreachable.invalid:model")
                .unwrap()
                .id,
            "model"
        );
        assert!(resolve_pinned_bit(&profile, "missing").is_err());
        assert!(resolve_pinned_bit(&profile, "https://other.invalid:model").is_err());
        let model = profile
            .get_bit(
                "model".into(),
                Some("unreachable.invalid".into()),
                state.http_client.clone(),
            )
            .await
            .unwrap();
        let resolved = model.pack(state.clone()).await.unwrap();
        assert_eq!(resolved.bits.len(), 2);
        assert!(resolved.bits.iter().all(|bit| bit.download_link.is_none()));
        assert!(resolved.bits.iter().any(|bit| bit.id == "weights"));
        pack.artifacts[0].path = "bits/unselected/secret.bin".into();
        assert!(verify_pack_metadata(&pack, "model").is_err());
        pack.artifacts[0].path = "bits/source-identity/model.bin".into();
        pack.dependencies[0].dependencies.push("missing".into());
        assert!(verify_pack_metadata(&pack, "model").is_err());
    }

    mod model_store {
        use super::*;
        use crate::models::{
            acquire::AcquisitionConfig,
            fetch::{AddressPolicy, Fetcher},
            router::hosts,
            store::ModelStoreConfig,
            test_server::{Origin, serving},
        };
        use axum::Router;
        use flow_like_device_protocol::{DigestAlgorithm, PackagedBitMetadataV2};
        use flow_like_runtime::bit::{BitModelClassification, BitTypes};

        /// Refused by the production address policy at once, so a download fails offline.
        const UNREACHABLE: &str = "https://127.0.0.1:9/model.onnx";
        const WEIGHTS: &[u8] = b"detector weights";

        fn digest(bytes: &[u8]) -> ModelAssetDigest {
            ModelAssetDigest {
                algorithm: DigestAlgorithm::Blake3,
                hex: blake3::hash(bytes).to_hex().to_string(),
            }
        }

        /// A placement of `root` pinning an object detector whose weights are a model-store
        /// asset at `source`, which its workload reads directly.
        fn detector(root: &Path, source: &str) -> (PlacementConfig, ModelAssetDescriptor) {
            let asset = ModelAssetDescriptor {
                digest: digest(WEIGHTS),
                size: WEIGHTS.len() as u64,
                file_name: "model.onnx".into(),
                sources: vec![source.into()],
            };
            let bit = Bit {
                id: "detector".into(),
                bit_type: BitTypes::ObjectDetection,
                hash: "detector-hash".into(),
                file_name: Some("model.onnx".into()),
                size: Some(asset.size),
                download_link: Some("https://cdn.flow-like.com/detector.onnx".into()),
                ..Bit::default()
            };
            let metadata = PackagedBitMetadata::V2(PackagedBitMetadataV2 {
                version: Default::default(),
                bit: serde_json::to_value(&bit).unwrap(),
                dependencies: vec![],
                assets: vec![PackagedBitAsset {
                    bit_id: bit.id.clone(),
                    descriptor: asset.clone(),
                }],
                artifacts: vec![],
            });
            let metadata = serde_json::to_vec(&metadata).unwrap();
            std::fs::create_dir_all(root.join("bits/metadata")).unwrap();
            std::fs::write(root.join("bits/metadata/detector.json"), &metadata).unwrap();
            let mut config: PlacementConfig = serde_json::from_value(serde_json::json!({
                "id":"placement","project_id":"project","deployment_id":"deployment","revision":"r1",
                "source":"offline","project_path":root,"events":[]
            }))
            .unwrap();
            config.bit_pins.push(ProjectBitPin {
                bit_id: bit.id,
                metadata_sha256: artifact_sha256(&metadata),
            });
            (config, asset)
        }

        fn publish(store: &ModelStore, asset: &ModelAssetDescriptor, bytes: &[u8]) {
            let mut staged = store.open_partial(&asset.digest).unwrap();
            staged.write_all(bytes).unwrap();
            store.publish(asset).unwrap();
        }

        fn placement(id: &str) -> AssetOwner {
            AssetOwner::new(OwnerKind::Placement, id).unwrap()
        }

        /// A detector placement with its own project, agent state and placement data.
        struct Fixture {
            _project: tempfile::TempDir,
            _state: tempfile::TempDir,
            data: tempfile::TempDir,
            config: PlacementConfig,
            asset: ModelAssetDescriptor,
            acquisition: AcquisitionManager,
        }

        impl Fixture {
            fn new() -> Self {
                let fetcher = Fetcher::new(AddressPolicy::global_only()).unwrap();
                Self::with(fetcher, UNREACHABLE)
            }

            fn with(fetcher: Fetcher, source: &str) -> Self {
                let (project, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
                let (config, asset) = detector(project.path(), source);
                let store = ModelStore::open(state.path(), ModelStoreConfig::default()).unwrap();
                let acquisition = AcquisitionManager::start(
                    Arc::new(store),
                    fetcher,
                    AcquisitionConfig::default(),
                )
                .unwrap();
                Self {
                    _project: project,
                    _state: state,
                    data: tempfile::tempdir().unwrap(),
                    config,
                    asset,
                    acquisition,
                }
            }

            /// The agent's preparation before a start, with these placements still installed.
            async fn start(&self, live: &[&str]) -> Result<()> {
                let live: HashSet<String> = live.iter().map(|id| id.to_string()).collect();
                let cancel = CancellationToken::new();
                let data = self.data.path();
                prepare_with(&self.acquisition, &self.config, data, || Ok(live), &cancel).await
            }

            fn link(&self) -> PathBuf {
                let bit_store = placement_bit_store(&self.config, self.data.path()).unwrap();
                bit_store.join("detector-hash/model.onnx")
            }
        }

        #[tokio::test]
        #[cfg(unix)]
        async fn a_placement_references_its_stored_assets_and_reads_them_through_links() {
            let fixture = Fixture::new();
            let store = fixture.acquisition.store();
            publish(store, &fixture.asset, WEIGHTS);
            let other = ModelAssetDescriptor {
                digest: digest(b"other"),
                size: 5,
                ..fixture.asset.clone()
            };
            publish(store, &other, b"other");
            for holder in ["placement", "removed", "kept"] {
                store.add_ref(&other.digest, &placement(holder)).unwrap();
            }
            let pruned = digest(b"neither stored nor requested");
            store.add_ref(&pruned, &placement("removed")).unwrap();
            store.lease(&fixture.asset.digest, "project").unwrap();
            fixture.start(&["placement", "kept"]).await.unwrap();
            let pinned = store.refs(&fixture.asset.digest).unwrap();
            assert_eq!(pinned, vec![placement("placement")]);
            assert_eq!(store.refs(&other.digest).unwrap(), vec![placement("kept")]);
            assert!(store.refs(&pruned).unwrap().is_empty());
            let blob = store.path_of(&fixture.asset.digest).unwrap().unwrap();
            assert_eq!(std::fs::read_link(fixture.link()).unwrap(), blob);
            assert_eq!(std::fs::read(fixture.link()).unwrap(), WEIGHTS);
            fixture.start(&["placement"]).await.unwrap();
            assert_eq!(std::fs::read_link(fixture.link()).unwrap(), blob);
        }

        #[tokio::test]
        #[cfg(unix)]
        async fn a_missing_asset_is_acquired_again_before_the_placement_starts() {
            let fixture = Fixture::new();
            let (acquisition, digest) = (&fixture.acquisition, &fixture.asset.digest);
            let failed = format!("{:#}", fixture.start(&["placement"]).await.unwrap_err());
            assert!(failed.starts_with(MODEL_ASSET_MISSING), "{failed}");
            assert!(failed.contains("egress_blocked"), "{failed}");
            let jobs = acquisition.jobs();
            assert_eq!(
                jobs.len(),
                1,
                "the asset is downloaded from its sources first"
            );
            assert_eq!(jobs[0].asset.sources, vec![UNREACHABLE]);
            let pinned = acquisition.store().refs(digest).unwrap();
            assert_eq!(pinned, vec![placement("placement")]);

            acquisition.begin_push(digest, false).await.unwrap();
            let pushed = acquisition.push_chunk(digest, 0, WEIGHTS).await.unwrap();
            assert_eq!(pushed, ModelAssetState::Present);
            fixture.start(&["placement"]).await.unwrap();
            assert_eq!(std::fs::read(fixture.link()).unwrap(), WEIGHTS);

            let blob = acquisition.store().path_of(digest).unwrap().unwrap();
            std::fs::remove_file(blob).unwrap();
            let failed = format!("{:#}", fixture.start(&["placement"]).await.unwrap_err());
            assert!(failed.starts_with(MODEL_ASSET_MISSING), "{failed}");
            assert_eq!(
                acquisition.jobs().len(),
                1,
                "a lost blob is downloaded again"
            );
        }

        #[tokio::test]
        #[cfg(unix)]
        async fn a_lost_asset_is_downloaded_again_from_its_source_at_start() {
            let weights = Arc::new(WEIGHTS.to_vec());
            let routes = Router::new().route("/model.onnx", serving(weights));
            let origin = Origin::start(routes).await;
            let fixture = Fixture::with(origin.fetcher(), &origin.url("/model.onnx"));
            fixture.start(&["placement"]).await.unwrap();
            assert_eq!(std::fs::read(fixture.link()).unwrap(), WEIGHTS);
            assert_eq!(origin.hits.to("/model.onnx").len(), 1);

            let store = fixture.acquisition.store();
            let blob = store.path_of(&fixture.asset.digest).unwrap().unwrap();
            std::fs::remove_file(&blob).unwrap();
            fixture.start(&["placement"]).await.unwrap();
            assert_eq!(std::fs::read(fixture.link()).unwrap(), WEIGHTS);
            assert_eq!(origin.hits.to("/model.onnx").len(), 2);
        }

        #[tokio::test]
        #[cfg(unix)]
        async fn hydrate_reads_stored_assets_in_place_and_names_missing_ones() {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let state = crate::runtime::offline_state(root, None).await.unwrap();
            let bits = vec!["detector".to_owned()];
            let app = App::new(
                Some("project".into()),
                Metadata::default(),
                bits,
                state.clone(),
            );
            let app = app.await.unwrap();
            let (config, _) = detector(root, UNREACHABLE);
            let mut profile = Profile::default();
            let validated = hydrate_bits(&config, &app, &state, &mut profile, false).await;
            validated.unwrap();
            let missing = hydrate_bits(&config, &app, &state, &mut profile, true).await;
            let missing = format!("{:#}", missing.unwrap_err());
            assert!(missing.starts_with(MODEL_ASSET_MISSING), "{missing}");

            let blob = root.join("blob");
            std::fs::write(&blob, WEIGHTS).unwrap();
            let link = root.join("bits/detector-hash/model.onnx");
            std::fs::create_dir_all(root.join("bits/detector-hash")).unwrap();
            std::os::unix::fs::symlink(&blob, &link).unwrap();
            let started = hydrate_bits(&config, &app, &state, &mut profile, true).await;
            started.unwrap();
            let linked = std::fs::symlink_metadata(&link).unwrap();
            assert!(
                linked.file_type().is_symlink(),
                "a stored asset is never copied"
            );
            assert_eq!(profile.bits, vec!["detector"]);
            let bit = resolve_pinned_bit(&profile, "detector").unwrap();
            assert!(bit.download_link.is_none());
            std::fs::write(&blob, b"truncated").unwrap();
            let truncated = hydrate_bits(&config, &app, &state, &mut profile, true).await;
            assert!(truncated.is_err());
        }

        #[test]
        #[cfg(unix)]
        fn links_and_mount_points_replace_each_other_but_never_a_directory() {
            let directory = tempfile::tempdir().unwrap();
            let blob = directory.path().join("blob");
            std::fs::write(&blob, b"weights").unwrap();
            let target = directory.path().join("model.onnx");
            let length = |path: &Path| std::fs::symlink_metadata(path).unwrap().len();
            mount_point(&target).unwrap();
            assert_eq!(length(&target), 0);
            link_blob(&blob, &target).unwrap();
            link_blob(&blob, &target).unwrap();
            assert_eq!(std::fs::read_link(&target).unwrap(), blob);
            mount_point(&target).unwrap();
            assert!(std::fs::symlink_metadata(&target).unwrap().is_file());
            assert_eq!(length(&target), 0);
            std::fs::write(&target, b"forged").unwrap();
            mount_point(&target).unwrap();
            assert_eq!(length(&target), 0);
            let nested = directory.path().join("nested");
            std::fs::create_dir(&nested).unwrap();
            assert!(link_blob(&blob, &nested).is_err());
            assert!(mount_point(&nested).is_err());
        }

        #[test]
        fn hosted_bits_need_the_model_host_and_other_local_bits_their_runtime() {
            let llm = |provider: &str| Bit {
                id: "qwen".into(),
                bit_type: BitTypes::Llm,
                file_name: Some("qwen.gguf".into()),
                parameters: serde_json::json!({"context_length": 8192,
                    "model_classification": BitModelClassification::default(),
                    "provider": {"provider_name": provider, "model_id": null, "version": null,
                                 "params": null}}),
                ..Bit::default()
            };
            let local = llm("Local");
            assert!(hosts(&local));
            let none = CompletionModelCapabilities::default();
            assert!(check_model_runtime(&local, &none, true, true).is_ok());
            assert!(check_model_runtime(&local, &none, true, false).is_err());
            assert!(check_model_runtime(&local, &none, false, true).is_err());
            let server = CompletionModelCapabilities {
                local_server: true,
                ..none
            };
            assert!(check_model_runtime(&local, &server, false, false).is_ok());
            let mlx = llm("MLX");
            assert!(mlx.is_mlx_model());
            assert_eq!(
                hosts(&mlx),
                cfg!(all(target_os = "macos", target_arch = "aarch64"))
            );
            assert!(check_model_runtime(&mlx, &server, false, true).is_err());
            assert!(!hosts(&llm("openai")));
            let embedding = |file: &str| Bit {
                id: "minilm".into(),
                bit_type: BitTypes::Embedding,
                file_name: Some(file.into()),
                parameters: serde_json::json!({"languages": [], "vector_length": 384,
                    "input_length": 512, "prefix": {"query": "", "paragraph": ""},
                    "pooling": "CLS", "provider": {"provider_name": "Local"}}),
                ..Bit::default()
            };
            assert!(hosts(&embedding("model.onnx")));
            assert!(hosts(&embedding("nomic.Q4_K_M.GGUF")));
            assert!(!hosts(&embedding("model.safetensors")));
        }
    }
}
