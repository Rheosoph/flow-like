use std::collections::HashMap;

use aws_config::SdkConfig;
use flow_like_types::{
    Value,
    json::{Deserialize, Serialize},
    rand::{self, Rng},
};
use schemars::JsonSchema;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ModelProvider {
    pub provider_name: String,
    pub model_id: Option<String>,
    pub version: Option<String>,
    pub params: Option<HashMap<String, Value>>,
}

/// Remote embedding provider implementation
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub enum RemoteEmbeddingProvider {
    /// Internal OpenAI-compatible embedding gateway.
    ///
    /// Older bit configs serialized as `HuggingfaceEndpoint` or
    /// `CloudflareWorkersAI` are accepted as aliases so existing records can be
    /// migrated by config alone.
    #[default]
    #[serde(alias = "HuggingfaceEndpoint", alias = "CloudflareWorkersAI")]
    Internal,
}

/// Configuration for remote execution via API proxy
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct RemoteExecutionConfig {
    /// Deprecated per-bit endpoint URL. Remote embeddings now use the shared
    /// INTERNAL_EMBEDDING_ENDPOINT secret resolved by the API.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Optional API key secret override. Defaults to INTERNAL_EMBEDDING_SECRET.
    #[serde(default)]
    pub secret_name: Option<String>,
    /// Which remote provider implementation to use
    #[serde(default)]
    pub implementation: Option<RemoteEmbeddingProvider>,
    /// Deployed model ID for the upstream provider (e.g., Internal gateway model name)
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct EmbeddingModelProvider {
    pub languages: Vec<String>,
    pub vector_length: u32,
    pub input_length: u32,
    pub prefix: Prefix,
    pub pooling: Pooling,
    pub provider: ModelProvider,
    /// Remote execution configuration (for API proxy mode)
    #[serde(default)]
    pub remote: Option<RemoteExecutionConfig>,
}

impl EmbeddingModelProvider {
    /// Check if this provider supports remote execution via API proxy
    pub fn supports_remote(&self) -> bool {
        let has_model_id = self
            .remote
            .as_ref()
            .and_then(|r| r.model_id.as_deref())
            .or(self.provider.model_id.as_deref())
            .is_some_and(|model_id| !model_id.trim().is_empty());

        if !has_model_id {
            return false;
        }

        self.remote
            .as_ref()
            .is_some_and(|r| r.implementation.is_some())
            || is_internal_hosted_provider_name(&self.provider.provider_name)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ImageEmbeddingModelProvider {
    pub languages: Vec<String>,
    pub vector_length: u32,
    pub pooling: Pooling,
    pub provider: ModelProvider,
    /// Remote execution configuration (for API proxy mode)
    #[serde(default)]
    pub remote: Option<RemoteExecutionConfig>,
}

impl ImageEmbeddingModelProvider {
    /// Check if this provider supports remote execution via API proxy
    pub fn supports_remote(&self) -> bool {
        let has_model_id = self
            .remote
            .as_ref()
            .and_then(|r| r.model_id.as_deref())
            .or(self.provider.model_id.as_deref())
            .is_some_and(|model_id| !model_id.trim().is_empty());

        if !has_model_id {
            return false;
        }

        self.remote
            .as_ref()
            .is_some_and(|r| r.implementation.is_some())
            || is_internal_hosted_provider_name(&self.provider.provider_name)
    }
}

fn is_internal_hosted_provider_name(provider_name: &str) -> bool {
    let normalized = provider_name.trim().to_ascii_lowercase();
    normalized == "premium"
        || normalized == "hosted"
        || normalized == "internal"
        || normalized.starts_with("hosted:")
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ImageGenerationModelProvider {
    pub provider: ModelProvider,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct VideoGenerationModelProvider {
    pub provider: ModelProvider,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Prefix {
    pub query: String,
    pub paragraph: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub enum Pooling {
    CLS,
    Mean,
    None,
}

#[derive(Clone, Default, Debug)]
pub struct ModelProviderConfiguration {
    pub openai_config: Vec<OpenAIConfig>,
    pub anthropic_config: Vec<AnthropicConfig>,
    pub gemini_config: Vec<GeminiConfig>,
    pub huggingface_config: Vec<HuggingfaceConfig>,
    pub cohere_config: Vec<CohereConfig>,
    pub perplexity_config: Vec<PerplexityConfig>,
    pub groq_config: Vec<GroqConfig>,
    pub together_config: Vec<TogetherConfig>,
    pub openrouter_config: Vec<OpenRouterConfig>,
    pub deepseek_config: Vec<DeepseekConfig>,
    pub mistral_config: Vec<MistralConfig>,
    pub voyageai_config: Vec<VoyageAIConfig>,
    pub ollama_config: Vec<OllamaConfig>,
    pub hyperbolic_config: Vec<HyperbolicConfig>,
    pub moonshot_config: Vec<MoonshotConfig>,
    pub galadriel_config: Vec<GaladrielConfig>,
    pub mira_config: Vec<MiraConfig>,
    pub mozilla_config: Vec<MozillaConfig>,
    pub xai_config: Vec<XAIConfig>,
    pub vertex_config: Vec<VertexConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenAIConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub organization: Option<String>,
    pub proxy: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnthropicConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub beta: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeminiConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HuggingfaceConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
    pub sub_provider: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CohereConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerplexityConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroqConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TogetherConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeepseekConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MistralConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoyageAIConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OllamaConfig {
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HyperbolicConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoonshotConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GaladrielConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiraConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MozillaConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XAIConfig {
    pub api_key: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct VertexConfig {
    pub project_id: Option<String>,
    pub location: Option<String>,
    pub service_account_json: Option<String>,
    pub access_token: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BedrockConfig {
    pub config: SdkConfig,
}

pub fn random_provider<T>(vec: &[T]) -> flow_like_types::Result<T>
where
    T: Clone,
{
    if vec.is_empty() {
        return Err(flow_like_types::anyhow!("No Provider found"));
    }

    let index = {
        let mut rng = rand::rng();
        rng.random_range(0..vec.len())
    };
    Ok(vec[index].clone())
}
