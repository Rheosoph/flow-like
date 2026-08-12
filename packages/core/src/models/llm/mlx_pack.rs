//! Materializes the individually cached files of an MLX model into one directory.
//!
//! Bits are stored as `<hash>/<file_name>`, while MLX loaders expect the layout of
//! a Hugging Face repository. This module builds that layout without moving or
//! mutating the downloaded artifacts.

use crate::bit::{Bit, BitPack, BitTypes, safe_mlx_asset_path};
use flow_like_storage::{Path as StorePath, files::store::local_store::LocalObjectStore};
use flow_like_types::{Result, anyhow};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const MLX_MODEL_CACHE_DIRECTORY: &str = "mlx-models";
const CACHE_FORMAT_VERSION: &str = "flow-like-mlx-pack-v1";

/// The MLX loader family required by a materialized model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MlxModelKind {
    Llm,
    Vlm,
}

impl MlxModelKind {
    fn from_root_bit(root: &Bit) -> Result<Self> {
        match root.bit_type {
            BitTypes::Llm => Ok(Self::Llm),
            BitTypes::Vlm => Ok(Self::Vlm),
            _ => Err(anyhow!(
                "MLX model root {} must be an LLM or VLM bit",
                root.id
            )),
        }
    }

    fn cache_label(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Vlm => "vlm",
        }
    }
}

/// A validated local model directory ready to be passed to an MLX loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedMlxModel {
    pub path: PathBuf,
    pub kind: MlxModelKind,
}

#[derive(Clone, Debug)]
struct MlxAsset {
    bit_id: String,
    bit_hash: String,
    relative_path: PathBuf,
    portable_key: String,
    source_path: PathBuf,
    file_size: u64,
}

