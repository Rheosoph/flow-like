use crate::config::PlacementConfig;
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{ProjectArtifactFile, artifact_sha256, validate_artifact_digest};
use flow_like_runtime::{
    app::App,
    bit::{Bit, BitPack},
    flow::{
        board::Board,
        execution::{ExecutionEnvironment, context::ExecutionContext},
        node::{Node, NodeLogic},
    },
    profile::{Profile, ProfileCustomBit},
    state::FlowLikeState,
    utils::compression::compress_to_file_json,
};
use flow_like_wasm::{WasmConfig, WasmEngine, WasmNodeLogic, WasmSecurityConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

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
}

fn bit_asset_path(bit: &Bit) -> Result<Option<String>> {
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
    flow_like_device_protocol::validate_artifact_relative_path(&path)?;
    ensure!(
        bit.size
            .is_some_and(|size| size > 0 && size <= 4 * 1024 * 1024 * 1024),
        "Bit artifact size is missing or exceeds its limit"
    );
    Ok(Some(path))
}

fn verify_pack_metadata(pack: &PackagedBit, expected: &str) -> Result<()> {
    ensure!(pack.bit.id == expected, "Bit metadata identity differs");
    ensure!(
        pack.dependencies.len() <= 2048 && pack.artifacts.len() <= 2049,
        "Too many Bit dependencies"
    );
    let mut ids = HashSet::new();
    let mut assets = BTreeMap::new();
    for bit in std::iter::once(&pack.bit).chain(&pack.dependencies) {
        crate::config::validate_id("Bit", &bit.id)?;
        ensure!(ids.insert(&bit.id), "Duplicate Bit dependency");
        if let Some(path) = bit_asset_path(bit)? {
            if let Some(size) = assets.insert(path, bit.size.unwrap()) {
                ensure!(
                    Some(size) == bit.size,
                    "Bit artifacts disagree about file size"
                );
            }
        }
    }
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

fn verify_asset(mut file: File, asset: &ProjectArtifactFile) -> Result<()> {
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
) -> Result<()> {
    for asset in assets {
        let parts = asset.path.split('/').collect::<Vec<_>>();
        let source = open_bounded(root, &parts, asset.size)?;
        let Some(destination) = destination else {
            verify_asset(source, asset)?;
            continue;
        };
        ensure!(
            parts.first() == Some(&"bits"),
            "Invalid selected Bit asset prefix"
        );
        if root.join("bits").canonicalize()? == destination.canonicalize()? {
            verify_asset(source, asset)?;
            continue;
        }
        materialize_bit_asset(source, destination, &parts[1..], asset)?;
    }
    Ok(())
}

fn materialize_bit_asset(
    mut source: File,
    destination: &Path,
    parts: &[&str],
    asset: &ProjectArtifactFile,
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
            return verify_asset(open_bounded(destination, parts, asset.size)?, asset);
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
    for pin in &config.bit_pins {
        pin.validate()?;
        let bytes = read_bounded(
            &config.project_path,
            &["bits", "metadata", &format!("{}.json", pin.bit_id)],
            16 * 1024 * 1024,
        )?;
        metadata_bytes += bytes.len();
        ensure!(
            metadata_bytes <= 64 * 1024 * 1024,
            "Selected Bit metadata exceeds 64 MiB"
        );
        ensure!(
            artifact_sha256(&bytes) == pin.metadata_sha256,
            "Bit metadata digest differs"
        );
        let pack: PackagedBit =
            serde_json::from_slice(&bytes).context("Invalid packaged Bit metadata")?;
        verify_pack_metadata(&pack, &pin.bit_id)?;
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
    let bit_store = FlowLikeState::bit_store(state).await?;
    let destination = if materialize {
        match &bit_store {
            flow_like_storage::files::store::FlowLikeStore::Local(store) => {
                Some(store.directory_to_filesystem(&flow_like_storage::Path::from(""))?)
            }
            _ if assets.is_empty() => None,
            _ => anyhow::bail!("Selected Bit assets require a private local runtime store"),
        }
    } else {
        None
    };
    if !assets.is_empty() {
        let root = config.project_path.clone();
        let selected = assets.into_values().collect::<Vec<_>>();
        let target = destination.clone();
        tokio::task::spawn_blocking(move || {
            prepare_bit_assets(&root, &selected, target.as_deref())
        })
        .await??;
    }
    let capabilities = FlowLikeState::completion_model_capabilities(state).await;
    for bit in bits.values_mut() {
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
        prepare_bit_assets(source.path(), &assets, Some(&destination))?;
        assert!(!partial.exists());
        let target = destination.join("hash/nested/model");
        let before = std::fs::metadata(&target)?;
        assert_eq!(before.mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(destination.join("hash/nested"))?.mode() & 0o777,
            0o700
        );
        prepare_bit_assets(source.path(), &assets, Some(&destination))?;
        assert_eq!(std::fs::metadata(&target)?.ino(), before.ino());
        assert_eq!(std::fs::read(&target)?, b"weights");
        std::fs::remove_file(&target)?;
        std::os::unix::fs::symlink(source.path().join("bits/hash/nested/model"), &target)?;
        assert!(prepare_bit_assets(source.path(), &assets, Some(&destination)).is_err());
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
}
