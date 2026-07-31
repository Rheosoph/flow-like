use crate::flow::execution::context::ExecutionContext;
use crate::state::FlowLikeState;
use crate::utils::compression::{compress_to_file_json, from_compressed_json};
use crate::utils::download::download_bit;
use flow_like_model_provider::history::History;
use flow_like_model_provider::llm::{CompletionClientDyn, CompletionModelHandle};
use flow_like_model_provider::provider::{
    EmbeddingModelProvider, ImageEmbeddingModelProvider, ModelProvider,
};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::files::store::local_store::LocalObjectStore;
use flow_like_types::Value;
use flow_like_types::intercom::InterComCallback;

use rig::agent::AgentBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use url::Url;

const NAME_HINT_WEIGHT: f32 = 0.2; // weight of name similarity for best model preference
const NAME_HINT_SIMILARITY_THRESHOLD: f32 = 0.5; // minimum required similarity score to model name
pub const MLX_PROVIDER_NAME: &str = "MLX";
const INLINE_MLX_ASSET_IDENTITY_DOMAIN: &[u8] = b"flow-like-inline-mlx-asset-v1";
const MLX_RUNTIME_MODEL_IDENTITY_DOMAIN: &[u8] = b"flow-like-mlx-runtime-model-v1";
const USER_SOURCE_ARTIFACT_IDENTITY_DOMAIN: &[u8] = b"flow-like-user-source-artifact-v1";
const USER_SOURCE_PACK_IDENTITY_DOMAIN: &[u8] = b"flow-like-user-source-pack-v1";
const MLX_RUNTIME_MODEL_ID_PREFIX: &str = "mlx-source-";
const USER_SOURCE_ARTIFACT_ID_PREFIX: &str = "user-source-";
const USER_SOURCE_PACK_ID_PREFIX: &str = "user-source-pack-";
const MAX_INLINE_MLX_ASSETS: usize = 512;
const MAX_MLX_ASSET_PATH_BYTES: usize = 1_024;
const MAX_MLX_ASSET_PATH_COMPONENTS: usize = 64;
const MAX_MLX_ASSET_COMPONENT_BYTES: usize = 255;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineHuggingFaceMlxManifest {
    schema: u32,
    repo_id: String,
    revision: String,
    format: String,
    files: Vec<InlineHuggingFaceMlxFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineHuggingFaceMlxFile {
    path: String,
    size: u64,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    oid: Option<String>,
    #[serde(default)]
    lfs_oid: Option<String>,
}

struct ResolvedInlineMlxAsset {
    download_link: String,
    file_name: String,
    size: u64,
}

/// MLX Swift is only supported by the application on physical Apple-silicon
/// devices. Keep this check in core so downloads and runtime construction
/// cannot be enabled accidentally by a frontend-only capability flag.
pub const fn can_host_mlx() -> bool {
    cfg!(all(
        target_arch = "aarch64",
        any(
            target_os = "macos",
            all(
                target_os = "ios",
                not(any(target_abi = "sim", target_abi = "macabi"))
            )
        )
    ))
}

fn valid_huggingface_repo_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    if bytes.is_empty() || bytes.len() > 96 {
        return false;
    }
    if !bytes
        .first()
        .is_some_and(|value| value.is_ascii_alphanumeric())
        || !bytes
            .last()
            .is_some_and(|value| value.is_ascii_alphanumeric())
    {
        return false;
    }
    bytes
        .iter()
        .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
}

fn validate_huggingface_repo_id(repo_id: &str) -> flow_like_types::Result<()> {
    if repo_id.trim() != repo_id {
        return Err(flow_like_types::anyhow!(
            "Hugging Face repo_id must not contain surrounding whitespace"
        ));
    }
    let mut components = repo_id.split('/');
    let owner = components.next().unwrap_or_default();
    let repository = components.next().unwrap_or_default();
    if components.next().is_some()
        || !valid_huggingface_repo_component(owner)
        || !valid_huggingface_repo_component(repository)
    {
        return Err(flow_like_types::anyhow!(
            "Hugging Face repo_id must have the form owner/repository"
        ));
    }
    Ok(())
}

fn validate_huggingface_revision(revision: &str) -> flow_like_types::Result<()> {
    if !(40..=64).contains(&revision.len())
        || !revision.bytes().all(|value| value.is_ascii_hexdigit())
    {
        return Err(flow_like_types::anyhow!(
            "Hugging Face revision must be a full hexadecimal commit SHA"
        ));
    }
    Ok(())
}

fn huggingface_pinned_download_url(
    repo_id: &str,
    revision: &str,
    file_name: &str,
) -> flow_like_types::Result<String> {
    let mut url = Url::parse("https://huggingface.co").map_err(|error| {
        flow_like_types::anyhow!("Failed to construct Hugging Face download URL: {}", error)
    })?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            flow_like_types::anyhow!("Failed to construct Hugging Face download path")
        })?;
        for component in repo_id.split('/') {
            segments.push(component);
        }
        segments.push("resolve");
        segments.push(revision);
        for component in file_name.split('/') {
            segments.push(component);
        }
    }
    url.set_query(Some("download=true"));
    Ok(url.to_string())
}

fn update_source_identity_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn inline_mlx_asset_id(asset: &ResolvedInlineMlxAsset) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INLINE_MLX_ASSET_IDENTITY_DOMAIN);
    update_source_identity_field(&mut hasher, asset.download_link.as_bytes());
    update_source_identity_field(&mut hasher, asset.file_name.as_bytes());
    update_source_identity_field(&mut hasher, &asset.size.to_le_bytes());
    format!("mlx-inline-{}", hasher.finalize().to_hex())
}

fn user_source_artifact_identity(
    bit_type: &BitTypes,
    download_link: Option<&str>,
    file_name: Option<&str>,
    size: Option<u64>,
) -> Option<String> {
    let artifact_kind = match bit_type {
        BitTypes::Llm => b"llm".as_slice(),
        BitTypes::Vlm => b"vlm".as_slice(),
        BitTypes::Projection => b"projection".as_slice(),
        _ => return None,
    };
    let download_link = download_link?.trim();
    let file_name = file_name?.trim();
    if download_link.is_empty() || file_name.is_empty() {
        return None;
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(USER_SOURCE_ARTIFACT_IDENTITY_DOMAIN);
    update_source_identity_field(&mut hasher, artifact_kind);
    update_source_identity_field(&mut hasher, download_link.as_bytes());
    update_source_identity_field(&mut hasher, file_name.as_bytes());
    match size {
        Some(size) => {
            update_source_identity_field(&mut hasher, b"size");
            update_source_identity_field(&mut hasher, &size.to_le_bytes());
        }
        None => update_source_identity_field(&mut hasher, b"unknown-size"),
    }
    Some(format!(
        "{}{}",
        USER_SOURCE_ARTIFACT_ID_PREFIX,
        hasher.finalize().to_hex()
    ))
}

fn is_prefixed_blake3_identity(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn safe_mlx_asset_path(file_name: &str) -> flow_like_types::Result<(PathBuf, String)> {
    if file_name.is_empty() {
        return Err(flow_like_types::anyhow!("path is empty"));
    }
    if file_name.len() > MAX_MLX_ASSET_PATH_BYTES {
        return Err(flow_like_types::anyhow!(
            "path exceeds the {} byte limit",
            MAX_MLX_ASSET_PATH_BYTES
        ));
    }
    if file_name.contains('\\') {
        return Err(flow_like_types::anyhow!(
            "backslash path separators are not portable"
        ));
    }
    if file_name.starts_with('/') {
        return Err(flow_like_types::anyhow!("absolute paths are not allowed"));
    }

    let mut path = PathBuf::new();
    let mut key_parts = Vec::new();
    for (index, component) in file_name.split('/').enumerate() {
        if index >= MAX_MLX_ASSET_PATH_COMPONENTS {
            return Err(flow_like_types::anyhow!(
                "path exceeds the {} component limit",
                MAX_MLX_ASSET_PATH_COMPONENTS
            ));
        }
        if component.is_empty() {
            return Err(flow_like_types::anyhow!(
                "empty path components are not allowed"
            ));
        }
        if component.len() > MAX_MLX_ASSET_COMPONENT_BYTES {
            return Err(flow_like_types::anyhow!(
                "path component exceeds the {} byte limit",
                MAX_MLX_ASSET_COMPONENT_BYTES
            ));
        }
        if component == "." || component == ".." {
            return Err(flow_like_types::anyhow!(
                "current or parent traversal is not allowed"
            ));
        }
        if component.contains('\0') {
            return Err(flow_like_types::anyhow!("NUL bytes are not allowed"));
        }
        if component.contains(':') {
            return Err(flow_like_types::anyhow!(
                "colon characters are not portable in file names"
            ));
        }
        path.push(component);
        key_parts.push(component.to_ascii_lowercase());
    }

    Ok((path, key_parts.join("/")))
}

fn inline_mlx_asset_bit_type(file_name: &str) -> BitTypes {
    let base_name = file_name
        .rsplit('/')
        .next()
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    match base_name.as_str() {
        "config.json" => BitTypes::Config,
        "tokenizer.json"
        | "tokenizer.model"
        | "sentencepiece.bpe.model"
        | "spiece.model"
        | "vocab.json"
        | "vocab.txt"
        | "merges.txt" => BitTypes::Tokenizer,
        "tokenizer_config.json" => BitTypes::TokenizerConfig,
        "special_tokens_map.json" => BitTypes::SpecialTokensMap,
        "processor_config.json" | "preprocessor_config.json" => BitTypes::PreprocessorConfig,
        _ => BitTypes::File,
    }
}

fn resolve_inline_huggingface_mlx_manifest(
    value: Value,
) -> flow_like_types::Result<Vec<ResolvedInlineMlxAsset>> {
    let manifest: InlineHuggingFaceMlxManifest =
        flow_like_types::json::from_value(value).map_err(|error| {
            flow_like_types::anyhow!("Invalid Hugging Face MLX manifest: {}", error)
        })?;
    if manifest.schema != 1 {
        return Err(flow_like_types::anyhow!(
            "Unsupported Hugging Face MLX manifest schema {}; expected 1",
            manifest.schema
        ));
    }
    if manifest.format != "mlx" {
        return Err(flow_like_types::anyhow!(
            "Hugging Face manifest format must be \"mlx\""
        ));
    }
    validate_huggingface_repo_id(&manifest.repo_id)?;
    validate_huggingface_revision(&manifest.revision)?;
    if manifest.files.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Hugging Face MLX manifest must contain at least one file"
        ));
    }
    if manifest.files.len() > MAX_INLINE_MLX_ASSETS {
        return Err(flow_like_types::anyhow!(
            "Hugging Face MLX manifest contains {} files; the limit is {}",
            manifest.files.len(),
            MAX_INLINE_MLX_ASSETS
        ));
    }

    manifest
        .files
        .into_iter()
        .map(|file| {
            if file.path.trim() != file.path {
                return Err(flow_like_types::anyhow!(
                    "Hugging Face MLX file paths must not contain surrounding whitespace"
                ));
            }
            safe_mlx_asset_path(&file.path)?;
            if file.size == 0 {
                return Err(flow_like_types::anyhow!(
                    "Hugging Face MLX file {} has a zero size",
                    file.path
                ));
            }

            // These optional source-integrity hints are deliberately retained in
            // the public manifest but are not used as local content hashes. Hub
            // OID formats may evolve independently of the pinned commit URL.
            let _source_hints = (file.role, file.oid, file.lfs_oid);
            let download_link = huggingface_pinned_download_url(
                &manifest.repo_id,
                &manifest.revision.to_ascii_lowercase(),
                &file.path,
            )?;
            Ok(ResolvedInlineMlxAsset {
                download_link,
                file_name: file.path,
                size: file.size,
            })
        })
        .collect()
}

