pub mod local;

use crate::{bit::Bit, state::FlowLikeState};
use flow_like_model_provider::llm::{
    ModelLogic, anthropic::AnthropicModel, cohere::CohereModel, deepseek::DeepseekModel,
    galadriel::GaladrielModel, gemini::GeminiModel, groq::GroqModel, huggingface::HuggingfaceModel,
    hyperbolic::HyperbolicModel, lmstudio::LMStudioModel, mira::MiraModel, mistral::MistralModel,
    moonshot::MoonshotModel, mozilla::MozillaModel, ollama::OllamaModel, openai::OpenAIModel,
    openrouter::OpenRouterModel, perplexity::PerplexityModel, together::TogetherModel,
    voyageai::VoyageAIModel, xai::XAIModel,
};
use flow_like_types::{Result, sync::Mutex, tokio::time::interval};
use local::LocalModel;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct ExecutionSettings {
    pub gpu_mode: bool,
    pub max_context_size: usize,
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        ExecutionSettings::new()
    }
}

impl ExecutionSettings {
    pub fn new() -> Self {
        Self {
            gpu_mode: false,
            max_context_size: 32_000,
        }
    }
}

// TODO: implement DashMap
pub struct ModelFactory {
    pub cached_models: HashMap<String, Arc<dyn ModelLogic>>,
    pub ttl_list: HashMap<String, SystemTime>,
    pub execution_settings: ExecutionSettings,
}

impl Default for ModelFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelFactory {
    pub fn new() -> Self {
        Self {
            cached_models: HashMap::new(),
            ttl_list: HashMap::new(),
            execution_settings: ExecutionSettings::new(),
        }
    }

    pub fn set_execution_settings(&mut self, settings: ExecutionSettings) {
        self.execution_settings = settings;
    }

