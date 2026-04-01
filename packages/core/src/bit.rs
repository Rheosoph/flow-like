use crate::flow::execution::context::ExecutionContext;
use crate::state::FlowLikeState;
use crate::utils::compression::{compress_to_file_json, from_compressed_json};
use crate::utils::download::download_bit;
use flow_like_model_provider::history::History;
use flow_like_model_provider::provider::{
    EmbeddingModelProvider, ImageEmbeddingModelProvider, ModelProvider,
};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::files::store::local_store::LocalObjectStore;
use flow_like_types::Value;
use flow_like_types::intercom::InterComCallback;

use rig::agent::AgentBuilder;
use rig::client::completion::{CompletionClientDyn, CompletionModelHandle};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

const NAME_HINT_WEIGHT: f32 = 0.2; // weight of name similarity for best model preference
const NAME_HINT_SIMILARITY_THRESHOLD: f32 = 0.5; // minimum required similarity score to model name

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

impl Metadata {
    pub async fn presign(&mut self, prefix: Path, store: &FlowLikeStore) {
        if let Some(icon) = &self.icon {
            if icon.starts_with("http://") || icon.starts_with("https://") {
                return;
            }
            let icon_path = prefix.child(format!("{icon}.webp"));
            if let Ok(url) = store
                .sign(
                    "GET",
                    &icon_path,
                    std::time::Duration::from_secs(60 * 60 * 24),
                )
                .await
            {
                self.icon = Some(url.to_string());
            }
        }

        if let Some(thumbnail) = &self.thumbnail {
            if thumbnail.starts_with("http://") || thumbnail.starts_with("https://") {
                return;
            }
            let thumbnail_path = prefix.child(format!("{thumbnail}.webp"));
            if let Ok(url) = store
                .sign(
                    "GET",
                    &thumbnail_path,
                    std::time::Duration::from_secs(60 * 60 * 24),
                )
                .await
            {
                self.thumbnail = Some(url.to_string());
            }
        }

        for media in &mut self.preview_media {
            if media.starts_with("http://") || media.starts_with("https://") {
                continue;
            }
            let media_path = prefix.child(format!("{media}.webp"));
            if let Ok(url) = store
                .sign(
                    "GET",
                    &media_path,
                    std::time::Duration::from_secs(60 * 60 * 24),
                )
                .await
            {
                *media = url.to_string();
            }
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum BitTypes {
    Llm,
    Vlm,
    Embedding,
    ImageEmbedding,
    File,
    Media,
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
    pub async fn get_installed(
        &self,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<Bit>> {
        let bits_store = FlowLikeState::bit_store(&state).await?.as_generic();

        let mut installed_bits = vec![];
        for bit in self.bits.iter() {
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
        let mut deduplicated_bits = vec![];
        let mut deduplication_helper = HashSet::new();
        self.bits.iter().for_each(|bit| {
            // If there is no download link we treat it as a virtual / proxied bit.
            // These should count as a successful "download" operation from a UX perspective
            // so we simply don't schedule a download but DO include it in the returned list.
            if bit.download_link.is_none() {
                println!("Skipping network download for bit {}: no download link (proxied or empty model)", bit.id);
                // Do not attempt any download but keep it in the final success vector
                return;
            }

            if deduplication_helper.contains(&bit.hash) {
                println!("Skipping bit {}: duplicate hash already queued", bit.id);
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

            deduplicated_bits.push(bit.clone());
            deduplication_helper.insert(bit.hash.clone());
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
                .filter(|b| b.download_link.is_none() && b.size.unwrap_or(0) > 0)
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
            .filter(|b| b.download_link.is_none() && b.size.unwrap_or(0) > 0)
            .cloned()
            .collect::<Vec<_>>();
        result.extend(deduplicated_bits);
        Ok(result)
    }

    pub fn size(&self) -> u64 {
        let mut size = 0;
        let mut bits_considered = HashSet::new();
        for bit in self.bits.iter() {
            if bits_considered.contains(&bit.hash) {
                continue;
            }
            bits_considered.insert(bit.hash.clone());
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
        let mut dependencies = self.dependencies(state).await?;
        dependencies.bits.push(self.clone());
        Ok(dependencies)
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
                .build(self, context.app_state.clone(), context.token.clone())
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
    use flow_like_types::{sync::Mutex, tokio};

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
}