fn validate_inline_mlx_assets(
    assets: &[ResolvedInlineMlxAsset],
    kind: &BitTypes,
) -> flow_like_types::Result<()> {
    if assets.is_empty() {
        return Err(flow_like_types::anyhow!(
            "MLX models require a Hugging Face file manifest"
        ));
    }

    let mut paths = HashMap::<String, String>::new();
    let mut total_size = 0u64;
    for asset in assets {
        let parsed_url = Url::parse(&asset.download_link).map_err(|_| {
            flow_like_types::anyhow!("MLX asset {} has an invalid download URL", asset.file_name)
        })?;
        if !matches!(parsed_url.scheme(), "http" | "https")
            || parsed_url.host_str().is_none()
            || !parsed_url.username().is_empty()
            || parsed_url.password().is_some()
        {
            return Err(flow_like_types::anyhow!(
                "MLX asset {} must use an absolute HTTP(S) URL without credentials",
                asset.file_name
            ));
        }
        if asset.size == 0 {
            return Err(flow_like_types::anyhow!(
                "MLX asset {} has a zero size",
                asset.file_name
            ));
        }
        total_size = total_size.checked_add(asset.size).ok_or_else(|| {
            flow_like_types::anyhow!("MLX asset manifest total size overflowed u64")
        })?;

        let (_, portable_key) = safe_mlx_asset_path(&asset.file_name)?;
        if let Some(existing) = paths.insert(portable_key, asset.file_name.clone()) {
            return Err(flow_like_types::anyhow!(
                "Duplicate MLX asset paths {:?} and {:?}",
                existing,
                asset.file_name
            ));
        }
    }

    for (portable_key, original_path) in &paths {
        let components = portable_key.split('/').collect::<Vec<_>>();
        for depth in 1..components.len() {
            let parent = components[..depth].join("/");
            if let Some(parent_path) = paths.get(&parent) {
                return Err(flow_like_types::anyhow!(
                    "MLX asset paths {:?} and {:?} conflict because one is a file parent of the other",
                    parent_path,
                    original_path
                ));
            }
        }
    }

    let has_exact_root_file = |file_name: &str| {
        paths
            .get(file_name)
            .is_some_and(|original| original == file_name)
    };
    if !has_exact_root_file("config.json") {
        return Err(flow_like_types::anyhow!(
            "MLX model is missing required config.json"
        ));
    }
    if !paths.keys().any(|path| path.ends_with(".safetensors")) {
        return Err(flow_like_types::anyhow!(
            "MLX model must contain at least one .safetensors weight file"
        ));
    }
    if !has_exact_root_file("tokenizer.json") {
        return Err(flow_like_types::anyhow!(
            "MLX model is missing required tokenizer.json"
        ));
    }
    if !has_exact_root_file("tokenizer_config.json") {
        return Err(flow_like_types::anyhow!(
            "MLX model is missing required tokenizer_config.json"
        ));
    }
    if *kind == BitTypes::Vlm
        && !has_exact_root_file("processor_config.json")
        && !has_exact_root_file("preprocessor_config.json")
    {
        return Err(flow_like_types::anyhow!(
            "MLX VLM is missing processor_config.json or preprocessor_config.json"
        ));
    }

    Ok(())
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Metadata {
    pub name: String,
    pub description: String,
    pub long_description: Option<String>,
    pub release_notes: Option<String>,
    pub tags: Vec<String>,
    pub use_case: Option<String>,
    pub icon: Option<String>,
    pub thumbnail: Option<String>,
    pub preview_media: Vec<String>,
    pub age_rating: Option<i32>,
    pub website: Option<String>,
    pub support_url: Option<String>,
    pub docs_url: Option<String>,
    pub organization_specific_values: Option<Vec<u8>>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub created_at: SystemTime,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub updated_at: SystemTime,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            name: "Unknown".to_string(),
            description: "No description".to_string(),
            long_description: None,
            release_notes: None,
            tags: vec![],
            use_case: None,
            icon: None,
            thumbnail: None,
            preview_media: vec![],
            age_rating: None,
            website: None,
            support_url: None,
            docs_url: None,
            organization_specific_values: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }
}

const MEDIA_URL_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24);

/// Signs one `.webp` media asset stored under `prefix`, leaving values that are
/// already absolute URLs untouched.
///
/// Uses the cached signer so repeated calls return a byte-identical URL: the
/// media referenced here ends up in `<img src>` attributes, and a URL that
/// changes on every request defeats the browser cache and makes otherwise
/// unchanged metadata compare unequal on the client.
async fn presign_media_asset(name: &str, prefix: &Path, store: &FlowLikeStore) -> Option<String> {
    if name.starts_with("http://") || name.starts_with("https://") {
        return None;
    }

    let path = prefix.child(format!("{name}.webp"));
    store
        .sign_cached("GET", &path, MEDIA_URL_TTL)
        .await
        .ok()
        .map(|url| url.to_string())
}