/// Builds a deterministic Hugging Face-style directory for an MLX LLM or VLM.
///
/// `pack` may be the direct result of [`Bit::pack`], which conventionally contains
/// `root` as well as its dependencies. The matching root entry is ignored once;
/// distinct bits that resolve to the same relative path are rejected.
///
/// Concrete assets must already be downloaded into `bit_store`. Files are
/// hard-linked into a same-filesystem temporary directory when possible, copied
/// otherwise, validated, and atomically renamed into the deterministic cache
/// location.
pub fn materialize_mlx_model(
    root: &Bit,
    pack: &BitPack,
    bit_store: &Arc<LocalObjectStore>,
) -> Result<MaterializedMlxModel> {
    let kind = MlxModelKind::from_root_bit(root)?;
    let (store_root, cache_root) = prepare_cache_root(bit_store)?;
    let assets = collect_assets(root, pack, bit_store, &store_root)?;

    validate_asset_manifest(&assets, kind)?;
    validate_json_assets(&assets, None)?;

    let cache_key = model_cache_key(root, kind, &assets);
    let model_directory = cache_root.join(cache_key);

    if model_directory.exists() {
        validate_materialized_directory(&model_directory, &assets)?;
        validate_json_assets(&assets, Some(&model_directory))?;
        return Ok(MaterializedMlxModel {
            path: model_directory,
            kind,
        });
    }

    let staging = tempfile::Builder::new()
        .prefix(".mlx-model-")
        .tempdir_in(&cache_root)
        .map_err(|error| {
            anyhow!(
                "Failed to create an MLX model staging directory in {}: {}",
                cache_root.display(),
                error
            )
        })?;

    for asset in assets.values() {
        let destination = staging.path().join(&asset.relative_path);
        let parent = destination.parent().ok_or_else(|| {
            anyhow!(
                "MLX asset {} has no destination parent",
                asset.relative_path.display()
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            anyhow!(
                "Failed to create MLX model directory {}: {}",
                parent.display(),
                error
            )
        })?;

        if fs::hard_link(&asset.source_path, &destination).is_err() {
            let copied = fs::copy(&asset.source_path, &destination).map_err(|error| {
                anyhow!(
                    "Failed to copy MLX asset {} to {}: {}",
                    asset.source_path.display(),
                    destination.display(),
                    error
                )
            })?;
            if copied != asset.file_size {
                return Err(anyhow!(
                    "Copied MLX asset {} has size {}, expected {}",
                    asset.relative_path.display(),
                    copied,
                    asset.file_size
                ));
            }
        }
    }

    validate_materialized_directory(staging.path(), &assets)?;
    validate_json_assets(&assets, Some(staging.path()))?;

    match fs::rename(staging.path(), &model_directory) {
        Ok(()) => {}
        // Another materializer may have won the race. Never replace a directory;
        // accept the winner only after applying the same strict validation.
        Err(_) if model_directory.exists() => {
            validate_materialized_directory(&model_directory, &assets)?;
            validate_json_assets(&assets, Some(&model_directory))?;
        }
        Err(error) => {
            return Err(anyhow!(
                "Failed to atomically publish MLX model directory {}: {}",
                model_directory.display(),
                error
            ));
        }
    }

    Ok(MaterializedMlxModel {
        path: model_directory,
        kind,
    })
}

fn prepare_cache_root(bit_store: &Arc<LocalObjectStore>) -> Result<(PathBuf, PathBuf)> {
    let cache_root = bit_store
        .path_to_filesystem(&StorePath::from(MLX_MODEL_CACHE_DIRECTORY))
        .map_err(|error| anyhow!("Failed to resolve the local MLX cache path: {}", error))?;
    let store_root = cache_root.parent().ok_or_else(|| {
        anyhow!(
            "Local MLX cache path {} has no store root",
            cache_root.display()
        )
    })?;
    let store_root = fs::canonicalize(store_root).map_err(|error| {
        anyhow!(
            "Failed to resolve local bits store {}: {}",
            store_root.display(),
            error
        )
    })?;

    match fs::symlink_metadata(&cache_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(anyhow!(
                "MLX cache path {} must not be a symlink",
                cache_root.display()
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(anyhow!(
                "MLX cache path {} is not a directory",
                cache_root.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(&cache_root) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(anyhow!(
                        "Failed to create MLX cache directory {}: {}",
                        cache_root.display(),
                        error
                    ));
                }
            }
        }
        Err(error) => {
            return Err(anyhow!(
                "Failed to inspect MLX cache directory {}: {}",
                cache_root.display(),
                error
            ));
        }
    }

    // Recheck after creation because another process may have won the
    // NotFound/create race with a non-directory or symlink.
    let cache_metadata = fs::symlink_metadata(&cache_root).map_err(|error| {
        anyhow!(
            "Failed to inspect MLX cache directory {} after creation: {}",
            cache_root.display(),
            error
        )
    })?;
    if cache_metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "MLX cache path {} must not be a symlink",
            cache_root.display()
        ));
    }
    if !cache_metadata.is_dir() {
        return Err(anyhow!(
            "MLX cache path {} is not a directory",
            cache_root.display()
        ));
    }

    let canonical_cache_root = fs::canonicalize(&cache_root).map_err(|error| {
        anyhow!(
            "Failed to resolve MLX cache directory {}: {}",
            cache_root.display(),
            error
        )
    })?;
    if !canonical_cache_root.starts_with(&store_root) {
        return Err(anyhow!(
            "MLX cache directory {} escapes the local bits store",
            canonical_cache_root.display()
        ));
    }

    Ok((store_root, canonical_cache_root))
}

fn collect_assets(
    root: &Bit,
    pack: &BitPack,
    bit_store: &Arc<LocalObjectStore>,
    store_root: &Path,
) -> Result<BTreeMap<String, MlxAsset>> {
    let mut assets = BTreeMap::new();
    add_asset(root, bit_store, store_root, &mut assets)?;

    for bit in &pack.bits {
        if same_bit_artifact(root, bit) {
            continue;
        }
        add_asset(bit, bit_store, store_root, &mut assets)?;
    }

    if assets.is_empty() {
        return Err(anyhow!(
            "MLX model {} has no downloaded file assets",
            root.id
        ));
    }

    for key in assets.keys() {
        for (separator_index, _) in key.match_indices('/') {
            let ancestor = &key[..separator_index];
            if assets.contains_key(ancestor) {
                return Err(anyhow!(
                    "MLX asset paths {} and {} conflict because one is a file parent of the other",
                    ancestor,
                    key
                ));
            }
        }
    }

    Ok(assets)
}

