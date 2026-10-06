use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use flow_like::{
    app::{
        App,
        sharing::device::{
            DeviceExportFile, DeviceLatestEvent, DeviceProjectSnapshot, MAX_DEVICE_EXPORT_CHUNK,
            validate_local_source,
        },
    },
    bit::{Bit, BitContentDigest, BitDigestAlgorithm, BitPack, bit_sources},
    flow_like_storage::{Path, files::store::FlowLikeStore, object_store::ObjectStoreExt},
    state::FlowLikeState,
};
use flow_like_device_protocol::{
    DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest, PackagedBitAsset, PackagedBitMetadata,
    PackagedBitMetadataV2, PackagedBitMetadataVersion2, ProjectArtifactFile,
    validate_artifact_relative_path,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::{Mutex, Semaphore};

const TTL: Duration = Duration::from_secs(3600);
/// Bit files without a digest known up front still travel in the artifact up to this size; the
/// device acquires every other file into its model store (Bit metadata v2).
const CARRIED_ASSET_MAX_BYTES: u64 = 64 * 1024 * 1024;
static PREPARING: Semaphore = Semaphore::const_new(1);
static EXPORTS: LazyLock<Mutex<HashMap<String, ExportSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct ExportSession {
    owner: String,
    expires: Instant,
    snapshot: Arc<DeviceProjectSnapshot>,
}

#[derive(Clone, Default, Serialize)]
pub struct Assets {
    bit_pins: Vec<BitPin>,
    package_pins: Vec<PackagePin>,
}
#[derive(Clone, Serialize)]
struct BitPin {
    bit_id: String,
    metadata_sha256: String,
}
#[derive(Clone, Serialize)]
struct PackagePin {
    package_id: String,
    version: String,
    wasm_sha256: String,
    manifest_sha256: String,
}
#[derive(Serialize)]
pub struct PreparedExport {
    export_id: String,
    project_id: String,
    source: &'static str,
    files: Vec<DeviceExportFile>,
    assets: Assets,
    /// Every event of a local project that follows Latest, with the flow version its staged
    /// copy was pinned to. Empty for an online project, whose hub resolves them.
    latest_events: Vec<DeviceLatestEvent>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn snapshot_digest(snapshot: &DeviceProjectSnapshot, path: &str, size: u64) -> Result<String> {
    let mut hash = Sha256::new();
    let mut offset = 0;
    while offset < size {
        let length = (size - offset).min(MAX_DEVICE_EXPORT_CHUNK as u64) as usize;
        hash.update(snapshot.read_chunk(path, offset, length)?);
        offset += length as u64;
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
        "Invalid dependency identifier"
    );
    Ok(())
}
fn reject_embedded_credentials(value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "api_key"
                        | "apikey"
                        | "access_token"
                        | "refresh_token"
                        | "password"
                        | "secret"
                        | "authorization"
                ) {
                    ensure!(
                        value.is_null() || value.as_str() == Some(""),
                        "A selected model contains embedded credentials. Configure credentials separately on the device."
                    );
                }
                reject_embedded_credentials(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_embedded_credentials(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn private_metadata_url(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .and_then(|text| reqwest::Url::parse(text).ok())
        .is_some_and(|url| {
            url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
        })
}

fn public_metadata(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(values) => {
            values.retain(|key, value| {
                !matches!(
                    key.as_str(),
                    "download_link"
                        | "download_url"
                        | "cwasm_download_url"
                        | "widget_bundle_download_url"
                ) && !private_metadata_url(value)
            });
            for value in values.values_mut() {
                public_metadata(value);
            }
        }
        serde_json::Value::Array(values) => {
            values.retain(|value| !private_metadata_url(value));
            for value in values {
                public_metadata(value);
            }
        }
        _ => {}
    }
}

fn public_bit_metadata(bit: &flow_like::bit::Bit) -> Result<serde_json::Value> {
    reject_embedded_credentials(&bit.parameters)?;
    let inline = |bit: &flow_like::bit::Bit| -> Result<Vec<serde_json::Value>> {
        let mut assets = bit.inline_mlx_asset_bits()
            .map_err(|_| anyhow::anyhow!("Inline MLX assets need a pinned repository revision and complete file manifest before deployment"))?;
        assets.extend(bit.projection_bit());
        Ok(assets
            .into_iter()
            .map(|asset| {
                serde_json::json!({
                    "id":asset.id,"hash":asset.hash,"file_name":asset.file_name,"size":asset.size,
                })
            })
            .collect())
    };
    let before = inline(bit)?;
    let mut value = serde_json::to_value(bit)?;
    public_metadata(&mut value);
    if let Some(projection) = bit.projection_bit() {
        let source = projection
            .download_link
            .context("Inline projector source is missing")?;
        let url = reqwest::Url::parse(&source).map_err(|_| {
            anyhow::anyhow!(
                "Inline projector needs a public canonical HTTPS source before deployment"
            )
        })?;
        ensure!(
            url.scheme() == "https"
                && url.as_str() == source
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Inline projector uses a signed or unsafe source URL. Select a public canonical HTTPS source without credentials, query parameters or fragments before deployment."
        );
        // This URL is part of the immutable source-derived identity, not a
        // download fallback. The artifact separately pins the copied bytes.
        let parameters = value
            .pointer_mut("/parameters/provider/params/projection")
            .and_then(serde_json::Value::as_object_mut)
            .context("Inline projector metadata changed during deployment preparation")?;
        parameters.insert("download_link".into(), serde_json::Value::String(source));
    }
    let public: flow_like::bit::Bit = serde_json::from_value(value.clone())?;
    ensure!(
        inline(&public)? == before,
        "Inline model asset identity changed when private metadata was removed. Select public pinned model sources before deployment."
    );
    Ok(value)
}

fn protocol_digest(digest: BitContentDigest) -> ModelAssetDigest {
    ModelAssetDigest {
        algorithm: match digest.algorithm {
            BitDigestAlgorithm::Blake3 => DigestAlgorithm::Blake3,
            BitDigestAlgorithm::Sha256 => DigestAlgorithm::Sha256,
        },
        hex: digest.hex,
    }
}

fn model_asset(
    item: &Bit,
    root: &Bit,
    name: &str,
    size: u64,
    digest: ModelAssetDigest,
) -> Result<PackagedBitAsset> {
    let descriptor = ModelAssetDescriptor {
        digest,
        size,
        file_name: name.to_owned(),
        sources: bit_sources(item, root),
    };
    descriptor.validate().map_err(|error| {
        anyhow!("Model file {name} cannot be deployed as a model asset: {error}")
    })?;
    Ok(PackagedBitAsset {
        bit_id: item.id.clone(),
        descriptor,
    })
}

/// The file of a Bit in this computer's Bit store, where downloads put it. Only regular files
/// under `<bit store>/<hash>/` resolve; links, traversal and the store's own directories do not.
fn local_bit_path(
    store: &FlowLikeStore,
    hash: &str,
    file_name: &str,
) -> Result<std::path::PathBuf> {
    identifier(hash)?;
    ensure!(
        hash != "metadata" && hash != "deps-cache",
        "Bit store directory {hash} holds no model files"
    );
    validate_artifact_relative_path(&format!("bits/{hash}/{file_name}"))
        .map_err(|_| anyhow!("Model file name {file_name:?} is not a safe relative path"))?;
    let FlowLikeStore::Local(local) = store else {
        bail!("Model files are read only from this computer's own Bit store");
    };
    let location = Path::from(hash).join(file_name);
    validate_local_source(store, &location).with_context(|| {
        format!("Model file {hash}/{file_name} is not in this computer's Bit store")
    })?;
    Ok(local.path_to_filesystem(&location)?)
}

pub(crate) fn read_range(path: &std::path::Path, offset: u64, length: usize) -> Result<Vec<u8>> {
    ensure!(
        length > 0 && length <= MAX_DEVICE_EXPORT_CHUNK,
        "Read 1 byte to 1 MiB of a model file at a time, not {length} bytes"
    );
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    ensure!(
        offset <= size && length as u64 <= size - offset,
        "Model file holds {size} bytes; {length} bytes at offset {offset} lie outside it"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn sha256_of(path: &std::path::Path, size: u64) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let actual = file.metadata()?.len();
    ensure!(
        actual == size,
        "Model file {} holds {actual} bytes; its Bit declares {size}",
        path.display()
    );
    let mut hash = Sha256::new();
    let mut buffer = vec![0; MAX_DEVICE_EXPORT_CHUNK];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Downloads what of `pack` is not in this computer's Bit store yet.
async fn ensure_installed(
    state: &Arc<FlowLikeState>,
    pack: &BitPack,
    failure: String,
) -> Result<()> {
    if !pack.is_installed(state.clone()).await? {
        pack.download(state.clone(), None).await.context(failure)?;
    }
    Ok(())
}

/// The sha256 of a Bit file no digest names, hashed from this computer's copy (downloaded first).
async fn local_sha256(
    state: &Arc<FlowLikeState>,
    bits_source: &FlowLikeStore,
    item: &Bit,
    name: &str,
    size: u64,
) -> Result<ModelAssetDigest> {
    let single = BitPack {
        bits: vec![item.clone()],
    };
    let failure = format!("Model file {name} could not be downloaded to hash it");
    ensure_installed(state, &single, failure).await?;
    let path = local_bit_path(bits_source, &item.hash, name)?;
    let hex = tokio::task::spawn_blocking(move || sha256_of(&path, size)).await??;
    Ok(ModelAssetDigest {
        algorithm: DigestAlgorithm::Sha256,
        hex,
    })
}

/// The model-store assets of a pack: every Bit file with a digest known up front, and every
/// larger one hashed from this computer's copy. Smaller files without a digest stay artifacts.
async fn model_assets(
    state: &Arc<FlowLikeState>,
    bits_source: &FlowLikeStore,
    root: &Bit,
    pack: &BitPack,
) -> Result<Vec<PackagedBitAsset>> {
    let mut assets = Vec::new();
    for item in &pack.bits {
        let Some(name) = item.file_name.as_deref() else {
            continue;
        };
        let size = item.size.with_context(|| {
            format!("Model file {name} has no known size; refresh its metadata before deployment")
        })?;
        let digest = match item.content_digest(root) {
            Some(digest) => protocol_digest(digest),
            None if size <= CARRIED_ASSET_MAX_BYTES => continue,
            None => local_sha256(state, bits_source, item, name, size).await?,
        };
        assets.push(model_asset(item, root, name, size, digest)?);
    }
    Ok(assets)
}

/// v1 while every file travels in the artifact; v2 names the model-store assets the device
/// acquires itself, checked as the device will check it.
fn metadata_bytes(
    id: &str,
    bit: serde_json::Value,
    dependencies: Vec<serde_json::Value>,
    assets: Vec<PackagedBitAsset>,
    artifacts: Vec<ProjectArtifactFile>,
) -> Result<Vec<u8>> {
    if assets.is_empty() {
        let artifacts = artifacts
            .iter()
            .map(serde_json::to_value)
            .collect::<serde_json::Result<Vec<_>>>()?;
        return Ok(serde_json::to_vec(
            &serde_json::json!({"bit":bit,"dependencies":dependencies,"artifacts":artifacts}),
        )?);
    }
    let metadata = PackagedBitMetadata::V2(PackagedBitMetadataV2 {
        version: PackagedBitMetadataVersion2,
        bit,
        dependencies,
        assets,
        artifacts,
    });
    metadata.validate(id).map_err(|error| {
        anyhow!("Model {id} cannot be deployed with model files the device fetches: {error}")
    })?;
    Ok(serde_json::to_vec(&metadata)?)
}

/// Copies a Bit file into the snapshot once per location and pins its bytes.
async fn artifact_file(
    item: &Bit,
    name: &str,
    bits_source: &FlowLikeStore,
    snapshot: &mut DeviceProjectSnapshot,
    files: &mut BTreeMap<String, (u64, String)>,
) -> Result<ProjectArtifactFile> {
    let bits_store = bits_source.as_generic();
    let path = format!("bits/{}/{name}", item.hash);
    let (size, hash) = if let Some(existing) = files.get(&path) {
        existing.clone()
    } else {
        let location = Path::from(item.hash.as_str()).join(name);
        validate_local_source(bits_source, &location)?;
        let metadata = bits_store.head(&location).await.with_context(|| {
            format!("Download model asset {name} before exporting this project")
        })?;
        ensure!(
            item.size == Some(metadata.size),
            "Model asset size differs from its selected metadata"
        );
        snapshot
            .add_object(&path, bits_store.clone(), &metadata)
            .await?;
        let hash = snapshot_digest(snapshot, &path, metadata.size)?;
        files.insert(path.clone(), (metadata.size, hash.clone()));
        (metadata.size, hash)
    };
    ensure!(
        item.size == Some(size),
        "Selected models disagree about asset size"
    );
    Ok(ProjectArtifactFile {
        path,
        size,
        sha256: hash,
    })
}

fn check_file_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && !name.contains('\\')
            && name
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".."),
        "Invalid model asset filename"
    );
    Ok(())
}

/// Every pack member's identity and file name, before any byte of it is read.
fn check_pack(pack: &BitPack) -> Result<()> {
    ensure!(pack.bits.len() <= 2049, "Too many model dependency files");
    for item in &pack.bits {
        identifier(&item.id)?;
        reject_embedded_credentials(&item.parameters)?;
        if let Some(name) = &item.file_name {
            identifier(&item.hash)?;
            check_file_name(name)?;
        }
    }
    Ok(())
}

/// The pack members whose files travel in the artifact: all but the model-store assets.
fn carried_pack(pack: &BitPack, stored: &[PackagedBitAsset]) -> BitPack {
    let stored_ids = stored
        .iter()
        .map(|asset| asset.bit_id.as_str())
        .collect::<HashSet<_>>();
    BitPack {
        bits: pack
            .bits
            .iter()
            .filter(|item| !stored_ids.contains(item.id.as_str()))
            .cloned()
            .collect(),
    }
}

/// The artifact's model bytes stay within the per-file and per-project caps.
fn check_carried_sizes(carried: &BitPack) -> Result<()> {
    let selected_bytes = carried
        .bits
        .iter()
        .filter(|item| item.file_name.is_some())
        .try_fold(0_u64, |total, item| {
            let size = item.size.context(
                "Model artifact size is unknown; refresh its metadata before deployment",
            )?;
            ensure!(
                size > 0 && size <= 4 * 1024 * 1024 * 1024,
                "Model artifact exceeds 4 GiB"
            );
            total.checked_add(size).context("Model asset size overflow")
        })?;
    ensure!(
        selected_bytes <= 8 * 1024 * 1024 * 1024,
        "Selected model assets exceed 8 GiB"
    );
    Ok(())
}

fn public_dependencies(pack: &BitPack, root: &Bit) -> Result<Vec<serde_json::Value>> {
    pack.bits
        .iter()
        .filter(|item| item.id != root.id)
        .map(public_bit_metadata)
        .collect()
}

/// Where the selected Bits' files come from, and whether devices fetch model files themselves.
struct BitSources {
    state: Arc<FlowLikeState>,
    store: FlowLikeStore,
    model_store: bool,
}

/// The selected Bit's and its dependencies' public metadata, once every pack member is checked.
fn public_pack_metadata(
    bit: &Bit,
    pack: &BitPack,
) -> Result<(serde_json::Value, Vec<serde_json::Value>)> {
    let public_bit = public_bit_metadata(bit)?;
    let public_dependencies = public_dependencies(pack, bit)?;
    check_pack(pack)?;
    Ok((public_bit, public_dependencies))
}

/// The pack's model-store assets: none while some target device lacks a model store.
async fn stored_assets(
    sources: &BitSources,
    root: &Bit,
    pack: &BitPack,
) -> Result<Vec<PackagedBitAsset>> {
    if !sources.model_store {
        return Ok(Vec::new());
    }
    model_assets(&sources.state, &sources.store, root, pack).await
}

/// The files of the members that travel in the artifact, downloaded when missing and copied
/// into the snapshot once per path.
async fn carried_artifacts(
    sources: &BitSources,
    carried: &BitPack,
    snapshot: &mut DeviceProjectSnapshot,
    files: &mut BTreeMap<String, (u64, String)>,
) -> Result<Vec<ProjectArtifactFile>> {
    check_carried_sizes(carried)?;
    let failure = "A required model asset could not be downloaded".to_owned();
    ensure_installed(&sources.state, carried, failure).await?;
    let mut artifacts = BTreeMap::new();
    for item in &carried.bits {
        if let Some(name) = &item.file_name {
            let artifact = artifact_file(item, name, &sources.store, snapshot, files).await?;
            artifacts.insert(artifact.path.clone(), artifact);
        }
    }
    Ok(artifacts.into_values().collect())
}

/// Packs one selected Bit: its public metadata (v1, or v2 naming model-store assets) and the
/// artifact files that travel with it.
async fn bit_pin(
    sources: &BitSources,
    id: &str,
    bit: &Bit,
    snapshot: &mut DeviceProjectSnapshot,
    files: &mut BTreeMap<String, (u64, String)>,
) -> Result<BitPin> {
    let pack = bit.pack(sources.state.clone()).await?;
    let (public_bit, public_dependencies) = public_pack_metadata(bit, &pack)?;
    let stored = stored_assets(sources, bit, &pack).await?;
    let carried = carried_pack(&pack, &stored);
    let artifacts = carried_artifacts(sources, &carried, snapshot, files).await?;
    let bytes = metadata_bytes(id, public_bit, public_dependencies, stored, artifacts)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Model metadata exceeds 16 MiB"
    );
    snapshot
        .add_bytes(&format!("bits/metadata/{id}.json"), &bytes)
        .await?;
    Ok(BitPin {
        bit_id: id.to_owned(),
        metadata_sha256: digest(&bytes),
    })
}

/// `model_store`: every target device acquires model files into its model store, so Bit
/// metadata goes v2 and the artifact carries only small files without a known digest.
async fn dependencies(
    handle: &AppHandle,
    app: &App,
    snapshot: &mut DeviceProjectSnapshot,
    model_store: bool,
) -> Result<Assets> {
    ensure!(
        app.bits.len() <= 256 && app.packages.len() <= 64,
        "Too many selected dependencies"
    );
    let state = TauriFlowLikeState::construct(handle).await?;
    let profile = TauriSettingsState::current_profile(handle).await?;
    let http = TauriFlowLikeState::http_client(handle).await?;
    let sources = BitSources {
        store: FlowLikeState::bit_store(&state).await?,
        state,
        model_store,
    };
    let mut assets = Assets::default();
    let mut files = BTreeMap::<String, (u64, String)>::new();
    for reference in &app.bits {
        let (hub, id) = reference
            .rsplit_once(':')
            .map_or((None, reference.as_str()), |(hub, id)| {
                (Some(hub.to_owned()), id)
            });
        identifier(id)?;
        let bit = profile
            .hub_profile
            .get_bit(id.to_owned(), hub, http.clone())
            .await?;
        let pin = bit_pin(&sources, id, &bit, snapshot, &mut files).await?;
        assets.bit_pins.push(pin);
    }
    assets.package_pins = package_pins(handle, app, snapshot).await?;
    Ok(assets)
}

/// The selected WASM packages, each copied into the snapshot and pinned by its digests.
async fn package_pins(
    handle: &AppHandle,
    app: &App,
    snapshot: &mut DeviceProjectSnapshot,
) -> Result<Vec<PackagePin>> {
    let mut pins = Vec::new();
    let mut total_wasm = 0usize;
    if !app.packages.is_empty() {
        let registry = crate::functions::registry::registry_client(handle).await?;
        let project = (!matches!(app.visibility, flow_like::app::AppVisibility::Offline))
            .then_some(app.id.as_str());
        let mut packages = app.packages.iter().collect::<Vec<_>>();
        packages.sort();
        for (id, version) in packages {
            identifier(id)?;
            identifier(version)?;
            let installed = registry.get_installed(id).await;
            let selected = installed
                .as_ref()
                .filter(|package| registry.from_current_registry(package))
                .and_then(|package| package.get_version(version))
                .filter(|selected| !selected.manifest.nodes_withheld());
            let (manifest, wasm) = if let Some(selected) = selected {
                ensure!(
                    selected.manifest.id == *id
                        && selected.manifest.version == *version
                        && selected.manifest.validate().is_ok(),
                    "Installed package manifest differs from the project's pin"
                );
                let metadata = tokio::fs::metadata(&selected.wasm_path).await?;
                ensure!(metadata.len() <= 64 * 1024 * 1024, "Package exceeds 64 MiB");
                use tokio::io::AsyncReadExt;
                let file = tokio::fs::File::open(&selected.wasm_path).await?;
                let mut wasm = Vec::new();
                file.take(64 * 1024 * 1024 + 1)
                    .read_to_end(&mut wasm)
                    .await?;
                ensure!(
                    wasm.len() <= 64 * 1024 * 1024 && wasm.starts_with(b"\0asm"),
                    "Package must contain portable WASM bytes"
                );
                if let Some(expected) = &selected.wasm_hash {
                    ensure!(
                        expected == &blake3::hash(&wasm).to_hex().to_string(),
                        "Installed package digest differs; reinstall it before deploying"
                    );
                }
                (selected.manifest.clone(), wasm)
            } else {
                registry
                    .export_package_version(id, version, 64 * 1024 * 1024, project)
                    .await?
            };
            total_wasm += wasm.len();
            ensure!(
                total_wasm <= 256 * 1024 * 1024,
                "Selected WASM packages exceed 256 MiB"
            );
            let wasm_sha256 = digest(&wasm);
            let manifest = serde_json::to_vec(&manifest)?;
            ensure!(
                manifest.len() <= 1024 * 1024,
                "Package manifest exceeds 1 MiB"
            );
            snapshot
                .add_bytes(&format!("packages/{id}/{version}/module.wasm"), &wasm)
                .await?;
            snapshot
                .add_bytes(&format!("packages/{id}/{version}/manifest.json"), &manifest)
                .await?;
            pins.push(PackagePin {
                package_id: id.clone(),
                version: version.clone(),
                wasm_sha256,
                manifest_sha256: digest(&manifest),
            });
        }
    }
    Ok(pins)
}

async fn prepare(
    handle: &AppHandle,
    app_id: String,
    online_app: Option<App>,
    user_sub: Option<String>,
    model_store: bool,
) -> Result<PreparedExport> {
    let _permit = PREPARING
        .try_acquire()
        .context("Another project snapshot is being prepared")?;
    let owner = TauriSettingsState::current_profile(handle)
        .await?
        .hub_profile
        .id;
    {
        let mut sessions = EXPORTS.lock().await;
        sessions.retain(|_, session| session.expires > Instant::now());
        ensure!(
            sessions.len() < 2,
            "Finish or discard another prepared deployment before exporting"
        );
    }
    let state = TauriFlowLikeState::construct(handle).await?;
    let (app, mut snapshot, source) = if let Some(app) = online_app {
        ensure!(
            app.id == app_id && !matches!(app.visibility, flow_like::app::AppVisibility::Offline),
            "Online project identity differs"
        );
        (
            app,
            DeviceProjectSnapshot::online_dependencies(&app_id).await?,
            "online",
        )
    } else {
        let app = App::load(app_id.clone(), state).await?;
        let snapshot = app
            .export_device_snapshot(
                user_sub
                    .as_deref()
                    .context("Select the account whose local project data should be exported")?,
            )
            .await?;
        (app, snapshot, "offline")
    };
    let assets = dependencies(handle, &app, &mut snapshot, model_store).await?;
    ensure!(
        TauriSettingsState::current_profile(handle)
            .await?
            .hub_profile
            .id
            == owner,
        "The active account changed during export"
    );
    let export_id = uuid::Uuid::new_v4().to_string();
    let response = PreparedExport {
        export_id: export_id.clone(),
        project_id: app_id,
        source,
        files: snapshot.files(),
        assets,
        latest_events: snapshot.latest_events().to_vec(),
    };
    EXPORTS.lock().await.insert(
        export_id.clone(),
        ExportSession {
            owner,
            expires: Instant::now() + TTL,
            snapshot: Arc::new(snapshot),
        },
    );
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let mut sessions = EXPORTS.lock().await;
            let Some(session) = sessions.get(&export_id) else {
                break;
            };
            if session.expires <= Instant::now() {
                sessions.remove(&export_id);
                break;
            }
        }
    });
    Ok(response)
}