impl Metadata {
    pub async fn presign(&mut self, prefix: Path, store: &FlowLikeStore) {
        if let Some(icon) = &self.icon
            && let Some(url) = presign_media_asset(icon, &prefix, store).await
        {
            self.icon = Some(url);
        }

        if let Some(thumbnail) = &self.thumbnail
            && let Some(url) = presign_media_asset(thumbnail, &prefix, store).await
        {
            self.thumbnail = Some(url);
        }

        for media in &mut self.preview_media {
            if let Some(url) = presign_media_asset(media, &prefix, store).await {
                *media = url;
            }
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum BitTypes {
    Llm,
    Vlm,
    Tts,
    Stt,
    Embedding,
    ImageEmbedding,
    File,
    Media,
    ImageGeneration,
    VideoGeneration,
    Template,
    Tokenizer,
    TokenizerConfig,
    SpecialTokensMap,
    Config,
    Course,
    PreprocessorConfig,
    Projection,
    Project,
    Board,
    #[default]
    Other,
    ObjectDetection,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct BitModelPreference {
    pub multimodal: Option<bool>,
    pub cost_weight: Option<f32>,
    pub speed_weight: Option<f32>,
    pub reasoning_weight: Option<f32>,
    pub creativity_weight: Option<f32>,
    pub factuality_weight: Option<f32>,
    pub function_calling_weight: Option<f32>,
    pub safety_weight: Option<f32>,
    pub openness_weight: Option<f32>,
    pub multilinguality_weight: Option<f32>,
    pub coding_weight: Option<f32>,
    pub model_hint: Option<String>,
}

fn enforce_bound(weight: &mut Option<f32>) {
    if let Some(w) = weight {
        *w = w.clamp(0.0, 1.0);
    }
}

impl BitModelPreference {
    fn normalize_weight(weight: &mut Option<f32>) {
        if let Some(w) = weight {
            if *w <= 0.0 {
                *weight = None;
            } else if *w > 1.0 {
                *weight = Some(1.0);
            }
        }
    }

    pub fn enforce_bounds(&mut self) {
        enforce_bound(&mut self.cost_weight);
        enforce_bound(&mut self.speed_weight);
        enforce_bound(&mut self.reasoning_weight);
        enforce_bound(&mut self.creativity_weight);
        enforce_bound(&mut self.factuality_weight);
        enforce_bound(&mut self.function_calling_weight);
        enforce_bound(&mut self.safety_weight);
        enforce_bound(&mut self.openness_weight);
        enforce_bound(&mut self.multilinguality_weight);
        enforce_bound(&mut self.coding_weight);
    }

    pub fn parse(&self) -> Self {
        let mut cloned = self.clone();
        cloned.inner_parse();
        cloned
    }

    fn inner_parse(&mut self) {
        Self::normalize_weight(&mut self.cost_weight);
        Self::normalize_weight(&mut self.speed_weight);
        Self::normalize_weight(&mut self.reasoning_weight);
        Self::normalize_weight(&mut self.creativity_weight);
        Self::normalize_weight(&mut self.factuality_weight);
        Self::normalize_weight(&mut self.function_calling_weight);
        Self::normalize_weight(&mut self.safety_weight);
        Self::normalize_weight(&mut self.openness_weight);
        Self::normalize_weight(&mut self.multilinguality_weight);
        Self::normalize_weight(&mut self.coding_weight);

        if let Some(model_hint) = &self.model_hint
            && model_hint.is_empty()
        {
            self.model_hint = None;
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct BitModelClassification {
    cost: f32,
    speed: f32,
    reasoning: f32,
    creativity: f32,
    factuality: f32,
    function_calling: f32,
    safety: f32,
    openness: f32,
    multilinguality: f32,
    coding: f32,
}

impl BitModelClassification {
    fn name_similarity(&self, hint: &str, bit: &Bit) -> flow_like_types::Result<f32> {
        let mut similarity: f32 = 0.0;

        if bit.meta.is_empty() {
            return Err(flow_like_types::anyhow!("No meta data found"));
        }

        for meta in bit.meta.values() {
            let local_similarity = strsim::jaro_winkler(&meta.name, hint) as f32;
            println!(
                "[BIT NAME SIMILARITY] similarity '{}' <-> '{}': {}",
                meta.name, hint, local_similarity
            );
            if local_similarity > similarity {
                similarity = local_similarity;
            }
        }

        let provider = bit.try_to_provider();
        match provider {
            Some(provider) => {
                if let Some(model_id) = provider.model_id {
                    let local_similarity = strsim::jaro_winkler(&model_id, hint) as f32;
                    println!(
                        "[BIT NAME SIMILARITY] similarity (provider) '{model_id}' <-> '{hint}': {local_similarity}"
                    );
                    if local_similarity > similarity {
                        similarity = local_similarity;
                    }
                }
            }
            None => return Ok(similarity),
        }
        Ok(similarity)
    }

    /// Calculates the score of the model in a range from 0 to 1 based on the provided preference
    pub fn score(&self, preference: &BitModelPreference, bit: &Bit) -> f32 {
        // If preference is multimodal but model doesn't support it return a score of 0
        if let Some(multimodal) = preference.multimodal
            && multimodal
            && !bit.is_multimodal()
        {
            return 0.0;
        }

        // Map weights to model fields dynamically
        let field_weight_pairs = vec![
            (preference.cost_weight, self.cost),
            (preference.speed_weight, self.speed),
            (preference.reasoning_weight, self.reasoning),
            (preference.creativity_weight, self.creativity),
            (preference.factuality_weight, self.factuality),
            (preference.function_calling_weight, self.function_calling),
            (preference.safety_weight, self.safety),
            (preference.openness_weight, self.openness),
            (preference.multilinguality_weight, self.multilinguality),
            (preference.coding_weight, self.coding),
        ];

        // Total accumulated preferences weights set by user
        let preferences_acc: f32 = field_weight_pairs.iter().filter_map(|(w, _)| *w).sum();

        // Model matching preferences accross all traits/characteristics
        let mut preference_match_score = 0.0;
        for (preference_weight, model_trait) in field_weight_pairs {
            if let Some(preference_weight) = preference_weight {
                preference_match_score += preference_weight * model_trait;
            }
        }

        // Model matching naming hint given by user (if any and if similarity is greater than threshold else 0.0)
        let name_match_score = preference
            .model_hint
            .as_ref()
            .and_then(|hint| self.name_similarity(hint, bit).ok())
            .filter(|&score| score > NAME_HINT_SIMILARITY_THRESHOLD)
            .unwrap_or(0.0);

        // Log results
        println!("[BIT SCORING] Accumulated Preference Weight: {preferences_acc}");
        println!("[BIT SCORING] Static Name Hint Weight: {NAME_HINT_WEIGHT}");
        println!("[BIT SCORING] Accumulated Preference Score: {preference_match_score}");
        println!("[BIT SCORING] Name Hint Score: {name_match_score}");

        // total score = match preferences + weighted match name
        let total_score = preference_match_score + (name_match_score * NAME_HINT_WEIGHT);
        // total weight = accumulated preference weights + static name weight
        let total_weight = preferences_acc + NAME_HINT_WEIGHT;

        // account for numerical stability
        if total_weight > 0.001 {
            total_score / total_weight
        } else {
            0.0
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
#[serde(default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Bit {
    pub id: String,
    #[serde(rename = "type")]
    pub bit_type: BitTypes,
    pub meta: std::collections::HashMap<String, Metadata>,
    pub authors: Vec<String>,
    pub repository: Option<String>,
    pub download_link: Option<String>,
    pub file_name: Option<String>,
    pub hash: String,
    pub size: Option<u64>,
    pub hub: String,
    pub parameters: Value,
    pub version: Option<String>,
    pub license: Option<String>,
    pub dependencies: Vec<String>,
    pub dependency_tree_hash: String,
    pub created: String,
    pub updated: String,
    pub model_slug: Option<String>,
    pub model_evaluation: Option<LlmModelEvaluation>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LlmModelEvaluation {
    pub slug: String,
    pub name: String,
    pub release_date: Option<String>,
    pub creator_name: String,
    pub creator_slug: String,
    pub evaluations: Option<Value>,
    pub pricing: Option<Value>,
    pub median_output_tokens_per_second: Option<f64>,
    pub median_time_to_first_token_seconds: Option<f64>,
    pub median_time_to_first_answer_token: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct LLMParameters {
    pub context_length: u32,
    pub provider: ModelProvider,
    pub model_classification: BitModelClassification,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct VLMParameters {
    pub context_length: u32,
    pub provider: ModelProvider,
    pub model_classification: BitModelClassification,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum TtsModelType {
    #[default]
    Kokoro,
    OmniVoice,
    Qwen3Tts,
    VibeVoice,
    VibeVoiceRealtime,
    Voxtral,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum TtsRuntimePreference {
    #[default]
    Auto,
    Cpu,
    Metal,
    Cuda,
    Accelerate,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum TtsDTypePreference {
    #[default]
    Auto,
    F32,
    F16,
    BF16,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct TtsAssetRef {
    pub bit: String,
    pub relative_path: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TtsModelParameters {
    pub model_type: TtsModelType,
    pub provider: ModelProvider,
    pub default_language: Option<String>,
    pub languages: Vec<String>,
    pub default_voice: Option<String>,
    pub voices: Vec<String>,
    pub runtime: Option<TtsRuntimePreference>,
    pub dtype: Option<TtsDTypePreference>,
    pub assets: Vec<TtsAssetRef>,
}

impl Default for TtsModelParameters {
    fn default() -> Self {
        Self {
            model_type: TtsModelType::default(),
            provider: ModelProvider {
                provider_name: "local:any-tts".to_string(),
                model_id: None,
                version: None,
                params: None,
            },
            default_language: None,
            languages: Vec::new(),
            default_voice: None,
            voices: Vec::new(),
            runtime: Some(TtsRuntimePreference::Auto),
            dtype: Some(TtsDTypePreference::Auto),
            assets: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum SttModelType {
    WhisperTiny,
    WhisperTinyEn,
    WhisperBase,
    WhisperBaseEn,
    WhisperSmall,
    WhisperSmallEn,
    WhisperMedium,
    WhisperMediumEn,
    WhisperLargeV3,
    WhisperLargeV3Turbo,
    DistilWhisperMediumEn,
    DistilWhisperLargeV2,
    DistilWhisperLargeV3,
    OlmoAsrTinyEn,
    OlmoAsrBaseEn,
    OlmoAsrSmallEn,
    OlmoAsrMediumEn,
    OlmoAsrLargeEn,
    OlmoAsrLargeEnV2,
    Qwen3Asr17B,
    MoonshineBaseEn,
}

impl Default for SttModelType {
    fn default() -> Self {
        Self::WhisperLargeV3Turbo
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum SttRuntimePreference {
    #[default]
    Auto,
    Cpu,
    Metal,
    Cuda,
    Accelerate,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum SttDTypePreference {
    #[default]
    Auto,
    F32,
    F16,
    BF16,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct SttAssetRef {
    pub bit: String,
    pub relative_path: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SttModelParameters {
    pub model_type: SttModelType,
    pub provider: ModelProvider,
    pub default_language: Option<String>,
    pub languages: Vec<String>,
    pub runtime: Option<SttRuntimePreference>,
    pub dtype: Option<SttDTypePreference>,
    pub assets: Vec<SttAssetRef>,
}

impl Default for SttModelParameters {
    fn default() -> Self {
        Self {
            model_type: SttModelType::default(),
            provider: ModelProvider {
                provider_name: STT_LOCAL_PROVIDER.to_string(),
                model_id: None,
                version: None,
                params: None,
            },
            default_language: None,
            languages: Vec::new(),
            runtime: Some(SttRuntimePreference::Auto),
            dtype: Some(SttDTypePreference::Auto),
            assets: Vec::new(),
        }
    }
}

/// Provider name marking an `Stt` bit as a local any-speech-to-text model
/// (as opposed to a hosted/API speech-to-text provider bit).
pub const STT_LOCAL_PROVIDER: &str = "local:any-speech-to-text";

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BitPack {
    pub bits: Vec<Bit>,
}

async fn collect_dependencies(
    bit: &Bit,
    state: Arc<FlowLikeState>,
) -> flow_like_types::Result<Vec<Bit>> {
    let http_client = state.http_client.clone();
    let hub = crate::hub::Hub::new(&bit.hub, http_client.clone()).await?;
    let bit_id = bit.id.clone();
    let bits = hub.get_bit_dependencies(&bit_id).await?;
    Ok(bits)
}

impl BitPack {
    fn is_virtual_bit(bit: &Bit) -> bool {
        bit.download_link.is_none()
    }

    pub async fn get_installed(
        &self,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<Bit>> {
        let bits_store = FlowLikeState::bit_store(&state).await?.as_generic();

        let mut installed_bits = vec![];
        for bit in self.bits.iter() {
            if Self::is_virtual_bit(bit) {
                installed_bits.push(bit.clone());
                continue;
            }

            let file_name = match bit.file_name.clone() {
                Some(file_name) => Some(file_name),
                None => continue,
            };
            let file_name = file_name.unwrap();
            let bit_path = Path::from(bit.hash.clone()).child(file_name);
            let meta = match bits_store.head(&bit_path).await {
                Ok(meta) => meta,
                Err(_) => continue,
            };

            let size = meta.size as u64;
            if size != bit.size.unwrap_or(0) {
                continue;
            }
            installed_bits.push(bit.clone());
        }
        Ok(installed_bits)
    }

    pub async fn download(
        &self,
        state: Arc<FlowLikeState>,
        callback: InterComCallback,
    ) -> flow_like_types::Result<Vec<Bit>> {
        if self.bits.iter().any(Bit::is_mlx_model) && !can_host_mlx() {
            return Err(flow_like_types::anyhow!(
                "MLX models can only be downloaded on supported Apple-silicon macOS or iOS devices"
            ));
        }

        let mut deduplicated_bits = vec![];
        let mut deduplication_helper = HashSet::new();
        self.bits.iter().for_each(|bit| {
            // If there is no download link we treat it as a virtual / proxied bit.
            // These should count as a successful "download" operation from a UX perspective
            // so we simply don't schedule a download but DO include it in the returned list.
            if Self::is_virtual_bit(bit) {
                println!("Skipping network download for bit {}: no download link (proxied or empty model)", bit.id);
                // Do not attempt any download but keep it in the final success vector
                return;
            }

            if bit.size.is_none() || bit.file_name.is_none() {
                println!("Skipping bit {}: missing size or file_name", bit.id);
                return;
            }

            if bit.size.unwrap_or(0) == 0 {
                println!("Skipping bit {}: size is zero, cannot download", bit.id);
                return;
            }

            let artifact_key = (
                bit.hash.clone(),
                bit.file_name
                    .clone()
                    .expect("file_name was checked immediately above"),
            );
            if !deduplication_helper.insert(artifact_key) {
                println!(
                    "Skipping bit {}: duplicate hash/file_name artifact already queued",
                    bit.id
                );
                return;
            }

            deduplicated_bits.push(bit.clone());
        });

        // If there is nothing to actually download we still return success with the original bits
        // so the frontend can proceed (useful for empty / proxied models)
        if deduplicated_bits.is_empty() {
            println!(
                "No concrete bits to download; returning success (all bits were proxied or lacked downloadable artifacts)"
            );
            let filtered: Vec<Bit> = self
                .bits
                .iter()
                .filter(|b| Self::is_virtual_bit(b))
                .cloned()
                .collect();
            return Ok(filtered);
        }

        println!(
            "Downloading {} bits: {}",
            deduplicated_bits.len(),
            deduplicated_bits
                .iter()
                .map(|bit| bit.id.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let download_futures: Vec<_> = deduplicated_bits
            .iter()
            .map(|bit| download_bit(bit, state.clone(), 3, &callback))
            .collect();

        let results = futures::future::join_all(download_futures).await;

        for result in results {
            match result {
                Ok(_) => println!("Download succeeded"),
                Err(e) => eprintln!("Download failed: {e}"),
            }
        }

        // Combine successfully queued bits (deduplicated_bits) with any virtual bits (those without download links)
        let mut result = self
            .bits
            .iter()
            .filter(|b| Self::is_virtual_bit(b))
            .cloned()
            .collect::<Vec<_>>();
        result.extend(deduplicated_bits);
        Ok(result)
    }

    pub fn size(&self) -> u64 {
        let mut size = 0;
        let mut bits_considered = HashSet::new();
        for bit in self.bits.iter() {
            let artifact_key = (bit.hash.clone(), bit.file_name.clone());
            if !bits_considered.insert(artifact_key) {
                continue;
            }
            if bit.size.is_some() {
                size += bit.size.unwrap();
            }
        }
        size
    }

    pub async fn is_installed(&self, state: Arc<FlowLikeState>) -> flow_like_types::Result<bool> {
        let bits_store = FlowLikeState::bit_store(&state).await?.as_generic();
        let mut installed = true;
        for bit in self.bits.iter() {
            if Self::is_virtual_bit(bit) {
                continue;
            }

            let file_name = match bit.file_name.clone() {
                Some(file_name) => file_name,
                None => {
                    installed = false;
                    break;
                }
            };
            let bit_path = Path::from(bit.hash.clone()).child(file_name);
            let metadata = match bits_store.head(&bit_path).await {
                Ok(metadata) => metadata,
                Err(_) => {
                    installed = false;
                    break;
                }
            };
            if metadata.size as u64 != bit.size.unwrap_or(0) {
                installed = false;
                break;
            }
        }
        Ok(installed)
    }
}

impl Bit {
    /// Returns the deterministic cache identity for a user-referenced local
    /// artifact. The source URL is part of the identity, so changing a pinned
    /// Hugging Face revision cannot reuse an older same-name, same-size file.
    pub fn user_source_artifact_identity(&self) -> Option<String> {
        user_source_artifact_identity(
            &self.bit_type,
            self.download_link.as_deref(),
            self.file_name.as_deref(),
            self.size,
        )
    }

    /// Source-derived identities deliberately are not content checksums. Verify
    /// that an identity matches this Bit's current source fields before using
    /// the trust-on-first-use download contract.
    pub fn has_matching_user_source_artifact_identity(&self) -> bool {
        self.user_source_artifact_identity()
            .is_some_and(|identity| identity == self.hash)
    }

    /// Refresh cache identities for a user-owned llama.cpp root.
    ///
    /// Legacy roots use `hash == id`; newer UI saves may carry an older
    /// source-derived hash while being edited. Both are safe to replace. A
    /// caller-supplied content hash is retained and continues to be verified.
    pub fn normalize_user_local_artifact_identity(&mut self) {
        if !matches!(self.bit_type, BitTypes::Llm | BitTypes::Vlm)
            || self
                .try_to_provider()
                .is_none_or(|provider| provider.provider_name != "Local")
        {
            return;
        }

        let Some(source_identity) = self.user_source_artifact_identity() else {
            return;
        };
        if self.hash.is_empty()
            || self.hash == self.id
            || self.hash.starts_with(USER_SOURCE_ARTIFACT_ID_PREFIX)
        {
            self.hash = source_identity.clone();
        }

        let mut hasher = blake3::Hasher::new();
        hasher.update(USER_SOURCE_PACK_IDENTITY_DOMAIN);
        update_source_identity_field(&mut hasher, self.id.as_bytes());
        update_source_identity_field(&mut hasher, source_identity.as_bytes());
        update_source_identity_field(&mut hasher, self.hash.as_bytes());
        if let Some(projection) = self.projection_bit() {
            update_source_identity_field(&mut hasher, b"projection");
            update_source_identity_field(&mut hasher, projection.hash.as_bytes());
        } else {
            update_source_identity_field(&mut hasher, b"no-projection");
        }
        self.dependency_tree_hash = format!(
            "{}{}",
            USER_SOURCE_PACK_ID_PREFIX,
            hasher.finalize().to_hex()
        );
    }

    /// Normalize an edited local user Bit while distinguishing an intentional
    /// new checksum from a checksum merely carried over by an edit form.
    pub fn normalize_edited_user_local_artifact_identity(&mut self, previous: Option<&Bit>) {
        if let Some(previous) = previous {
            let source_changed =
                previous.user_source_artifact_identity() != self.user_source_artifact_identity();
            if source_changed && self.hash == previous.hash {
                self.hash.clear();
            }
        }
        self.normalize_user_local_artifact_identity();
    }

    /// Local user models include their source-pack identity in the runtime
    /// cache key. Curated and remote models retain the historical stable id.
    pub fn runtime_model_cache_key(&self) -> String {
        if is_prefixed_blake3_identity(&self.dependency_tree_hash, USER_SOURCE_PACK_ID_PREFIX) {
            return format!("{}@{}", self.id, self.dependency_tree_hash);
        }
        self.id.clone()
    }

    /// Returns the deterministic runtime cache key for an MLX model.
    ///
    /// Dependency-free user models derive their identity from the validated
    /// inline Hugging Face artifact manifest. Curated models use the immutable
    /// root and dependency-tree identities emitted by the registry. Display
    /// metadata and manifest file order deliberately do not affect this key.
    pub fn mlx_runtime_model_cache_key(&self) -> flow_like_types::Result<String> {
        if !self.is_mlx_model() {
            return Err(flow_like_types::anyhow!(
                "MLX runtime cache keys require an MLX LLM or VLM bit"
            ));
        }

        let mut hasher = blake3::Hasher::new();
        hasher.update(MLX_RUNTIME_MODEL_IDENTITY_DOMAIN);
        update_source_identity_field(
            &mut hasher,
            match self.bit_type {
                BitTypes::Llm => b"llm",
                BitTypes::Vlm => b"vlm",
                _ => unreachable!("is_mlx_model only accepts LLM and VLM bits"),
            },
        );

        let has_inline_manifest =
            self.dependencies.is_empty() && self.parameters.get("huggingface").is_some();
        if has_inline_manifest {
            update_source_identity_field(&mut hasher, b"inline-huggingface");
            let mut assets = self.inline_mlx_asset_bits()?;
            assets.sort_by(|left, right| {
                left.file_name
                    .cmp(&right.file_name)
                    .then_with(|| left.hash.cmp(&right.hash))
            });
            for asset in assets {
                let file_name = asset.file_name.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!("Inline MLX asset is missing its file name")
                })?;
                let download_link = asset.download_link.as_deref().ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Inline MLX asset {file_name} is missing its source URL"
                    )
                })?;
                let size = asset.size.ok_or_else(|| {
                    flow_like_types::anyhow!("Inline MLX asset {file_name} is missing its size")
                })?;
                update_source_identity_field(&mut hasher, file_name.as_bytes());
                update_source_identity_field(&mut hasher, download_link.as_bytes());
                update_source_identity_field(&mut hasher, asset.hash.as_bytes());
                update_source_identity_field(&mut hasher, &size.to_le_bytes());
            }
        } else {
            update_source_identity_field(&mut hasher, b"registry-dependency-tree");
            update_source_identity_field(&mut hasher, self.hub.as_bytes());
            update_source_identity_field(&mut hasher, self.hash.as_bytes());
            update_source_identity_field(&mut hasher, self.dependency_tree_hash.as_bytes());

            let mut dependencies = self
                .dependencies
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            dependencies.sort_unstable();
            for dependency in dependencies {
                update_source_identity_field(&mut hasher, dependency.as_bytes());
            }
        }

        Ok(format!(
            "{}@{}{}",
            self.id,
            MLX_RUNTIME_MODEL_ID_PREFIX,
            hasher.finalize().to_hex()
        ))
    }

    pub fn is_mlx_model(&self) -> bool {
        matches!(self.bit_type, BitTypes::Llm | BitTypes::Vlm)
            && self.try_to_provider().is_some_and(|provider| {
                provider
                    .provider_name
                    .eq_ignore_ascii_case(MLX_PROVIDER_NAME)
            })
    }

    /// Materialize a user-owned MLX source manifest as ordinary downloadable
    /// artifact Bits. Curated/store MLX roots use registry dependencies instead;
    /// this helper is for dependency-free roots whose immutable Hugging Face
    /// manifest is embedded at `parameters.huggingface`.
    ///
    /// Artifact IDs are derived from the pinned URL, exact repository-relative
    /// path, and expected size. Editing any of those fields therefore selects a
    /// fresh local cache target while identical artifacts remain deduplicable.
    pub fn inline_mlx_asset_bits(&self) -> flow_like_types::Result<Vec<Bit>> {
        if !self.is_mlx_model() {
            return Ok(vec![]);
        }
        let Some(manifest) = self.parameters.get("huggingface").cloned() else {
            return Ok(vec![]);
        };
        let assets = resolve_inline_huggingface_mlx_manifest(manifest)?;
        validate_inline_mlx_assets(&assets, &self.bit_type)?;

        Ok(assets
            .into_iter()
            .map(|asset| {
                let id = inline_mlx_asset_id(&asset);
                Bit {
                    id: id.clone(),
                    bit_type: inline_mlx_asset_bit_type(&asset.file_name),
                    authors: self.authors.clone(),
                    repository: self.repository.clone(),
                    download_link: Some(asset.download_link),
                    file_name: Some(asset.file_name),
                    hash: id.clone(),
                    size: Some(asset.size),
                    hub: self.hub.clone(),
                    version: self.version.clone(),
                    license: self.license.clone(),
                    dependency_tree_hash: id,
                    created: self.created.clone(),
                    updated: self.updated.clone(),
                    ..Bit::default()
                }
            })
            .collect())
    }

    pub fn try_to_llm(&self) -> Option<LLMParameters> {
        if self.bit_type == BitTypes::Llm {
            let parameters =
                flow_like_types::json::from_value::<LLMParameters>(self.parameters.clone());
            if parameters.is_err() {
                return None;
            }
            return Some(parameters.unwrap());
        }
        None
    }

    pub fn try_to_vlm(&self) -> Option<VLMParameters> {
        if self.bit_type == BitTypes::Vlm {
            let parameters =
                flow_like_types::json::from_value::<VLMParameters>(self.parameters.clone());
            if parameters.is_err() {
                return None;
            }
            return Some(parameters.unwrap());
        }
        None
    }

    pub fn try_to_tts(&self) -> Option<TtsModelParameters> {
        if self.bit_type == BitTypes::Tts {
            let parameters =
                flow_like_types::json::from_value::<TtsModelParameters>(self.parameters.clone());
            if parameters.is_err() {
                return None;
            }
            return Some(parameters.unwrap());
        }
        None
    }

    pub fn try_to_stt_provider(&self) -> Option<ModelProvider> {
        if self.bit_type == BitTypes::Stt {
            let parameters =
                flow_like_types::json::from_value::<LLMParameters>(self.parameters.clone()).ok()?;
            if parameters.provider.provider_name.starts_with("local:") {
                return None;
            }
            return Some(parameters.provider);
        }
        None
    }

    /// Parses an `Stt` bit as a local any-speech-to-text model. Returns `None`
    /// for hosted/API speech-to-text provider bits (use [`Self::try_to_stt_provider`]).
    pub fn try_to_stt(&self) -> Option<SttModelParameters> {
        if self.bit_type != BitTypes::Stt {
            return None;
        }
        let parameters =
            flow_like_types::json::from_value::<SttModelParameters>(self.parameters.clone())
                .ok()?;
        if parameters.provider.provider_name == STT_LOCAL_PROVIDER {
            Some(parameters)
        } else {
            None
        }
    }

    pub fn score(&self, preference: &BitModelPreference) -> flow_like_types::Result<f32> {
        if let Some(parameters) = self.try_to_llm() {
            return Ok(parameters.model_classification.score(preference, self));
        }

        if let Some(parameters) = self.try_to_vlm() {
            return Ok(parameters.model_classification.score(preference, self));
        }

        Err(flow_like_types::anyhow!("Not a Model"))
    }

    pub fn try_to_embedding(&self) -> Option<EmbeddingModelProvider> {
        if self.bit_type == BitTypes::Embedding {
            let parameters = flow_like_types::json::from_value::<EmbeddingModelProvider>(
                self.parameters.clone(),
            );
            if parameters.is_err() {
                return None;
            }
            return Some(parameters.unwrap());
        }
        None
    }

    pub fn try_to_image_embedding(&self) -> Option<ImageEmbeddingModelProvider> {
        if self.bit_type == BitTypes::ImageEmbedding {
            let parameters = flow_like_types::json::from_value::<ImageEmbeddingModelProvider>(
                self.parameters.clone(),
            );
            if parameters.is_err() {
                return None;
            }
            return Some(parameters.unwrap());
        }
        None
    }

    pub fn try_to_provider(&self) -> Option<ModelProvider> {
        if let Some(parameters) = self.try_to_llm() {
            return Some(parameters.provider);
        }

        if let Some(parameters) = self.try_to_vlm() {
            return Some(parameters.provider);
        }

        if let Some(provider) = self.try_to_stt_provider() {
            return Some(provider);
        }

        None
    }

    pub fn try_to_embedding_provider(&self) -> Option<ModelProvider> {
        if let Some(parameters) = self.try_to_embedding() {
            return Some(parameters.provider);
        }

        if let Some(parameters) = self.try_to_image_embedding() {
            return Some(parameters.provider);
        }

        None
    }

    pub fn try_to_context_length(&self) -> Option<u32> {
        if let Some(parameters) = self.try_to_llm() {
            return Some(parameters.context_length);
        }

        if let Some(parameters) = self.try_to_vlm() {
            return Some(parameters.context_length);
        }

        None
    }

    pub async fn dependencies(
        &self,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<BitPack> {
        let bits_store = FlowLikeState::bit_store(&state).await?.as_generic();

        let cache_key = if self.dependency_tree_hash.is_empty() {
            &self.id
        } else {
            &self.dependency_tree_hash
        };
        let cache_dir = Path::from("deps-cache").child(format!("bit-deps-{}.bin", cache_key));

        let metadata = bits_store.head(&cache_dir).await;

        if metadata.is_ok() {
            let file = from_compressed_json::<BitPack>(bits_store.clone(), cache_dir.clone()).await;
            if let Ok(file) = file {
                return Ok(file);
            }
        }

        let dependencies = collect_dependencies(self, state.clone()).await?;

        println!("Dependencies for {} found", self.id);

        let bit_pack = BitPack { bits: dependencies };
        let res = compress_to_file_json(bits_store, cache_dir, &bit_pack).await;
        if res.is_err() {
            println!(
                "Failed to compress dependencies for {}, err: {}",
                self.id,
                res.err().unwrap()
            );
        }

        Ok(bit_pack)
    }

    pub async fn pack(&self, state: Arc<FlowLikeState>) -> flow_like_types::Result<BitPack> {
        // A bit that declares no dependencies has none to fetch — and user-owned
        // bits have no hub entry to ask, so the round trip would only 404.
        let mut dependencies = if self.dependencies.is_empty() {
            BitPack { bits: vec![] }
        } else {
            self.dependencies(state).await?
        };
        if self.dependencies.is_empty() && self.is_mlx_model() {
            dependencies.bits.extend(self.inline_mlx_asset_bits()?);
        }
        dependencies.bits.push(self.clone());
        if let Some(projection) = self.projection_bit() {
            dependencies.bits.push(projection);
        }
        Ok(dependencies)
    }

    /// The multimodal projector a user-configured local model carries inline.
    ///
    /// Curated vision models ship their `mmproj` file as a `Projection` bit in
    /// their dependency tree. User bits have no dependency tree of their own, so
    /// they describe the projector under `provider.params.projection`, and it is
    /// materialised here — downloaded, cached and handed to `--mmproj` through
    /// exactly the same paths as a curated one.
    pub fn projection_bit(&self) -> Option<Bit> {
        let projection = self
            .try_to_provider()?
            .params?
            .get("projection")
            .cloned()
            .filter(|value| !value.is_null())?;

        let download_link = projection
            .get("download_link")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|link| !link.is_empty())?
            .to_string();
        let file_name = projection
            .get("file_name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())?
            .to_string();
        let size = projection.get("size").and_then(|value| value.as_u64());

        let id = user_source_artifact_identity(
            &BitTypes::Projection,
            Some(&download_link),
            Some(&file_name),
            size,
        )?;
        Some(Bit {
            // `hash == id` marks an artifact whose checksum is not known upfront.
            // The id itself is source-derived so an edited pinned URL selects a
            // fresh cache directory even when file name and size are unchanged.
            hash: id.clone(),
            dependency_tree_hash: id.clone(),
            id,
            bit_type: BitTypes::Projection,
            download_link: Some(download_link),
            file_name: Some(file_name),
            size,
            hub: self.hub.clone(),
            version: self.version.clone(),
            license: self.license.clone(),
            created: self.created.clone(),
            updated: self.updated.clone(),
            ..Bit::default()
        })
    }

    pub async fn is_installed(&self, state: Arc<FlowLikeState>) -> flow_like_types::Result<bool> {
        let pack = self.pack(state.clone()).await?;
        pack.is_installed(state).await
    }

    pub fn is_multimodal(&self) -> bool {
        self.bit_type == BitTypes::Vlm || self.bit_type == BitTypes::ImageEmbedding
    }

    pub fn to_path(&self, file_system: &Arc<LocalObjectStore>) -> Option<PathBuf> {
        let file_name = self.file_name.clone()?;
        let bit_path = Path::from(self.hash.clone()).child(file_name);
        let path = file_system.path_to_filesystem(&bit_path).ok()?;
        Some(path)
    }

    pub async fn agent<'a>(
        &self,
        context: &mut ExecutionContext,
        history: &Option<History>,
    ) -> flow_like_types::Result<AgentBuilder<CompletionModelHandle<'a>>> {
        let (model_name, additional_params, completion_client) =
            self.completion_model(context, history).await?;
        let mut agent_builder = completion_client.agent(&model_name);

        if let Some(additional_params) = additional_params {
            agent_builder = agent_builder.additional_params(additional_params);
        }

        Ok(agent_builder)
    }

    pub async fn completion_model<'a>(
        &self,
        context: &mut ExecutionContext,
        history: &Option<History>,
    ) -> flow_like_types::Result<(
        String,
        Option<flow_like_types::Value>,
        Box<dyn CompletionClientDyn + Send + Sync + 'a>,
    )> {
        let (model_name, additional_params, completion_client) = {
            let model_factory = context.app_state.model_factory.clone();
            let model = model_factory
                .lock()
                .await
                .build(
                    self,
                    context.app_state.clone(),
                    context.token.clone(),
                    context.model_usage_context(),
                )
                .await?;
            let additional_params = model.additional_params(history);
            let default_model = model.default_model().await.unwrap_or_default();
            let provider = model.provider().await?;
            let completion = provider.into_client();
            (default_model, additional_params, completion)
        };

        Ok((model_name, additional_params, completion_client))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_storage::files::store::local_store::LocalObjectStore;
    use flow_like_types::Value;
    use flow_like_types::tokio;

    fn local_vlm_bit(projection: Value) -> Bit {
        let mut params = std::collections::HashMap::new();
        params.insert("projection".to_string(), projection);
        let parameters = VLMParameters {
            context_length: 8192,
            provider: ModelProvider {
                provider_name: "Local".to_string(),
                model_id: None,
                version: None,
                params: Some(params),
            },
            model_classification: BitModelClassification::default(),
        };

        Bit {
            id: "my-vlm".into(),
            bit_type: BitTypes::Vlm,
            hash: "my-vlm".into(),
            dependency_tree_hash: "my-vlm".into(),
            hub: "https://api.flow-like.com".into(),
            download_link: Some("https://example.com/model.gguf".into()),
            file_name: Some("model.gguf".into()),
            size: Some(4_000),
            parameters: flow_like_types::json::to_value(parameters).unwrap(),
            ..Default::default()
        }
    }

    fn inline_mlx_files(
        weight_path: &str,
        weight_size: u64,
        include_processor: bool,
    ) -> Vec<Value> {
        let mut files = vec![
            flow_like_types::json::json!({
                "path": "config.json",
                "size": 100,
                "role": "config",
            }),
            flow_like_types::json::json!({
                "path": "tokenizer.json",
                "size": 200,
            }),
            flow_like_types::json::json!({
                "path": "tokenizer_config.json",
                "size": 300,
            }),
            flow_like_types::json::json!({
                "path": weight_path,
                "size": weight_size,
                "oid": "0123456789abcdef",
                "lfs_oid": "fedcba9876543210",
            }),
        ];
        if include_processor {
            files.push(flow_like_types::json::json!({
                "path": "processor_config.json",
                "size": 400,
            }));
        }
        files
    }

    fn inline_mlx_bit(
        bit_type: BitTypes,
        repo_id: &str,
        revision: &str,
        weight_path: &str,
        weight_size: u64,
        include_processor: bool,
    ) -> Bit {
        let provider = ModelProvider {
            provider_name: MLX_PROVIDER_NAME.to_string(),
            model_id: Some(repo_id.to_string()),
            version: Some(revision.to_string()),
            params: None,
        };
        let mut parameters = match bit_type {
            BitTypes::Vlm => flow_like_types::json::to_value(VLMParameters {
                context_length: 8192,
                provider,
                model_classification: BitModelClassification::default(),
            })
            .unwrap(),
            _ => flow_like_types::json::to_value(LLMParameters {
                context_length: 8192,
                provider,
                model_classification: BitModelClassification::default(),
            })
            .unwrap(),
        };
        parameters.as_object_mut().unwrap().insert(
            "huggingface".to_string(),
            flow_like_types::json::json!({
                "schema": 1,
                "repo_id": repo_id,
                "revision": revision,
                "format": "mlx",
                "files": inline_mlx_files(weight_path, weight_size, include_processor),
            }),
        );

        Bit {
            id: "my-inline-mlx".into(),
            bit_type,
            hash: "my-inline-mlx".into(),
            dependency_tree_hash: "my-inline-mlx".into(),
            hub: "https://api.flow-like.com".into(),
            repository: Some(format!("https://huggingface.co/{repo_id}")),
            download_link: None,
            file_name: None,
            size: Some(0),
            parameters,
            ..Bit::default()
        }
    }

    fn inline_asset<'a>(assets: &'a [Bit], file_name: &str) -> &'a Bit {
        assets
            .iter()
            .find(|asset| asset.file_name.as_deref() == Some(file_name))
            .expect("inline MLX asset")
    }

    #[test]
    fn inline_mlx_manifest_derives_pinned_typed_artifacts() {
        let revision = "a".repeat(40);
        let bit = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &revision,
            "weights/model shard.safetensors",
            4_000,
            false,
        );

        let assets = bit.inline_mlx_asset_bits().unwrap();
        assert_eq!(assets.len(), 4);
        assert_eq!(
            inline_asset(&assets, "config.json").bit_type,
            BitTypes::Config
        );
        assert_eq!(
            inline_asset(&assets, "tokenizer.json").bit_type,
            BitTypes::Tokenizer
        );
        assert_eq!(
            inline_asset(&assets, "tokenizer_config.json").bit_type,
            BitTypes::TokenizerConfig
        );
        let weights = inline_asset(&assets, "weights/model shard.safetensors");
        assert_eq!(weights.bit_type, BitTypes::File);
        assert_eq!(
            weights.download_link.as_deref(),
            Some(
                format!(
                    "https://huggingface.co/owner/model/resolve/{revision}/weights/model%20shard.safetensors?download=true"
                )
                .as_str()
            )
        );
        assert!(assets.iter().all(|asset| asset.hash == asset.id));
    }

    #[test]
    fn inline_mlx_artifact_identity_changes_with_source_path_or_size() {
        let revision = "a".repeat(40);
        let base = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &revision,
            "weights/model.safetensors",
            4_000,
            false,
        )
        .inline_mlx_asset_bits()
        .unwrap();
        let other_repo = inline_mlx_bit(
            BitTypes::Llm,
            "owner/other-model",
            &revision,
            "weights/model.safetensors",
            4_000,
            false,
        )
        .inline_mlx_asset_bits()
        .unwrap();
        let other_path = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &revision,
            "weights/renamed.safetensors",
            4_000,
            false,
        )
        .inline_mlx_asset_bits()
        .unwrap();
        let other_size = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &revision,
            "weights/model.safetensors",
            4_001,
            false,
        )
        .inline_mlx_asset_bits()
        .unwrap();

        let base_id = &inline_asset(&base, "weights/model.safetensors").id;
        assert_ne!(
            base_id,
            &inline_asset(&other_repo, "weights/model.safetensors").id
        );
        assert_ne!(
            base_id,
            &inline_asset(&other_path, "weights/renamed.safetensors").id
        );
        assert_ne!(
            base_id,
            &inline_asset(&other_size, "weights/model.safetensors").id
        );
    }

    #[test]
    fn inline_mlx_runtime_cache_key_tracks_the_pinned_manifest_source() {
        let first = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &"a".repeat(40),
            "weights/model.safetensors",
            4_000,
            false,
        );
        let mut reordered = first.clone();
        reordered.updated = "metadata-only-change".into();
        reordered.parameters["huggingface"]["files"]
            .as_array_mut()
            .expect("Hugging Face files")
            .reverse();
        let edited_revision = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &"b".repeat(40),
            "weights/model.safetensors",
            4_000,
            false,
        );

        let first_key = first.mlx_runtime_model_cache_key().unwrap();
        assert!(first_key.starts_with("my-inline-mlx@mlx-source-"));
        assert_eq!(
            first_key,
            reordered.mlx_runtime_model_cache_key().unwrap(),
            "manifest order and display metadata do not change the model source"
        );
        assert_ne!(
            first_key,
            edited_revision.mlx_runtime_model_cache_key().unwrap(),
            "a new pinned revision must not reuse the loaded MLX runtime"
        );
    }

    #[test]
    fn curated_mlx_runtime_cache_key_tracks_root_and_dependency_tree_identity() {
        let mut first = inline_mlx_bit(
            BitTypes::Vlm,
            "owner/model",
            &"a".repeat(40),
            "weights/model.safetensors",
            4_000,
            true,
        );
        first
            .parameters
            .as_object_mut()
            .expect("MLX parameters")
            .remove("huggingface");
        first.hash = "curated-root-hash".into();
        first.dependencies = vec!["hub:weights".into(), "hub:tokenizer".into()];
        first.dependency_tree_hash = "curated-tree-a".into();

        let mut reordered = first.clone();
        reordered.dependencies.reverse();
        reordered.updated = "metadata-only-change".into();
        let mut edited_tree = first.clone();
        edited_tree.dependency_tree_hash = "curated-tree-b".into();
        let mut edited_root = first.clone();
        edited_root.hash = "other-curated-root-hash".into();

        let first_key = first.mlx_runtime_model_cache_key().unwrap();
        assert_eq!(
            first_key,
            reordered.mlx_runtime_model_cache_key().unwrap(),
            "dependency order and display metadata do not change the model source"
        );
        assert_ne!(
            first_key,
            edited_tree.mlx_runtime_model_cache_key().unwrap(),
            "a new dependency tree must not reuse the loaded MLX runtime"
        );
        assert_ne!(
            first_key,
            edited_root.mlx_runtime_model_cache_key().unwrap(),
            "a new root source must not reuse the loaded MLX runtime"
        );
    }

    #[test]
    fn inline_mlx_manifest_rejects_unsafe_or_incomplete_layouts() {
        let revision = "a".repeat(40);
        let unsafe_path = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            &revision,
            "../model.safetensors",
            4_000,
            false,
        );
        assert!(unsafe_path.inline_mlx_asset_bits().is_err());

        let missing_processor = inline_mlx_bit(
            BitTypes::Vlm,
            "owner/model",
            &revision,
            "model.safetensors",
            4_000,
            false,
        );
        assert!(missing_processor.inline_mlx_asset_bits().is_err());

        let invalid_revision = inline_mlx_bit(
            BitTypes::Llm,
            "owner/model",
            "main",
            "model.safetensors",
            4_000,
            false,
        );
        assert!(invalid_revision.inline_mlx_asset_bits().is_err());
    }

    #[tokio::test]
    async fn pack_carries_inline_mlx_assets_without_asking_a_hub() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        let store = LocalObjectStore::new(temp_dir.path().to_path_buf()).unwrap();
        config.stores.bits_store = Some(FlowLikeStore::Local(store.into()));
        let http_client = crate::utils::http::HTTPClient::new_without_refetch();
        let state = Arc::new(FlowLikeState::new(config, http_client));
        let bit = inline_mlx_bit(
            BitTypes::Vlm,
            "owner/model",
            &"a".repeat(40),
            "model.safetensors",
            4_000,
            true,
        );

        let pack = bit.pack(state).await.unwrap();
        assert_eq!(pack.bits.len(), 6);
        assert!(pack.bits.iter().any(|candidate| candidate.id == bit.id));
        assert!(
            pack.bits
                .iter()
                .any(|candidate| candidate.file_name.as_deref() == Some("processor_config.json"))
        );
    }

    #[test]
    fn projection_bit_materialises_an_inline_projector() {
        let bit = local_vlm_bit(flow_like_types::json::json!({
            "download_link": " https://example.com/mmproj-F16.gguf ",
            "file_name": "mmproj-F16.gguf",
            "size": 700,
        }));

        let projector = bit.projection_bit().expect("projector");
        assert!(projector.id.starts_with(USER_SOURCE_ARTIFACT_ID_PREFIX));
        assert_eq!(projector.bit_type, BitTypes::Projection);
        // hash == id keeps the trust-on-first-use download contract, while the
        // source-derived id prevents stale same-name/same-size reuse.
        assert_eq!(projector.hash, projector.id);
        assert!(projector.has_matching_user_source_artifact_identity());
        assert_eq!(
            projector.download_link.as_deref(),
            Some("https://example.com/mmproj-F16.gguf")
        );
        assert_eq!(projector.file_name.as_deref(), Some("mmproj-F16.gguf"));
        assert_eq!(projector.size, Some(700));
        assert_eq!(projector.hub, bit.hub);
    }

    #[test]
    fn projection_bit_ignores_incomplete_specs() {
        assert!(local_vlm_bit(Value::Null).projection_bit().is_none());
        assert!(
            local_vlm_bit(flow_like_types::json::json!({ "file_name": "mmproj.gguf" }))
                .projection_bit()
                .is_none()
        );
        assert!(
            local_vlm_bit(flow_like_types::json::json!({
                "download_link": "   ",
                "file_name": "mmproj.gguf",
            }))
            .projection_bit()
            .is_none()
        );
    }

    #[tokio::test]
    async fn pack_carries_the_projector_without_asking_a_hub() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        let store = LocalObjectStore::new(temp_dir.path().to_path_buf()).unwrap();
        config.stores.bits_store = Some(FlowLikeStore::Local(store.into()));
        let http_client = crate::utils::http::HTTPClient::new_without_refetch();
        let state = Arc::new(FlowLikeState::new(config, http_client));

        let bit = local_vlm_bit(flow_like_types::json::json!({
            "download_link": "https://example.com/mmproj-F16.gguf",
            "file_name": "mmproj-F16.gguf",
            "size": 700,
        }));

        // No dependency ids, so this must resolve offline — a user bit has no hub entry.
        let pack = bit.pack(state).await.unwrap();
        assert_eq!(pack.bits.len(), 2);
        assert!(pack.bits.iter().any(|b| b.id == "my-vlm"));
        assert!(pack.bits.iter().any(|b| b.bit_type == BitTypes::Projection
            && b.id.starts_with(USER_SOURCE_ARTIFACT_ID_PREFIX)));
    }

    #[test]
    fn user_gguf_source_identity_changes_for_a_new_pinned_url() {
        let revision_a = "a".repeat(40);
        let revision_b = "b".repeat(40);
        let mut first = local_vlm_bit(Value::Null);
        first.download_link = Some(format!(
            "https://huggingface.co/owner/model/resolve/{revision_a}/model.gguf"
        ));
        first.normalize_user_local_artifact_identity();

        let mut second = local_vlm_bit(Value::Null);
        second.download_link = Some(format!(
            "https://huggingface.co/owner/model/resolve/{revision_b}/model.gguf"
        ));
        second.normalize_user_local_artifact_identity();

        assert_eq!(first.id, second.id);
        assert_eq!(first.file_name, second.file_name);
        assert_eq!(first.size, second.size);
        assert_ne!(first.hash, second.hash);
        assert_ne!(first.dependency_tree_hash, second.dependency_tree_hash);
        assert_ne!(
            first.runtime_model_cache_key(),
            second.runtime_model_cache_key()
        );
        assert!(first.has_matching_user_source_artifact_identity());
        assert!(second.has_matching_user_source_artifact_identity());
    }

    #[test]
    fn user_projector_source_identity_invalidates_the_pack_and_runtime_cache() {
        let projection = |revision: &str| {
            flow_like_types::json::json!({
                "download_link": format!(
                    "https://huggingface.co/owner/model/resolve/{revision}/mmproj.gguf"
                ),
                "file_name": "mmproj.gguf",
                "size": 700,
            })
        };
        let mut first = local_vlm_bit(projection(&"a".repeat(40)));
        let mut second = local_vlm_bit(projection(&"b".repeat(40)));

        let first_projection = first.projection_bit().expect("first projector");
        let second_projection = second.projection_bit().expect("second projector");
        assert_eq!(first_projection.file_name, second_projection.file_name);
        assert_eq!(first_projection.size, second_projection.size);
        assert_ne!(first_projection.hash, second_projection.hash);

        first.normalize_user_local_artifact_identity();
        second.normalize_user_local_artifact_identity();
        assert_ne!(first.dependency_tree_hash, second.dependency_tree_hash);
        assert_ne!(
            first.runtime_model_cache_key(),
            second.runtime_model_cache_key()
        );
    }

    #[test]
    fn explicit_content_hash_is_preserved_only_for_an_unchanged_source() {
        let verified_hash = "c".repeat(64);
        let mut previous = local_vlm_bit(Value::Null);
        previous.hash = verified_hash.clone();
        previous.download_link = Some(format!(
            "https://huggingface.co/owner/model/resolve/{}/model.gguf",
            "a".repeat(40)
        ));
        previous.normalize_user_local_artifact_identity();

        let mut unchanged = previous.clone();
        unchanged.normalize_edited_user_local_artifact_identity(Some(&previous));
        assert_eq!(unchanged.hash, verified_hash);

        let mut edited = previous.clone();
        edited.download_link = Some(format!(
            "https://huggingface.co/owner/model/resolve/{}/model.gguf",
            "b".repeat(40)
        ));
        edited.normalize_edited_user_local_artifact_identity(Some(&previous));

        assert_ne!(edited.hash, verified_hash);
        assert!(edited.has_matching_user_source_artifact_identity());
        assert_ne!(previous.dependency_tree_hash, edited.dependency_tree_hash);
    }

    #[tokio::test]
    async fn test_download_skips_and_succeeds_without_links() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        let store = LocalObjectStore::new(temp_dir.path().to_path_buf()).unwrap();
        config.stores.bits_store = Some(FlowLikeStore::Local(store.into()));
        let http_client = crate::utils::http::HTTPClient::new_without_refetch();
        let state = FlowLikeState::new(config, http_client);
        let state = Arc::new(state);

        let proxied_bit = Bit {
            id: "proxied".into(),
            bit_type: BitTypes::Other,
            meta: Default::default(),
            authors: vec![],
            repository: None,
            download_link: None,
            file_name: None,
            hash: "hash_proxied".into(),
            size: Some(123),
            hub: "hub".into(),
            parameters: Value::Null,
            version: None,
            license: None,
            dependencies: vec![],
            dependency_tree_hash: "hash_proxied".into(),
            created: chrono::Utc::now().to_rfc3339(),
            updated: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        };

        let zero_size_bit = Bit {
            id: "zero".into(),
            bit_type: BitTypes::Other,
            meta: Default::default(),
            authors: vec![],
            repository: None,
            download_link: Some("http://example.com/file.bin".into()),
            file_name: Some("file.bin".into()),
            hash: "hash_zero".into(),
            size: Some(0),
            hub: "hub".into(),
            parameters: Value::Null,
            version: None,
            license: None,
            dependencies: vec![],
            dependency_tree_hash: "hash_zero".into(),
            created: chrono::Utc::now().to_rfc3339(),
            updated: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        };

        let pack = BitPack {
            bits: vec![proxied_bit.clone(), zero_size_bit.clone()],
        };
        let result = pack.download(state, None).await.unwrap();
        assert!(result.iter().any(|b| b.id == proxied_bit.id));
        assert!(!result.iter().any(|b| b.id == zero_size_bit.id));
    }

    #[test]
    fn pack_size_uses_the_full_storage_artifact_key() {
        let first = Bit {
            hash: "shared-content-hash".into(),
            file_name: Some("config.json".into()),
            size: Some(10),
            ..Bit::default()
        };
        let second = Bit {
            id: "second".into(),
            hash: first.hash.clone(),
            file_name: Some("tokenizer.json".into()),
            size: Some(10),
            ..Bit::default()
        };
        let exact_duplicate = Bit {
            id: "duplicate".into(),
            ..first.clone()
        };

        assert_eq!(
            BitPack {
                bits: vec![first, second, exact_duplicate],
            }
            .size(),
            20
        );
    }

    #[test]
    fn mlx_provider_is_recognized_only_for_language_model_bits() {
        let parameters = LLMParameters {
            context_length: 4096,
            provider: ModelProvider {
                provider_name: "mlx".to_string(),
                model_id: Some("mlx-community/test".to_string()),
                version: None,
                params: None,
            },
            model_classification: BitModelClassification::default(),
        };
        let mut bit = Bit {
            bit_type: BitTypes::Llm,
            parameters: flow_like_types::json::to_value(parameters).unwrap(),
            ..Bit::default()
        };

        assert!(bit.is_mlx_model());
        bit.bit_type = BitTypes::Other;
        assert!(!bit.is_mlx_model());
    }

    #[cfg(not(all(
        target_arch = "aarch64",
        any(
            target_os = "macos",
            all(target_os = "ios", not(any(target_abi = "sim", target_abi = "macabi")))
        )
    )))]
    #[test]
    fn mlx_capability_is_false_off_supported_apple_devices() {
        assert!(!can_host_mlx());
    }
}
