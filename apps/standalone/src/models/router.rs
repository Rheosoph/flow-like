//! Serves a placement's Local, MLX, ONNX and own-device Bits from the device's model host.
//!
//! The placement asks the agent over its supervisor channel for one of its pinned Bits. The
//! agent reads that Bit's committed metadata, hosts the model on demand (sharing an existing
//! hosted model with the same assets) and answers the gateway endpoint with the placement's
//! own bearer token. A Bit the host does not serve stays on the in-process path.

use super::{
    db::{AssetOwner, OwnerKind},
    host::ModelHost,
    store::ModelStore,
};
use crate::config::PlacementConfig;
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    DigestAlgorithm, MODEL_DISPLAY_NAME_MAX_BYTES, ModelAssetDescriptor, ModelAssetDigest,
    ModelEngine, ModelKind, ModelPooling, ModelSpec, PACKAGED_BIT_METADATA_MAX_BYTES,
    PackagedBitAsset, PackagedBitMetadata, ProjectArtifactFile, validate_artifact_relative_path,
};
use flow_like_runtime::{
    bit::{Bit, BitTypes},
    flow_like_model_provider::provider::Pooling,
    models::device::{
        DeviceModelTarget, LocalModelRouter, ModelEndpoint, ModelUnavailable,
        ModelUnavailableReason,
    },
};
use flow_like_types::async_trait;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const IMPORT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// How long a placement uses the agent's answer for one of its Bits before it asks again.
const ROUTE_TTL: Duration = Duration::from_secs(30);
const TOKENIZER_ROLES: [(BitTypes, &str); 4] = [
    (BitTypes::Tokenizer, "tokenizer.json"),
    (BitTypes::Config, "config.json"),
    (BitTypes::TokenizerConfig, "tokenizer_config.json"),
    (BitTypes::SpecialTokensMap, "special_tokens_map.json"),
];

/// The gateway endpoint of one hosted model, for one placement.
pub struct PlacementEndpoint {
    pub base_url: String,
    pub bearer: Zeroizing<String>,
    pub model: String,
}

/// A pinned Bit's committed metadata (`bits/metadata/<bit_id>.json`) with its Bits typed. In
/// v2 weights are model-store assets; small files may still be artifact files of the revision.
struct BitMetadata {
    v2: bool,
    bit: Bit,
    dependencies: Vec<Bit>,
    assets: Vec<PackagedBitAsset>,
    artifacts: Vec<ProjectArtifactFile>,
}

/// A file a hosted model loads, and the artifact path it is imported from when it is not a
/// model-store asset.
type BitFile = (ModelAssetDescriptor, Option<String>);

impl BitMetadata {
    fn typed(packaged: PackagedBitMetadata) -> Result<Self> {
        let (v2, bit, dependencies, assets, artifacts) = match packaged {
            PackagedBitMetadata::V1(v1) => (false, v1.bit, v1.dependencies, vec![], v1.artifacts),
            PackagedBitMetadata::V2(v2) => (true, v2.bit, v2.dependencies, v2.assets, v2.artifacts),
        };
        let bit: Bit = serde_json::from_value(bit).context("Read a packaged Bit")?;
        let dependencies = dependencies
            .into_iter()
            .map(serde_json::from_value)
            .collect::<serde_json::Result<_>>()
            .with_context(|| format!("Read the dependencies of Bit {}", bit.id))?;
        Ok(Self {
            v2,
            bit,
            dependencies,
            assets,
            artifacts,
        })
    }

    /// The store assets of a Bit, in load order; an artifact file is described by its sha256.
    fn files_of(&self, bit: &Bit) -> Vec<BitFile> {
        let assets: Vec<_> = self
            .assets
            .iter()
            .filter(|entry| entry.bit_id == bit.id)
            .map(|entry| (entry.descriptor.clone(), None))
            .collect();
        if !assets.is_empty() {
            return assets;
        }
        let Some(file_name) = &bit.file_name else {
            return Vec::new();
        };
        let path = format!("bits/{}/{file_name}", bit.hash);
        self.artifacts
            .iter()
            .filter(|artifact| artifact.path == path)
            .map(|artifact| {
                (
                    ModelAssetDescriptor {
                        digest: ModelAssetDigest {
                            algorithm: DigestAlgorithm::Sha256,
                            hex: artifact.sha256.clone(),
                        },
                        size: artifact.size,
                        file_name: file_name.clone(),
                        sources: Vec::new(),
                    },
                    Some(artifact.path.clone()),
                )
            })
            .collect()
    }