fn add_asset(
    bit: &Bit,
    bit_store: &Arc<LocalObjectStore>,
    store_root: &Path,
    assets: &mut BTreeMap<String, MlxAsset>,
) -> Result<()> {
    let Some(file_name) = bit.file_name.as_deref() else {
        if bit.download_link.is_some() {
            return Err(anyhow!(
                "Downloadable MLX asset bit {} has no file_name",
                bit.id
            ));
        }
        return Ok(());
    };

    let (relative_path, portable_key) = safe_mlx_asset_path(file_name).map_err(|error| {
        anyhow!(
            "Unsafe MLX file_name {:?} for bit {}: {}",
            file_name,
            bit.id,
            error
        )
    })?;

    if let Some(existing) = assets.get(&portable_key) {
        return Err(anyhow!(
            "Duplicate MLX asset path {} from bits {} and {}",
            relative_path.display(),
            existing.bit_id,
            bit.id
        ));
    }

    let source_path = bit.to_path(bit_store).ok_or_else(|| {
        anyhow!(
            "Failed to resolve local path for MLX asset bit {} ({})",
            bit.id,
            relative_path.display()
        )
    })?;
    let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
        anyhow!(
            "MLX asset {} for bit {} is not downloaded at {}: {}",
            relative_path.display(),
            bit.id,
            source_path.display(),
            error
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "MLX asset {} for bit {} must not be a symlink",
            relative_path.display(),
            bit.id
        ));
    }
    if !metadata.is_file() {
        return Err(anyhow!(
            "MLX asset {} for bit {} is not a regular file",
            relative_path.display(),
            bit.id
        ));
    }

    let canonical_source = fs::canonicalize(&source_path).map_err(|error| {
        anyhow!(
            "Failed to resolve MLX asset {} for bit {}: {}",
            source_path.display(),
            bit.id,
            error
        )
    })?;
    if !canonical_source.starts_with(store_root) {
        return Err(anyhow!(
            "MLX asset {} for bit {} escapes the local bits store",
            canonical_source.display(),
            bit.id
        ));
    }

    if let Some(expected_size) = bit.size
        && expected_size != metadata.len()
    {
        return Err(anyhow!(
            "MLX asset {} for bit {} has size {}, expected {}",
            relative_path.display(),
            bit.id,
            metadata.len(),
            expected_size
        ));
    }

    assets.insert(
        portable_key.clone(),
        MlxAsset {
            bit_id: bit.id.clone(),
            bit_hash: bit.hash.clone(),
            relative_path,
            portable_key,
            source_path: canonical_source,
            file_size: metadata.len(),
        },
    );
    Ok(())
}

fn same_bit_artifact(left: &Bit, right: &Bit) -> bool {
    left.id == right.id
        && left.hub == right.hub
        && left.hash == right.hash
        && left.file_name == right.file_name
}

fn validate_asset_manifest(assets: &BTreeMap<String, MlxAsset>, kind: MlxModelKind) -> Result<()> {
    if !has_exact_root_file(assets, "config.json") {
        return Err(anyhow!("MLX model is missing required config.json"));
    }

    if !assets
        .values()
        .any(|asset| has_extension(&asset.relative_path, "safetensors"))
    {
        return Err(anyhow!(
            "MLX model must contain at least one .safetensors weight file"
        ));
    }

    if !has_exact_root_file(assets, "tokenizer.json") {
        return Err(anyhow!("MLX model is missing required tokenizer.json"));
    }
    if !has_exact_root_file(assets, "tokenizer_config.json") {
        return Err(anyhow!(
            "MLX model is missing required tokenizer_config.json"
        ));
    }

    if kind == MlxModelKind::Vlm
        && !has_exact_root_file(assets, "processor_config.json")
        && !has_exact_root_file(assets, "preprocessor_config.json")
    {
        return Err(anyhow!(
            "MLX VLM is missing processor_config.json or preprocessor_config.json"
        ));
    }

    Ok(())
}