    #[allow(clippy::cognitive_complexity)]
    async fn build_standard_model(
        &mut self,
        bit: &Bit,
        provider: &str,
        model_provider: &flow_like_model_provider::provider::ModelProvider,
        provider_config: &flow_like_model_provider::provider::ModelProviderConfiguration,
    ) -> Result<Arc<dyn ModelLogic>> {
        if let Some(model) = self.cached_models.get(&bit.id) {
            self.ttl_list.insert(bit.id.clone(), SystemTime::now());
            return Ok(model.clone());
        }

        let model: Arc<dyn ModelLogic> = match provider {
            "azure" | "openai" => {
                Arc::new(OpenAIModel::new(model_provider, provider_config).await?)
            }
            "anthropic" => Arc::new(AnthropicModel::new(model_provider, provider_config).await?),
            "gemini" => Arc::new(GeminiModel::new(model_provider, provider_config).await?),
            "huggingface" => {
                Arc::new(HuggingfaceModel::new(model_provider, provider_config).await?)
            }
            "cohere" => Arc::new(CohereModel::new(model_provider, provider_config).await?),
            "perplexity" => Arc::new(PerplexityModel::new(model_provider, provider_config).await?),
            "groq" => Arc::new(GroqModel::new(model_provider, provider_config).await?),
            "deepseek" => Arc::new(DeepseekModel::new(model_provider, provider_config).await?),
            "mistral" => Arc::new(MistralModel::new(model_provider, provider_config).await?),
            "together" => Arc::new(TogetherModel::new(model_provider, provider_config).await?),
            "openrouter" => Arc::new(OpenRouterModel::new(model_provider, provider_config).await?),
            "voyageai" => Arc::new(VoyageAIModel::new(model_provider, provider_config).await?),
            "ollama" => Arc::new(OllamaModel::new(model_provider, provider_config).await?),
            "hyperbolic" => Arc::new(HyperbolicModel::new(model_provider, provider_config).await?),
            "moonshot" => Arc::new(MoonshotModel::new(model_provider, provider_config).await?),
            "galadriel" => Arc::new(GaladrielModel::new(model_provider, provider_config).await?),
            "mira" => Arc::new(MiraModel::new(model_provider, provider_config).await?),
            "xai" => Arc::new(XAIModel::new(model_provider, provider_config).await?),
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported standard provider: {}",
                    provider
                ));
            }
        };

        self.ttl_list.insert(bit.id.clone(), SystemTime::now());
        self.cached_models.insert(bit.id.clone(), model.clone());
        Ok(model)
    }

    #[allow(clippy::cognitive_complexity)]
    async fn build_custom_model(
        &mut self,
        bit: &Bit,
        provider: &str,
        model_provider: &flow_like_model_provider::provider::ModelProvider,
    ) -> Result<Arc<dyn ModelLogic>> {
        if let Some(model) = self.cached_models.get(&bit.id) {
            self.ttl_list.insert(bit.id.clone(), SystemTime::now());
            return Ok(model.clone());
        }

        let model: Arc<dyn ModelLogic> = match provider {
            "custom:openai" => Arc::new(OpenAIModel::from_provider(model_provider).await?),
            "custom:anthropic" => Arc::new(AnthropicModel::from_provider(model_provider).await?),
            "custom:gemini" => Arc::new(GeminiModel::from_provider(model_provider).await?),
            "custom:groq" => Arc::new(GroqModel::from_provider(model_provider).await?),
            "custom:cohere" => Arc::new(CohereModel::from_provider(model_provider).await?),
            "custom:perplexity" => Arc::new(PerplexityModel::from_provider(model_provider).await?),
            "custom:xai" => Arc::new(XAIModel::from_provider(model_provider).await?),
            "custom:deepseek" => Arc::new(DeepseekModel::from_provider(model_provider).await?),
            "custom:mistral" => Arc::new(MistralModel::from_provider(model_provider).await?),
            "custom:ollama" => Arc::new(OllamaModel::from_provider(model_provider).await?),
            "custom:huggingface" => {
                Arc::new(HuggingfaceModel::from_provider(model_provider).await?)
            }
            "custom:together" => Arc::new(TogetherModel::from_provider(model_provider).await?),
            "custom:openrouter" => Arc::new(OpenRouterModel::from_provider(model_provider).await?),
            "custom:voyageai" => Arc::new(VoyageAIModel::from_provider(model_provider).await?),
            "custom:hyperbolic" => Arc::new(HyperbolicModel::from_provider(model_provider).await?),
            "custom:moonshot" => Arc::new(MoonshotModel::from_provider(model_provider).await?),
            "custom:galadriel" => Arc::new(GaladrielModel::from_provider(model_provider).await?),
            "custom:mira" => Arc::new(MiraModel::from_provider(model_provider).await?),
            "custom:mozilla" => Arc::new(MozillaModel::from_provider(model_provider).await?),
            "custom:lmstudio" => Arc::new(LMStudioModel::from_provider(model_provider).await?),
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported custom provider: {}",
                    provider
                ));
            }
        };

        self.ttl_list.insert(bit.id.clone(), SystemTime::now());
        self.cached_models.insert(bit.id.clone(), model.clone());
        Ok(model)
    }

    #[allow(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_lines)]
    pub async fn build(
        &mut self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
    ) -> Result<Arc<dyn ModelLogic>> {
        let provider_config = app_state.model_provider_config.clone();
        let settings = self.execution_settings.clone();
        let provider = bit.try_to_provider();
        if provider.is_none() {
            return Err(flow_like_types::anyhow!("Model type not supported"));
        }

        let model_provider =
            provider.ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let provider = model_provider.provider_name.clone();

        if provider == "Local" {
            if let Some(model) = self.cached_models.get(&bit.id) {
                self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                return Ok(model.clone());
            }

            let local_model = LocalModel::new(bit, app_state, &settings).await?;
            let local_model: Arc<LocalModel> = Arc::new(local_model);
            self.ttl_list.insert(bit.id.clone(), SystemTime::now());
            self.cached_models
                .insert(bit.id.clone(), local_model.clone());
            return Ok(local_model);
        }

        if provider.starts_with("custom:") {
            return self
                .build_custom_model(bit, &provider, &model_provider)
                .await;
        }

        if provider.to_lowercase() == "hosted" || provider.to_lowercase().starts_with("hosted:") {
            if let Some(model) = self.cached_models.get(&bit.id) {
                self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                return Ok(model.clone());
            }

            let mut model_provider = model_provider.clone();
            let mut params = model_provider.params.clone().unwrap_or_default();

            params.insert(
                "api_key".into(),
                flow_like_types::Value::String(access_token.clone().unwrap_or_default()),
            );

            params.insert(
                "model_id".into(),
                flow_like_types::Value::String(bit.id.clone()),
            );

            model_provider.model_id = Some(bit.id.clone());
            model_provider.params = Some(params.clone());

            let hosted_type = if provider.contains(':') {
                provider.split(':').nth(1).unwrap_or("openrouter")
            } else {
                "openrouter"
            };

            let model: Arc<dyn ModelLogic> = match hosted_type.to_lowercase().as_str() {
                "openrouter" => Arc::new(
                    OpenRouterModel::from_provider(&model_provider)
                        .await
                        .map_err(|e| {
                            flow_like_types::anyhow!(
                                "Failed to create hosted:openrouter model: {}",
                                e
                            )
                        })?,
                ),
                "openai" => Arc::new(OpenAIModel::from_provider(&model_provider).await.map_err(
                    |e| flow_like_types::anyhow!("Failed to create hosted:openai model: {}", e),
                )?),
                "anthropic" => Arc::new(
                    AnthropicModel::from_provider(&model_provider)
                        .await
                        .map_err(|e| {
                            flow_like_types::anyhow!(
                                "Failed to create hosted:anthropic model: {}",
                                e
                            )
                        })?,
                ),
                "azure" => Arc::new(OpenAIModel::from_provider(&model_provider).await.map_err(
                    |e| flow_like_types::anyhow!("Failed to create hosted:azure model: {}", e),
                )?),
                "bedrock" => {
                    return Err(flow_like_types::anyhow!(
                        "hosted:bedrock is not supported in local model factory"
                    ));
                }
                "vertex" => Arc::new(GeminiModel::from_provider(&model_provider).await.map_err(
                    |e| flow_like_types::anyhow!("Failed to create hosted:vertex model: {}", e),
                )?),
                _ => {
                    return Err(flow_like_types::anyhow!(
                        "Unsupported hosted provider type: {}",
                        hosted_type
                    ));
                }
            };

            self.ttl_list.insert(bit.id.clone(), SystemTime::now());
            self.cached_models.insert(bit.id.clone(), model.clone());
            return Ok(model);
        }

        self.build_standard_model(bit, &provider, &model_provider, &provider_config)
            .await
    }

    pub fn gc(&mut self) {
        let mut to_remove = Vec::new();
        for id in self.cached_models.keys() {
            // check if the model was not used for 5 minutes
            let ttl = self.ttl_list.get(id).unwrap();
            if ttl.elapsed().unwrap().as_secs() > 300 {
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            self.cached_models.remove(&id);
            self.ttl_list.remove(&id);
        }
    }
}

pub async fn start_gc(state: Arc<Mutex<ModelFactory>>) {
    let mut interval = interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        {
            let state = state.try_lock();
            if let Ok(mut state) = state {
                state.gc();
            }
        }
    }
}