    /// The one file of `bit` that the root model loads next to its weights.
    fn single_file(&self, bit: &Bit, role: &str) -> Result<BitFile> {
        let mut found = self.files_of(bit);
        ensure!(
            found.len() == 1,
            "Bit {}: its {role} must be exactly one file, not {}",
            self.bit.id,
            found.len()
        );
        Ok(found.remove(0))
    }
}

fn display_name(bit: &Bit) -> String {
    let name = bit
        .meta
        .get("en")
        .or_else(|| bit.meta.values().next())
        .map(|meta| meta.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or(&bit.id);
    let clean: String = name
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let mut end = clean.len().min(MODEL_DISPLAY_NAME_MAX_BYTES);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_owned()
}

fn is_local(bit: &Bit) -> bool {
    let provider = match bit.bit_type {
        BitTypes::Embedding => bit.try_to_embedding_provider(),
        _ => bit.try_to_provider(),
    };
    provider.is_some_and(|provider| provider.provider_name.trim().eq_ignore_ascii_case("local"))
}

fn embedding_engine(bit: &Bit) -> Option<ModelEngine> {
    let file = bit.file_name.as_deref()?.to_ascii_lowercase();
    if file.ends_with(".gguf") {
        Some(ModelEngine::Llamacpp)
    } else if file.ends_with(".onnx") {
        Some(ModelEngine::Onnx)
    } else {
        None
    }
}

/// The Bits the device's model host serves when their metadata is v2: Local chat and vision
/// models, Local GGUF or ONNX embedding models, and MLX chat and vision models on Apple-silicon
/// Macs.
pub fn hosts(bit: &Bit) -> bool {
    if bit.is_mlx_model() {
        return cfg!(all(target_os = "macos", target_arch = "aarch64"));
    }
    is_local(bit)
        && match bit.bit_type {
            BitTypes::Llm | BitTypes::Vlm => bit.file_name.is_some(),
            BitTypes::Embedding => embedding_engine(bit).is_some(),
            _ => false,
        }
}

fn bit_pooling(bit: &Bit) -> Option<ModelPooling> {
    match bit.try_to_embedding()?.pooling {
        Pooling::CLS => Some(ModelPooling::Cls),
        Pooling::Mean => Some(ModelPooling::Mean),
        Pooling::None => None,
    }
}

/// A model spec plus the artifact files it still needs imported into the store.
struct Hosting {
    spec: ModelSpec,
    imports: Vec<(ModelAssetDescriptor, String)>,
}

/// The tokenizer files an ONNX embedding model loads, named as the worker expects them.
fn tokenizer_files(metadata: &BitMetadata) -> Result<Vec<BitFile>> {
    TOKENIZER_ROLES
        .iter()
        .map(|(role, name)| {
            let dependency = metadata
                .dependencies
                .iter()
                .find(|dependency| dependency.bit_type == *role)
                .with_context(|| format!("ONNX Bit {} lacks its {name}", metadata.bit.id))?;
            let (mut descriptor, artifact) = metadata.single_file(dependency, name)?;
            descriptor.file_name = (*name).to_owned();
            Ok((descriptor, artifact))
        })
        .collect()
}

/// How an engine loads a Bit: its kind and engine, plus the files it loads next to the weights.
struct Loading {
    kind: ModelKind,
    engine: ModelEngine,
    projector: Option<String>,
    pooling: Option<ModelPooling>,
    extra: Vec<BitFile>,
}

impl Loading {
    fn llama(kind: ModelKind, pooling: Option<ModelPooling>) -> Self {
        Self {
            kind,
            engine: ModelEngine::Llamacpp,
            projector: None,
            pooling,
            extra: Vec::new(),
        }
    }
}

/// An MLX model loads the files of all its dependencies, named by their paths in the model
/// directory.
fn mlx_loading(metadata: &BitMetadata) -> Loading {
    let kind = match metadata.bit.bit_type {
        BitTypes::Vlm => ModelKind::Vision,
        _ => ModelKind::Chat,
    };
    Loading {
        engine: ModelEngine::Mlx,
        extra: metadata
            .dependencies
            .iter()
            .flat_map(|dependency| metadata.files_of(dependency))
            .collect(),
        ..Loading::llama(kind, None)
    }
}

/// A vision model's projector and an ONNX model's tokenizer files may be model-store assets
/// or artifact files of the revision.
fn loading(metadata: &BitMetadata) -> Result<Loading> {
    let bit = &metadata.bit;
    if bit.is_mlx_model() {
        return Ok(mlx_loading(metadata));
    }
    Ok(match bit.bit_type {
        BitTypes::Vlm => {
            let projection = bit
                .projection_bit()
                .context("A local vision Bit needs its projector")?;
            let projector = metadata.single_file(&projection, "projector")?;
            Loading {
                projector: Some(projector.0.file_name.clone()),
                extra: vec![projector],
                ..Loading::llama(ModelKind::Vision, None)
            }
        }
        BitTypes::Embedding if embedding_engine(bit) == Some(ModelEngine::Onnx) => Loading {
            engine: ModelEngine::Onnx,
            extra: tokenizer_files(metadata)?,
            ..Loading::llama(
                ModelKind::Embedding,
                Some(bit_pooling(bit).unwrap_or(ModelPooling::Mean)),
            )
        },
        BitTypes::Embedding => Loading::llama(ModelKind::Embedding, bit_pooling(bit)),
        _ => Loading::llama(ModelKind::Chat, None),
    })
}

/// The model a Bit asks for, or `None` when the host does not serve it.
fn hosting(metadata: &BitMetadata) -> Result<Option<Hosting>> {
    let bit = &metadata.bit;
    let mut files = metadata.files_of(bit);
    if !hosts(bit) || (files.is_empty() && !bit.is_mlx_model()) {
        return Ok(None);
    }
    let loading = loading(metadata)?;
    files.extend(loading.extra);
    let imports = files
        .iter()
        .filter_map(|(descriptor, artifact)| {
            artifact.clone().map(|path| (descriptor.clone(), path))
        })
        .collect();
    let spec = ModelSpec {
        display_name: display_name(bit),
        kind: loading.kind,
        engine: loading.engine,
        assets: files
            .into_iter()
            .map(|(descriptor, _)| descriptor)
            .collect(),
        projector: loading.projector,
        pooling: loading.pooling,
    };
    spec.validate()
        .with_context(|| format!("Bit {} does not describe a servable model", bit.id))?;
    Ok(Some(Hosting { spec, imports }))
}

fn read_metadata(config: &PlacementConfig, bit_id: &str) -> Result<Option<BitMetadata>> {
    let Some(pin) = config.bit_pins.iter().find(|pin| pin.bit_id == bit_id) else {
        return Ok(None);
    };
    let path = config
        .project_path
        .join("bits")
        .join("metadata")
        .join(format!("{bit_id}.json"));
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .with_context(|| format!("Open the metadata of Bit {bit_id}"))?
        .take(PACKAGED_BIT_METADATA_MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let packaged = crate::project_artifacts::packaged_metadata(&bytes, pin)?;
    BitMetadata::typed(packaged).map(Some)
}

/// A small artifact file of the committed revision, checked against its pinned size and sha256.
fn read_pinned_file(root: &Path, asset: &ModelAssetDescriptor, path: &str) -> Result<Vec<u8>> {
    validate_artifact_relative_path(path)?;
    ensure!(
        asset.size <= IMPORT_MAX_BYTES,
        "Artifact file {path} is too large to import into the model store"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(root.join(path))
        .with_context(|| format!("Open artifact file {path}"))?
        .take(asset.size + 1)
        .read_to_end(&mut bytes)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    ensure!(
        bytes.len() as u64 == asset.size && sha256 == asset.digest.hex,
        "Artifact file {path} differs from its pinned size or sha256"
    );
    Ok(bytes)
}

fn store_bytes(store: &ModelStore, asset: &ModelAssetDescriptor, bytes: &[u8]) -> Result<()> {
    let _reservation = store.reserve(&asset.digest, asset.size)?;
    let mut partial = store.open_partial(&asset.digest)?;
    partial.set_len(0)?;
    std::io::Write::write_all(&mut partial, bytes)?;
    partial.sync_all()?;
    store.publish(asset)
}

/// Copies a small artifact file of the committed revision into the store, verified.
fn import(store: &ModelStore, root: &Path, asset: &ModelAssetDescriptor, path: &str) -> Result<()> {
    if store.contains(&asset.digest, asset.size)? {
        return Ok(());
    }
    store_bytes(store, asset, &read_pinned_file(root, asset, path)?)
}

async fn import_all(
    store: &Arc<ModelStore>,
    root: &Path,
    imports: Vec<(ModelAssetDescriptor, String)>,
) -> Result<()> {
    let (store, root) = (Arc::clone(store), root.to_owned());
    tokio::task::spawn_blocking(move || {
        imports
            .iter()
            .try_for_each(|(asset, path)| import(&store, &root, asset, path))
    })
    .await?
}

/// The endpoint of one of the placement's pinned Bits, or `None` when the device's model
/// host does not serve it. A `device` Bit of this device whose model is gone fails with
/// [`OwnModelMissing`].
pub async fn host_for_placement(
    config: &PlacementConfig,
    device_id: &str,
    bit_id: &str,
) -> Result<Option<PlacementEndpoint>> {
    let host = ModelHost::current().context("The device's model host is not running")?;
    endpoint_for(&host, config, device_id, bit_id).await
}

pub(crate) async fn endpoint_for(
    host: &ModelHost,
    config: &PlacementConfig,
    device_id: &str,
    bit_id: &str,
) -> Result<Option<PlacementEndpoint>> {
    let Some(metadata) = read_metadata(config, bit_id)? else {
        return Ok(None);
    };
    let Some(model) = hosted_model(host, config, &metadata, device_id).await? else {
        return Ok(None);
    };
    Ok(Some(PlacementEndpoint {
        base_url: host.gateway().base_url(),
        bearer: host.gateway().tokens().issue_for(&config.id, &model)?,
        model,
    }))
}

/// The hosted model that serves a pinned Bit: the model a `device` Bit of this device names,
/// or one hosted on demand for a v2 Local Bit.
async fn hosted_model(
    host: &ModelHost,
    config: &PlacementConfig,
    metadata: &BitMetadata,
    device_id: &str,
) -> Result<Option<String>> {
    if let Some(named) = own_device_model(&metadata.bit, device_id) {
        return match named? {
            Some(model) if host.supervisor().model(&model).is_none() => Err(OwnModelMissing.into()),
            named => Ok(named),
        };
    }
    if !metadata.v2 {
        return Ok(None);
    }
    let Some(hosting) = hosting(metadata)? else {
        return Ok(None);
    };
    reference_imports(host.store(), &config.id, &hosting.imports)?;
    import_all(host.store(), &config.project_path, hosting.imports).await?;
    host.supervisor().ensure_hosted(hosting.spec).map(Some)
}

/// A placement references the files it imports as well as its model-store assets, so the
/// supervisor sees which hosted models placements still use.
fn reference_imports(
    store: &ModelStore,
    placement: &str,
    imports: &[(ModelAssetDescriptor, String)],
) -> Result<()> {
    let owner = AssetOwner::new(OwnerKind::Placement, placement)?;
    imports
        .iter()
        .try_for_each(|(asset, _)| store.add_ref(&asset.digest, &owner))
}

/// For a `device` Bit, the hosted model it names when it names this device.
fn own_device_model(bit: &Bit, device_id: &str) -> Option<Result<Option<String>>> {
    let target = DeviceModelTarget::from_bit(bit)?;
    Some(target.map(|target| (target.device_id == device_id).then_some(target.model)))
}

/// The answer for a `device` Bit that names this device and a model it no longer hosts.
#[derive(Debug)]
pub(crate) struct OwnModelMissing;

impl std::fmt::Display for OwnModelMissing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("This device does not host the model its Bit names")
    }
}

impl std::error::Error for OwnModelMissing {}

/// The typed failure a placement's call of such a Bit reports: `model_missing`, not a device
/// it cannot reach.
fn own_model_missing(bit: &Bit) -> flow_like_types::Error {
    match DeviceModelTarget::from_bit(bit) {
        Some(Ok(target)) => {
            ModelUnavailable::for_target(ModelUnavailableReason::ModelMissing, &target, None).into()
        }
        Some(Err(error)) => error,
        None => OwnModelMissing.into(),
    }
}

/// The agent's answers by Bit, each trusted for [`ROUTE_TTL`]: asking again re-hosts a model
/// that was removed while the placement ran, and finds one hosted since.
#[derive(Default)]
struct RouteCache(HashMap<String, (Instant, Option<ModelEndpoint>)>);

impl RouteCache {
    fn fresh(&self, bit_id: &str, now: Instant) -> Option<Option<ModelEndpoint>> {
        let (asked, answer) = self.0.get(bit_id)?;
        (now.saturating_duration_since(*asked) < ROUTE_TTL).then(|| answer.clone())
    }

    /// The last answer, however old, for when the agent cannot be asked.
    fn last(&self, bit_id: &str) -> Option<Option<ModelEndpoint>> {
        self.0.get(bit_id).map(|(_, answer)| answer.clone())
    }

    fn keep(&mut self, bit_id: &str, answer: Option<ModelEndpoint>, now: Instant) {
        self.0.insert(bit_id.to_owned(), (now, answer));
    }
}

/// The placement side: asks its supervisor channel per Bit and keeps the answer a while.
pub struct PlacementRouter {
    broker: Arc<crate::ipc::ChildBroker>,
    answers: tokio::sync::Mutex<RouteCache>,
}

static PLACEMENT_ROUTER: OnceLock<Arc<PlacementRouter>> = OnceLock::new();

/// Set once in a supervised placement process, before its runtime state is built.
pub fn install_placement_router(broker: Arc<crate::ipc::ChildBroker>) {
    let _ = PLACEMENT_ROUTER.set(Arc::new(PlacementRouter {
        broker,
        answers: tokio::sync::Mutex::default(),
    }));
}

pub fn placement_router() -> Option<Arc<dyn LocalModelRouter>> {
    PLACEMENT_ROUTER
        .get()
        .map(|router| Arc::clone(router) as Arc<dyn LocalModelRouter>)
}

#[async_trait]
impl LocalModelRouter for PlacementRouter {
    async fn route(&self, bit: &Bit) -> Option<flow_like_types::Result<ModelEndpoint>> {
        let mut answers = self.answers.lock().await;
        if let Some(answer) = answers.fresh(&bit.id, Instant::now()) {
            return answer.map(Ok);
        }
        match self.broker.host_model(&bit.id).await {
            Ok(answer) => {
                let endpoint = answer.map(|endpoint| ModelEndpoint {
                    base_url: endpoint.base_url,
                    bearer: endpoint.bearer.to_string(),
                    model: endpoint.model,
                });
                answers.keep(&bit.id, endpoint.clone(), Instant::now());
                endpoint.map(Ok)
            }
            Err(error) if error.is::<OwnModelMissing>() => Some(Err(own_model_missing(bit))),
            Err(error) => match answers.last(&bit.id) {
                Some(answer) => {
                    tracing::warn!(bit = %bit.id, "The device's model host did not answer again, so the last answer stays: {error:#}");
                    answer.map(Ok)
                }
                None => Some(Err(error.context(format!(
                    "Reach the device's model host for Bit {}",
                    bit.id
                )))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_runtime::bit::Metadata;
    use serde_json::json;

    fn descriptor(fill: char, name: &str) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Blake3,
                hex: fill.to_string().repeat(64),
            },
            size: 4,
            file_name: name.into(),
            sources: vec!["https://cdn.flow-like.com/bits/a".into()],
        }
    }

    fn bit(id: &str, kind: &str, file: Option<&str>, parameters: serde_json::Value) -> Bit {
        let mut bit = Bit {
            id: id.into(),
            bit_type: serde_json::from_value(json!(kind)).expect("a Bit type"),
            file_name: file.map(str::to_owned),
            hash: format!("{id}-hash"),
            size: Some(4),
            parameters,
            ..Bit::default()
        };
        bit.meta.insert(
            "en".into(),
            Metadata {
                name: "Qwen3 8B".into(),
                ..Metadata::default()
            },
        );
        bit
    }

    fn llm(provider: &str, params: serde_json::Value) -> serde_json::Value {
        json!({"context_length": 8192,
               "model_classification": flow_like_runtime::bit::BitModelClassification::default(),
               "provider": {"provider_name": provider, "model_id": null, "version": null,
                            "params": params}})
    }

    fn metadata(
        bit: Bit,
        dependencies: Vec<Bit>,
        assets: Vec<(String, ModelAssetDescriptor)>,
    ) -> BitMetadata {
        BitMetadata {
            v2: true,
            bit,
            dependencies,
            assets: assets
                .into_iter()
                .map(|(bit_id, descriptor)| PackagedBitAsset { bit_id, descriptor })
                .collect(),
            artifacts: Vec::new(),
        }
    }

    /// Metadata as a placement reads it: parsed and checked by the protocol's strict type.
    fn committed(metadata: serde_json::Value) -> Result<BitMetadata> {
        let packaged: PackagedBitMetadata = serde_json::from_value(metadata)?;
        let id = packaged.bit()["id"].as_str().unwrap_or_default().to_owned();
        packaged.validate(&id)?;
        BitMetadata::typed(packaged)
    }

    /// The artifact file of a Bit, with a sha256 distinct per path.
    fn artifact(bit: &Bit) -> ProjectArtifactFile {
        let path = format!("bits/{}/{}", bit.hash, bit.file_name.as_deref().unwrap());
        ProjectArtifactFile {
            sha256: format!("{:x}", Sha256::digest(path.as_bytes())),
            size: bit.size.unwrap(),
            path,
        }
    }

    #[test]
    fn the_host_serves_local_models_with_files_only() {
        let local = llm("Local", json!({}));
        assert!(hosts(&bit("qwen", "Llm", Some("qwen.gguf"), local.clone())));
        assert!(hosts(&bit(
            "llava",
            "Vlm",
            Some("llava.gguf"),
            local.clone()
        )));
        assert!(!hosts(&bit("bare", "Llm", None, local)));
        assert!(!hosts(&bit(
            "gpt",
            "Llm",
            Some("gpt.gguf"),
            llm("openai", json!({}))
        )));
        let embedding = |file| {
            let parameters = json!({"languages": [], "vector_length": 384, "input_length": 512,
                "prefix": {"query": "", "paragraph": ""}, "pooling": "Mean",
                "provider": {"provider_name": "Local"}});
            hosts(&bit("embed", "Embedding", Some(file), parameters))
        };
        assert!(embedding("nomic.Q4_K_M.GGUF"));
        assert!(embedding("model.onnx"));
        assert!(!embedding("model.safetensors"));
    }

    #[test]
    fn vision_projectors_may_travel_as_artifacts_of_v2_metadata() -> Result<()> {
        let projection = json!({"projection": {"download_link": "https://cdn.flow-like.com/bits/mmproj",
                                               "file_name": "mmproj.gguf", "size": 4}});
        let root = bit("llava", "Vlm", Some("llava.gguf"), llm("Local", projection));
        let projector = root.projection_bit().expect("a projector Bit");
        let meta = committed(json!({
            "version": 2,
            "bit": root,
            "dependencies": [projector],
            "assets": [{"bit_id": "llava", "descriptor": descriptor('a', "llava.gguf")}],
            "artifacts": [artifact(&projector)],
        }))?;
        let hosted = hosting(&meta)?.expect("a hosted model");
        assert_eq!(hosted.spec.kind, ModelKind::Vision);
        assert_eq!(hosted.spec.projector.as_deref(), Some("mmproj.gguf"));
        assert_eq!(hosted.spec.assets.len(), 2);
        assert_eq!(
            hosted.imports,
            vec![(hosted.spec.assets[1].clone(), artifact(&projector).path)]
        );

        let without = committed(json!({
            "version": 2,
            "bit": bit("llava", "Vlm", Some("llava.gguf"), llm("Local", json!({}))),
            "assets": [{"bit_id": "llava", "descriptor": descriptor('a', "llava.gguf")}],
        }))?;
        assert!(hosting(&without).is_err());
        Ok(())
    }

    #[test]
    fn local_chat_bits_become_llama_specs_with_their_split_parts() -> Result<()> {
        let root = bit(
            "qwen",
            "Llm",
            Some("qwen-00001-of-00002.gguf"),
            llm("Local", json!({})),
        );
        let hosting = hosting(&metadata(
            root,
            vec![],
            vec![
                ("qwen".into(), descriptor('a', "qwen-00001-of-00002.gguf")),
                ("qwen".into(), descriptor('b', "qwen-00002-of-00002.gguf")),
            ],
        ))?
        .expect("a hosted model");
        assert_eq!(hosting.spec.engine, ModelEngine::Llamacpp);
        assert_eq!(hosting.spec.kind, ModelKind::Chat);
        assert_eq!(hosting.spec.assets.len(), 2);
        assert_eq!(hosting.spec.display_name, "Qwen3 8B");
        assert!(hosting.imports.is_empty());
        Ok(())
    }

    #[test]
    fn onnx_embeddings_collect_their_tokenizer_files() -> Result<()> {
        let parameters = json!({"languages": [], "vector_length": 384, "input_length": 512,
            "prefix": {"query": "", "paragraph": ""}, "pooling": "CLS",
            "provider": {"provider_name": "Local"}});
        let root = bit("minilm", "Embedding", Some("model.onnx"), parameters);
        let dependencies: Vec<Bit> = [
            ("tok", "Tokenizer", "tokenizer.json"),
            ("cfg", "Config", "config.json"),
            ("tcfg", "TokenizerConfig", "tokenizer_config.json"),
            ("stm", "SpecialTokensMap", "special_tokens_map.json"),
        ]
        .into_iter()
        .map(|(id, kind, file)| bit(id, kind, Some(file), json!({})))
        .collect();
        let mut meta = metadata(
            root,
            dependencies,
            vec![("minilm".into(), descriptor('c', "model.onnx"))],
        );
        meta.artifacts = meta.dependencies.iter().map(artifact).collect();
        let hosting = hosting(&meta)?.expect("a hosted model");
        assert_eq!(hosting.spec.engine, ModelEngine::Onnx);
        assert_eq!(hosting.spec.pooling, Some(ModelPooling::Cls));
        assert_eq!(hosting.spec.assets.len(), 5);
        assert_eq!(hosting.imports.len(), 4);
        assert!(hosting.imports.iter().any(|(asset, path)| {
            asset.file_name == "tokenizer.json" && path == "bits/tok-hash/tokenizer.json"
        }));
        Ok(())
    }

    #[test]
    fn mlx_bits_load_every_dependency_file_on_apple_silicon() -> Result<()> {
        let files = [
            ("Config", "config.json"),
            ("Tokenizer", "tokenizer.json"),
            ("TokenizerConfig", "tokenizer_config.json"),
            ("File", "model-00001-of-00002.safetensors"),
            ("File", "model-00002-of-00002.safetensors"),
        ];
        let dependencies: Vec<Bit> = files
            .iter()
            .enumerate()
            .map(|(index, (kind, file))| bit(&format!("part-{index}"), kind, Some(file), json!({})))
            .collect();
        let assets = dependencies
            .iter()
            .zip(['a', 'b', 'c', 'd', 'e'])
            .map(|(part, fill)| {
                (
                    part.id.clone(),
                    descriptor(fill, part.file_name.as_deref().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        let apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        for (kind, served) in [("Llm", ModelKind::Chat), ("Vlm", ModelKind::Vision)] {
            let root = bit("qwen-mlx", kind, None, llm("MLX", json!({})));
            assert_eq!(hosts(&root), apple_silicon);
            let meta = metadata(root, dependencies.clone(), assets.clone());
            let Some(hosted) = hosting(&meta)? else {
                assert!(!apple_silicon);
                continue;
            };
            assert_eq!(hosted.spec.engine, ModelEngine::Mlx);
            assert_eq!(hosted.spec.kind, served);
            let names: Vec<_> = hosted
                .spec
                .assets
                .iter()
                .map(|asset| asset.file_name.as_str())
                .collect();
            assert_eq!(names, files.map(|(_, file)| file));
            assert!(hosted.imports.is_empty());
        }
        Ok(())
    }

    #[test]
    fn other_providers_and_v1_bits_stay_in_process() -> Result<()> {
        let openai = bit("gpt", "Llm", None, llm("openai", json!({})));
        assert!(own_device_model(&openai, "dev-1").is_none());
        assert!(hosting(&metadata(openai, vec![], vec![]))?.is_none());
        let own = bit(
            "remote",
            "Llm",
            None,
            llm("device", json!({"device_id": "dev-1", "model": "qwen"})),
        );
        let named = |device| own_device_model(&own, device).expect("a device Bit");
        assert_eq!(named("dev-1")?.as_deref(), Some("qwen"));
        assert_eq!(named("dev-2")?, None);
        let unnamed = bit(
            "bare",
            "Llm",
            None,
            llm("device", json!({"device_id": "dev-1"})),
        );
        assert!(
            own_device_model(&unnamed, "dev-1")
                .expect("a device Bit")
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn an_own_bit_whose_model_is_gone_fails_as_missing_not_unreachable() {
        let own = bit(
            "remote",
            "Llm",
            None,
            llm("device", json!({"device_id": "dev-1", "model": "qwen"})),
        );
        let error = own_model_missing(&own);
        let unavailable =
            flow_like_runtime::models::device::model_unavailable(&error).expect("a typed reason");
        assert_eq!(unavailable.reason, ModelUnavailableReason::ModelMissing);
        assert_eq!(
            unavailable.message,
            "Model 'Qwen3 8B' runs on device 'dev-1', which does not host this model."
        );
    }

    #[test]
    fn placements_ask_again_once_an_answer_is_old() {
        let endpoint = |model: &str| ModelEndpoint {
            base_url: "http://127.0.0.1:1/v1".into(),
            bearer: "flp_token".into(),
            model: model.into(),
        };
        let asked = Instant::now();
        let mut cache = RouteCache::default();
        assert!(cache.fresh("qwen", asked).is_none());
        cache.keep("qwen", Some(endpoint("auto-1a2b")), asked);
        cache.keep("remote", None, asked);
        let soon = asked + ROUTE_TTL - Duration::from_secs(1);
        let model = |answer: Option<Option<ModelEndpoint>>| answer.flatten().map(|e| e.model);
        assert_eq!(
            model(cache.fresh("qwen", soon)).as_deref(),
            Some("auto-1a2b")
        );
        assert_eq!(cache.fresh("remote", soon), Some(None));
        let later = asked + ROUTE_TTL;
        assert!(cache.fresh("qwen", later).is_none() && cache.fresh("remote", later).is_none());
        assert_eq!(model(cache.last("qwen")).as_deref(), Some("auto-1a2b"));
        cache.keep("qwen", Some(endpoint("auto-9f8e")), later);
        assert_eq!(
            model(cache.fresh("qwen", later)).as_deref(),
            Some("auto-9f8e")
        );
    }

    #[test]
    fn imports_verify_the_pinned_bytes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = ModelStore::open(
            directory.path(),
            super::super::store::ModelStoreConfig::default(),
        )?;
        let root = directory.path().join("project");
        std::fs::create_dir_all(root.join("bits/tok-hash"))?;
        std::fs::write(root.join("bits/tok-hash/tokenizer.json"), b"{}\n")?;
        let mut asset = ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: format!("{:x}", Sha256::digest(b"{}\n")),
            },
            size: 3,
            file_name: "tokenizer.json".into(),
            sources: vec![],
        };
        import(&store, &root, &asset, "bits/tok-hash/tokenizer.json")?;
        assert!(store.contains(&asset.digest, 3)?);
        asset.digest.hex = "e".repeat(64);
        assert!(import(&store, &root, &asset, "bits/tok-hash/tokenizer.json").is_err());
        assert!(import(&store, &root, &asset, "../escape").is_err());
        Ok(())
    }
}