fn has_exact_root_file(assets: &BTreeMap<String, MlxAsset>, file_name: &str) -> bool {
    assets
        .get(file_name)
        .is_some_and(|asset| asset.relative_path == Path::new(file_name))
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn validate_json_assets(
    assets: &BTreeMap<String, MlxAsset>,
    materialized_root: Option<&Path>,
) -> Result<()> {
    for asset in assets.values() {
        if !has_extension(&asset.relative_path, "json") {
            continue;
        }
        let path = materialized_root.map_or_else(
            || asset.source_path.clone(),
            |root| root.join(&asset.relative_path),
        );
        let file = fs::File::open(&path).map_err(|error| {
            anyhow!(
                "Failed to open MLX JSON asset {}: {}",
                asset.relative_path.display(),
                error
            )
        })?;
        let mut deserializer = serde_json::Deserializer::from_reader(file);
        serde::de::IgnoredAny::deserialize(&mut deserializer).map_err(|error| {
            anyhow!(
                "MLX JSON asset {} is invalid: {}",
                asset.relative_path.display(),
                error
            )
        })?;
        deserializer.end().map_err(|error| {
            anyhow!(
                "MLX JSON asset {} has trailing data: {}",
                asset.relative_path.display(),
                error
            )
        })?;
    }
    Ok(())
}

fn model_cache_key(root: &Bit, kind: MlxModelKind, assets: &BTreeMap<String, MlxAsset>) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, CACHE_FORMAT_VERSION.as_bytes());
    hash_field(&mut hasher, kind.cache_label().as_bytes());
    hash_field(&mut hasher, root.hub.as_bytes());
    hash_field(&mut hasher, root.id.as_bytes());
    hash_field(&mut hasher, root.hash.as_bytes());
    hash_field(&mut hasher, root.dependency_tree_hash.as_bytes());
    if let Some(version) = &root.version {
        hash_field(&mut hasher, version.as_bytes());
    }
    for asset in assets.values() {
        hash_field(
            &mut hasher,
            asset.relative_path.to_string_lossy().as_bytes(),
        );
        hash_field(&mut hasher, asset.portable_key.as_bytes());
        hash_field(&mut hasher, asset.bit_hash.as_bytes());
        hash_field(&mut hasher, &asset.file_size.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn hash_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn validate_materialized_directory(
    directory: &Path,
    assets: &BTreeMap<String, MlxAsset>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        anyhow!(
            "Failed to inspect materialized MLX directory {}: {}",
            directory.display(),
            error
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "Materialized MLX directory {} must not be a symlink",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(anyhow!(
            "Materialized MLX path {} is not a directory",
            directory.display()
        ));
    }

    let expected_portable_keys = assets.keys().cloned().collect::<BTreeSet<_>>();
    let expected_paths = assets
        .values()
        .map(|asset| path_to_forward_slashes(&asset.relative_path))
        .collect::<BTreeSet<_>>();
    let mut actual_paths = BTreeSet::new();
    let mut actual_portable_keys = BTreeSet::new();
    collect_materialized_files(
        directory,
        directory,
        &expected_portable_keys,
        &mut actual_paths,
        &mut actual_portable_keys,
    )?;

    if actual_paths != expected_paths {
        let missing = expected_paths
            .difference(&actual_paths)
            .cloned()
            .collect::<Vec<_>>();
        let extra = actual_paths
            .difference(&expected_paths)
            .cloned()
            .collect::<Vec<_>>();
        return Err(anyhow!(
            "Materialized MLX directory does not match its asset manifest (missing: {:?}, extra: {:?})",
            missing,
            extra
        ));
    }

    for asset in assets.values() {
        let path = directory.join(&asset.relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            anyhow!(
                "Failed to inspect materialized MLX asset {}: {}",
                path.display(),
                error
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(anyhow!(
                "Materialized MLX asset {} is not a regular non-symlink file",
                path.display()
            ));
        }
        if metadata.len() != asset.file_size {
            return Err(anyhow!(
                "Materialized MLX asset {} has size {}, expected {}",
                path.display(),
                metadata.len(),
                asset.file_size
            ));
        }
    }

    Ok(())
}

fn collect_materialized_files(
    root: &Path,
    directory: &Path,
    expected_keys: &BTreeSet<String>,
    actual_paths: &mut BTreeSet<String>,
    actual_portable_keys: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        anyhow!(
            "Failed to read materialized MLX directory {}: {}",
            directory.display(),
            error
        )
    })? {
        let entry =
            entry.map_err(|error| anyhow!("Failed to read MLX directory entry: {}", error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            anyhow!(
                "Failed to inspect materialized MLX path {}: {}",
                path.display(),
                error
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "Materialized MLX path {} must not be a symlink",
                path.display()
            ));
        }

        let relative = path.strip_prefix(root).map_err(|error| {
            anyhow!(
                "Materialized MLX path {} escaped {}: {}",
                path.display(),
                root.display(),
                error
            )
        })?;
        let relative_string = relative.to_str().ok_or_else(|| {
            anyhow!(
                "Materialized MLX path {} is not valid UTF-8",
                relative.display()
            )
        })?;
        let (_, portable_key) = safe_mlx_asset_path(&relative_string.replace('\\', "/"))?;

        if metadata.is_dir() {
            let prefix = format!("{portable_key}/");
            if !expected_keys.iter().any(|key| key.starts_with(&prefix)) {
                return Err(anyhow!(
                    "Materialized MLX directory {} is not part of the asset manifest",
                    relative.display()
                ));
            }
            collect_materialized_files(
                root,
                &path,
                expected_keys,
                actual_paths,
                actual_portable_keys,
            )?;
        } else if metadata.is_file() {
            if !actual_portable_keys.insert(portable_key) {
                return Err(anyhow!(
                    "Materialized MLX directory contains colliding path {}",
                    relative.display()
                ));
            }
            actual_paths.insert(path_to_forward_slashes(relative));
        } else {
            return Err(anyhow!(
                "Materialized MLX path {} is not a regular file or directory",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn path_to_forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_root(bit_type: BitTypes) -> Bit {
        Bit {
            id: "mlx-model".into(),
            bit_type,
            hash: "mlx-model-root".into(),
            dependency_tree_hash: "mlx-model-tree".into(),
            hub: "test".into(),
            ..Bit::default()
        }
    }

    fn asset_bit(id: &str, file_name: &str, bytes: &[u8]) -> Bit {
        Bit {
            id: id.into(),
            bit_type: BitTypes::Other,
            hash: format!("hash-{id}"),
            dependency_tree_hash: format!("hash-{id}"),
            file_name: Some(file_name.into()),
            download_link: Some(format!("https://example.invalid/{id}")),
            size: Some(bytes.len() as u64),
            hub: "test".into(),
            ..Bit::default()
        }
    }

    fn local_store() -> (tempfile::TempDir, Arc<LocalObjectStore>) {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(LocalObjectStore::new(temp.path().to_path_buf()).unwrap());
        (temp, store)
    }

    fn put_asset(store: &Arc<LocalObjectStore>, bit: &Bit, bytes: &[u8]) -> PathBuf {
        let path = bit.to_path(store).expect("asset path");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn valid_llm_assets() -> Vec<(Bit, Vec<u8>)> {
        vec![
            (
                asset_bit("config", "config.json", br#"{"model_type":"test"}"#),
                br#"{"model_type":"test"}"#.to_vec(),
            ),
            (
                asset_bit("weights", "weights/model.safetensors", b"safe tensor bytes"),
                b"safe tensor bytes".to_vec(),
            ),
            (
                asset_bit("tokenizer", "tokenizer.json", br#"{"version":"1.0"}"#),
                br#"{"version":"1.0"}"#.to_vec(),
            ),
            (
                asset_bit(
                    "tokenizer-config",
                    "tokenizer_config.json",
                    br#"{"tokenizer_class":"PreTrainedTokenizerFast"}"#,
                ),
                br#"{"tokenizer_class":"PreTrainedTokenizerFast"}"#.to_vec(),
            ),
        ]
    }

    fn install_assets(store: &Arc<LocalObjectStore>, assets: &[(Bit, Vec<u8>)]) -> BitPack {
        for (bit, bytes) in assets {
            put_asset(store, bit, bytes);
        }
        BitPack {
            bits: assets.iter().map(|(bit, _)| bit.clone()).collect(),
        }
    }

    #[test]
    fn materializes_llm_pack_with_deterministic_relative_layout() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let assets = valid_llm_assets();
        let mut pack = install_assets(&store, &assets);
        // `Bit::pack` includes the root. Its conventional repetition is not a
        // duplicate dependency path.
        pack.bits.push(root.clone());
        pack.bits.reverse();

        let first = materialize_mlx_model(&root, &pack, &store).unwrap();
        assert_eq!(first.kind, MlxModelKind::Llm);
        assert_eq!(
            fs::read(first.path.join("weights/model.safetensors")).unwrap(),
            b"safe tensor bytes"
        );
        assert!(first.path.join("config.json").is_file());
        assert!(first.path.join("tokenizer.json").is_file());

        let second = materialize_mlx_model(&root, &pack, &store).unwrap();
        assert_eq!(first, second);

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let source = assets
                .iter()
                .find(|(bit, _)| bit.id == "weights")
                .and_then(|(bit, _)| bit.to_path(&store))
                .unwrap();
            assert_eq!(
                fs::metadata(source).unwrap().ino(),
                fs::metadata(first.path.join("weights/model.safetensors"))
                    .unwrap()
                    .ino(),
                "same-filesystem assets should be hard-linked"
            );
        }
    }

    #[test]
    fn rejects_unsafe_portable_paths() {
        for unsafe_name in [
            "/config.json",
            "../config.json",
            "nested/../config.json",
            "nested//config.json",
            r"C:\config.json",
            "volume:config.json",
        ] {
            let (_temp, store) = local_store();
            let root = model_root(BitTypes::Llm);
            let bit = asset_bit("unsafe", unsafe_name, b"{}");
            let pack = BitPack { bits: vec![bit] };

            let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
            assert!(
                error.to_string().contains("Unsafe MLX file_name"),
                "{unsafe_name:?}: {error}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_and_file_parent_paths() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let first = asset_bit("first-config", "config.json", b"{}");
        let second = asset_bit("second-config", "CONFIG.JSON", b"{}");
        put_asset(&store, &first, b"{}");
        put_asset(&store, &second, b"{}");

        let error = materialize_mlx_model(
            &root,
            &BitPack {
                bits: vec![first, second],
            },
            &store,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Duplicate MLX asset path"));

        let (_temp, store) = local_store();
        let file = asset_bit("tokenizer-file", "tokenizer", b"file");
        let interloper = asset_bit("tokenizer-interloper", "tokenizer-json", b"file");
        let child = asset_bit("tokenizer-child", "tokenizer/tokenizer.json", b"{}");
        put_asset(&store, &file, b"file");
        put_asset(&store, &interloper, b"file");
        put_asset(&store, &child, b"{}");
        let error = materialize_mlx_model(
            &root,
            &BitPack {
                // `tokenizer-json` sorts between `tokenizer` and
                // `tokenizer/tokenizer.json`; adjacent-only comparisons miss
                // this parent conflict.
                bits: vec![file, interloper, child],
            },
            &store,
        )
        .unwrap_err();
        assert!(error.to_string().contains("file parent"));
    }

    #[test]
    fn validates_required_llm_assets_and_json() {
        let cases = [
            ("config", "missing required config.json"),
            ("weights", "at least one .safetensors"),
            ("tokenizer", "missing required tokenizer.json"),
            ("tokenizer-config", "missing required tokenizer_config.json"),
        ];

        for (omitted_id, expected_error) in cases {
            let (_temp, store) = local_store();
            let root = model_root(BitTypes::Llm);
            let assets = valid_llm_assets()
                .into_iter()
                .filter(|(bit, _)| bit.id != omitted_id)
                .collect::<Vec<_>>();
            let pack = install_assets(&store, &assets);
            let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
            assert!(
                error.to_string().contains(expected_error),
                "{omitted_id}: {error}"
            );
        }

        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let mut assets = valid_llm_assets();
        let (config, bytes) = assets
            .iter_mut()
            .find(|(bit, _)| bit.id == "config")
            .unwrap();
        *bytes = b"{ invalid".to_vec();
        config.size = Some(bytes.len() as u64);
        let pack = install_assets(&store, &assets);
        let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
        assert!(error.to_string().contains("config.json is invalid"));
    }

    #[test]
    fn accepts_and_validates_vlm_processor_configs() {
        for config_name in ["processor_config.json", "preprocessor_config.json"] {
            let (_temp, store) = local_store();
            let root = model_root(BitTypes::Vlm);
            let mut assets = valid_llm_assets();
            let bytes = br#"{"image_processor_type":"test"}"#.to_vec();
            assets.push((asset_bit("processor", config_name, &bytes), bytes.clone()));
            let pack = install_assets(&store, &assets);

            let materialized = materialize_mlx_model(&root, &pack, &store).unwrap();
            assert_eq!(materialized.kind, MlxModelKind::Vlm);
            assert!(materialized.path.join(config_name).is_file());
        }

        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Vlm);
        let assets = valid_llm_assets();
        let pack = install_assets(&store, &assets);
        let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("processor_config.json or preprocessor_config.json")
        );

        let (_temp, store) = local_store();
        let mut assets = valid_llm_assets();
        let bytes = b"{ broken".to_vec();
        assets.push((
            asset_bit("processor", "preprocessor_config.json", &bytes),
            bytes,
        ));
        let pack = install_assets(&store, &assets);
        let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("preprocessor_config.json is invalid")
        );
    }

    #[test]
    fn rejects_non_file_and_symlink_sources() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let assets = valid_llm_assets();
        let directory_bit = assets
            .iter()
            .find(|(bit, _)| bit.id == "weights")
            .map(|(bit, _)| bit.clone())
            .unwrap();
        for (bit, bytes) in &assets {
            if bit.id != directory_bit.id {
                put_asset(&store, bit, bytes);
            }
        }
        let directory_path = directory_bit.to_path(&store).unwrap();
        fs::create_dir_all(&directory_path).unwrap();
        let pack = BitPack {
            bits: assets.iter().map(|(bit, _)| bit.clone()).collect(),
        };
        let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
        assert!(error.to_string().contains("is not a regular file"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let (_temp, store) = local_store();
            let assets = valid_llm_assets();
            let config = assets
                .iter()
                .find(|(bit, _)| bit.id == "config")
                .map(|(bit, _)| bit.clone())
                .unwrap();
            for (bit, bytes) in &assets {
                if bit.id != config.id {
                    put_asset(&store, bit, bytes);
                }
            }
            let target = store
                .path_to_filesystem(&StorePath::from("symlink-target.json"))
                .unwrap();
            fs::write(&target, b"{}").unwrap();
            let config_path = config.to_path(&store).unwrap();
            fs::create_dir_all(config_path.parent().unwrap()).unwrap();
            symlink(target, config_path).unwrap();

            let pack = BitPack {
                bits: assets.iter().map(|(bit, _)| bit.clone()).collect(),
            };
            let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
            assert!(error.to_string().contains("must not be a symlink"));
        }
    }

    #[test]
    fn rejects_non_model_roots() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Other);
        let error = materialize_mlx_model(&root, &BitPack { bits: vec![] }, &store).unwrap_err();
        assert!(error.to_string().contains("must be an LLM or VLM bit"));
    }

    #[test]
    fn legacy_tokenizer_substitutes_are_rejected() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let mut assets = valid_llm_assets()
            .into_iter()
            .filter(|(bit, _)| bit.id != "tokenizer")
            .collect::<Vec<_>>();
        assets.extend([
            (
                asset_bit("vocab", "vocab.json", br#"{"hello":0}"#),
                br#"{"hello":0}"#.to_vec(),
            ),
            (
                asset_bit("merges", "merges.txt", b"#version: 0.2\n"),
                b"#version: 0.2\n".to_vec(),
            ),
        ]);
        let pack = install_assets(&store, &assets);
        let error = materialize_mlx_model(&root, &pack, &store).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing required tokenizer.json")
        );
    }

    #[test]
    fn safetensors_extension_is_case_insensitive() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let mut assets = valid_llm_assets();
        let (weights, _) = assets
            .iter_mut()
            .find(|(bit, _)| bit.id == "weights")
            .unwrap();
        weights.file_name = Some("weights/model.SAFETENSORS".to_string());
        let pack = install_assets(&store, &assets);

        let materialized = materialize_mlx_model(&root, &pack, &store).unwrap();
        assert!(
            materialized
                .path
                .join("weights/model.SAFETENSORS")
                .is_file()
        );
    }

    #[test]
    fn cache_manifest_is_order_independent() {
        let (_temp, store) = local_store();
        let root = model_root(BitTypes::Llm);
        let assets = valid_llm_assets();
        let pack = install_assets(&store, &assets);
        let first = materialize_mlx_model(&root, &pack, &store).unwrap();

        let mut reverse_pack = pack.clone();
        reverse_pack.bits.reverse();
        let second = materialize_mlx_model(&root, &reverse_pack, &store).unwrap();
        assert_eq!(first.path, second.path);

        let files = fs::read_dir(first.path.parent().unwrap())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
    }
}