/// `model_store`: every target agent advertises `model_store`, so Bit metadata goes v2.
#[tauri::command]
pub async fn prepare_device_project_export(
    app_handle: AppHandle,
    app_id: String,
    online_app: Option<App>,
    user_sub: Option<String>,
    model_store: Option<bool>,
) -> Result<PreparedExport, TauriFunctionError> {
    prepare(
        &app_handle,
        app_id,
        online_app,
        user_sub,
        model_store.unwrap_or(false),
    )
    .await
    .map_err(Into::into)
}

/// The file `<hash>/<file_name>` in this computer's Bit store.
pub(crate) async fn local_bit_file(
    app_handle: &AppHandle,
    hash: &str,
    file_name: &str,
) -> Result<std::path::PathBuf, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let bits_source = FlowLikeState::bit_store(&state).await?;
    Ok(local_bit_path(&bits_source, hash, file_name)?)
}

/// Reads up to 1 MiB of a file in this computer's Bit store, `<hash>/<file_name>`, to send a
/// model asset to a device that could not fetch it itself.
#[tauri::command]
pub async fn read_bit_store_chunk(
    app_handle: AppHandle,
    hash: String,
    file_name: String,
    offset: u64,
    length: usize,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    let path = local_bit_file(&app_handle, &hash, &file_name).await?;
    let bytes = tokio::task::spawn_blocking(move || read_range(&path, offset, length))
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// The prepared snapshot of the active account; each read keeps it another hour.
pub(crate) async fn export_snapshot(
    app_handle: &AppHandle,
    export_id: &str,
) -> Result<Arc<DeviceProjectSnapshot>, TauriFunctionError> {
    let owner = TauriSettingsState::current_profile(app_handle)
        .await?
        .hub_profile
        .id;
    let mut sessions = EXPORTS.lock().await;
    let session = sessions
        .get_mut(export_id)
        .filter(|session| session.owner == owner && session.expires > Instant::now())
        .ok_or_else(|| {
            TauriFunctionError::new("Prepared export expired or belongs to another account")
        })?;
    session.expires = Instant::now() + TTL;
    Ok(session.snapshot.clone())
}

#[tauri::command]
pub async fn read_device_project_export_chunk(
    app_handle: AppHandle,
    export_id: String,
    path: String,
    offset: u64,
    length: usize,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    let snapshot = export_snapshot(&app_handle, &export_id).await?;
    let bytes = tokio::task::spawn_blocking(move || snapshot.read_chunk(&path, offset, length))
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn release_device_project_export(
    app_handle: AppHandle,
    export_id: String,
) -> Result<(), TauriFunctionError> {
    let owner = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;
    let mut sessions = EXPORTS.lock().await;
    if sessions
        .get(&export_id)
        .is_some_and(|session| session.owner == owner)
    {
        sessions.remove(&export_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inline_model(provider: &str) -> flow_like::bit::Bit {
        flow_like::bit::Bit {
            id: "inline-model".into(),
            bit_type: flow_like::bit::BitTypes::Vlm,
            parameters: serde_json::json!({"context_length":8192,"model_classification":flow_like::bit::BitModelClassification::default(),
                "provider":{"provider_name":provider,"model_id":"inline-model","params":{}}}),
            ..Default::default()
        }
    }
    #[test]
    fn public_projector_source_preserves_identity_and_signed_sources_are_rejected() {
        let mut bit = inline_model("local");
        let safe = "https://example.test/immutable/revision/projector.gguf";
        bit.parameters["provider"]["params"]["projection"] = serde_json::json!({
            "download_link":safe,"file_name":"projector.gguf","size":700});
        let original = bit.projection_bit().expect("projector");
        let public = public_bit_metadata(&bit).unwrap();
        assert_eq!(
            public
                .pointer("/parameters/provider/params/projection/download_link")
                .and_then(|value| value.as_str()),
            Some(safe)
        );
        let restored: flow_like::bit::Bit = serde_json::from_value(public).unwrap();
        assert_eq!(restored.projection_bit().unwrap().id, original.id);
        for unsafe_source in [
            "https://example.test/projector?signature=private",
            "https://user:private@example.test/projector",
            "https://example.test/projector#fragment",
            "http://example.test/projector",
        ] {
            bit.parameters["provider"]["params"]["projection"]["download_link"] =
                unsafe_source.into();
            let error = public_bit_metadata(&bit).unwrap_err().to_string();
            assert!(error.contains("signed or unsafe"));
            assert!(!error.contains("private"));
        }
    }
    #[test]
    fn pinned_inline_mlx_manifest_survives_metadata_sanitization() {
        let mut bit = inline_model("mlx");
        bit.parameters["huggingface"] = serde_json::json!({"schema":1,"repo_id":"owner/model","revision":"a".repeat(40),"format":"mlx","files":[
            {"path":"config.json","size":100},{"path":"tokenizer.json","size":200},
            {"path":"tokenizer_config.json","size":300},{"path":"processor_config.json","size":400},
            {"path":"model.safetensors","size":4000}]});
        let original = bit.inline_mlx_asset_bits().unwrap();
        assert_eq!(original.len(), 5);
        let public: flow_like::bit::Bit =
            serde_json::from_value(public_bit_metadata(&bit).unwrap()).unwrap();
        let actual = public.inline_mlx_asset_bits().unwrap();
        assert_eq!(
            actual.iter().map(|bit| &bit.id).collect::<Vec<_>>(),
            original.iter().map(|bit| &bit.id).collect::<Vec<_>>()
        );
        bit.parameters["huggingface"]["revision"] = "main".into();
        assert!(
            public_bit_metadata(&bit)
                .unwrap_err()
                .to_string()
                .contains("pinned repository revision")
        );
    }
    #[test]
    fn signed_media_urls_are_removed_from_nested_arrays() {
        let mut metadata = serde_json::json!({"preview_media":[
            "https://media.test/public.webp", "https://media.test/a?signature=private",
            ["https://user:private@media.test/a", "https://media.test/a#private", 7],
            {"urls":["HTTPS://media.test/a?token=private", "https://media.test/kept"]}
        ]});
        public_metadata(&mut metadata);
        assert_eq!(
            metadata,
            serde_json::json!({"preview_media":[
                "https://media.test/public.webp", [7], {"urls":["https://media.test/kept"]}
            ]})
        );
        assert!(!metadata.to_string().contains("private"));
    }
    fn bit_store() -> (std::path::PathBuf, FlowLikeStore) {
        let root =
            std::env::temp_dir().join(format!("flow-like-bit-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = flow_like::flow_like_storage::files::store::local_store::LocalObjectStore::new(
            root.clone(),
        )
        .unwrap();
        (root, FlowLikeStore::Local(store.into()))
    }

    fn hub_model() -> Bit {
        Bit {
            id: "qwen3-8b".into(),
            bit_type: flow_like::bit::BitTypes::Llm,
            hash: "9a129038d9a00aed0cf6a7ea059ca50a813449061ab87848cf1a13eafdf33b2c".into(),
            file_name: Some("Qwen3-8B-Q4_K_M.gguf".into()),
            size: Some(4096),
            download_link: Some("https://cdn.flow-like.com/bits/qwen3-8b-q4".into()),
            parameters: serde_json::json!({"provider":{"provider_name":"Local",
                "model_id":"Qwen/Qwen3-8B-GGUF","version":"03e404fd168941cfed98f46654680130dd85968b"}}),
            ..Default::default()
        }
    }

    #[test]
    fn bit_store_reads_stay_inside_regular_files_of_the_store() {
        let (root, store) = bit_store();
        let hash = "a".repeat(64);
        std::fs::create_dir_all(root.join(&hash)).unwrap();
        std::fs::write(root.join(&hash).join("model.gguf"), b"0123456789").unwrap();
        let path = local_bit_path(&store, &hash, "model.gguf").unwrap();
        assert_eq!(read_range(&path, 2, 3).unwrap(), b"234");
        assert_eq!(read_range(&path, 9, 1).unwrap(), b"9");
        for (offset, length) in [(10, 1), (0, 11), (0, 0), (0, MAX_DEVICE_EXPORT_CHUNK + 1)] {
            assert!(
                read_range(&path, offset, length).is_err(),
                "{offset}+{length}"
            );
        }
        for (dir, name) in [
            (hash.as_str(), "../escape"),
            (hash.as_str(), "missing.gguf"),
            (hash.as_str(), "dir\\model.gguf"),
            (hash.as_str(), ""),
            ("metadata", "model.gguf"),
            ("deps-cache", "model.gguf"),
            ("..", "model.gguf"),
            ("a/b", "model.gguf"),
        ] {
            assert!(local_bit_path(&store, dir, name).is_err(), "{dir}/{name}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                root.join(&hash).join("model.gguf"),
                root.join(&hash).join("link.gguf"),
            )
            .unwrap();
            assert!(local_bit_path(&store, &hash, "link.gguf").is_err());
        }
        let missing = std::env::temp_dir().join(format!("absent-{}", uuid::Uuid::new_v4()));
        assert!(sha256_of(&missing, 1).is_err());
        assert_eq!(
            sha256_of(&path, 10).unwrap(),
            format!("{:x}", Sha256::digest(b"0123456789"))
        );
        assert!(sha256_of(&path, 11).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hub_files_become_assets_with_their_digest_and_sources() {
        let bit = hub_model();
        let digest = protocol_digest(bit.content_digest(&bit).unwrap());
        let asset = model_asset(&bit, &bit, "Qwen3-8B-Q4_K_M.gguf", 4096, digest).unwrap();
        assert_eq!(asset.bit_id, "qwen3-8b");
        assert_eq!(asset.descriptor.digest.algorithm, DigestAlgorithm::Blake3);
        assert_eq!(asset.descriptor.digest.hex, bit.hash);
        assert_eq!(
            asset.descriptor.sources,
            vec![
                "https://cdn.flow-like.com/bits/qwen3-8b-q4".to_owned(),
                "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/03e404fd168941cfed98f46654680130dd85968b/Qwen3-8B-Q4_K_M.gguf?download=true".to_owned(),
            ]
        );
        let unsafe_name = model_asset(
            &bit,
            &bit,
            "../escape.gguf",
            4096,
            protocol_digest(bit.content_digest(&bit).unwrap()),
        );
        assert!(unsafe_name.is_err());
    }

    #[test]
    fn metadata_goes_v2_only_with_model_store_assets_and_is_checked_like_the_device() {
        let bit = hub_model();
        let public = public_bit_metadata(&bit).unwrap();
        let v1 = metadata_bytes("qwen3-8b", public.clone(), vec![], vec![], vec![]).unwrap();
        let v1: serde_json::Value = serde_json::from_slice(&v1).unwrap();
        assert!(v1.get("version").is_none());
        assert_eq!(v1["artifacts"], serde_json::json!([]));

        let digest = protocol_digest(bit.content_digest(&bit).unwrap());
        let asset = model_asset(&bit, &bit, "Qwen3-8B-Q4_K_M.gguf", 4096, digest).unwrap();
        let bytes = metadata_bytes(
            "qwen3-8b",
            public.clone(),
            vec![],
            vec![asset.clone()],
            vec![],
        )
        .unwrap();
        let parsed: PackagedBitMetadata = serde_json::from_slice(&bytes).unwrap();
        assert!(matches!(parsed, PackagedBitMetadata::V2(_)));
        parsed.validate("qwen3-8b").unwrap();
        assert!(
            !String::from_utf8(bytes.clone())
                .unwrap()
                .contains("\"artifacts\"")
        );

        let mut resized = asset;
        resized.descriptor.size = 4097;
        assert!(metadata_bytes("qwen3-8b", public, vec![], vec![resized], vec![]).is_err());
    }

    #[test]
    fn model_credentials_are_not_exported_as_public_metadata() {
        assert!(
            reject_embedded_credentials(&serde_json::json!({"provider": {"api_key":"secret"}}))
                .is_err()
        );
        assert!(
            reject_embedded_credentials(&serde_json::json!({"api_key":null,"model":"model"}))
                .is_ok()
        );
        assert!(identifier("../escape").is_err());
    }
}
