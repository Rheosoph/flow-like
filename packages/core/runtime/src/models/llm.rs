pub mod local;
pub mod mlx;
pub mod mlx_pack;

use super::{
    device::{self, ModelEndpoint},
    factory_cache::{FactoryCache, MODEL_IDLE_TTL},
    media::{MediaReader, RemoteMediaModel},
};
use crate::{
    bit::Bit,
    state::{CompletionModelCapabilities, FlowLikeState},
    utils::execute::RuntimeLocator,
};
use flow_like_model_provider::llm::{
    ModelLogic, anthropic::AnthropicModel, bedrock::BedrockModel, cohere::CohereModel,
    deepseek::DeepseekModel, galadriel::GaladrielModel, gemini::GeminiModel, groq::GroqModel,
    huggingface::HuggingfaceModel, hyperbolic::HyperbolicModel, llamacpp::LlamaCppModel,
    lmstudio::LMStudioModel, mira::MiraModel, mistral::MistralModel, moonshot::MoonshotModel,
    mozilla::MozillaModel, ollama::OllamaModel, openai::OpenAIModel, openrouter::OpenRouterModel,
    perplexity::PerplexityModel, together::TogetherModel, vertex::VertexModel,
    voyageai::VoyageAIModel, xai::XAIModel,
};
use flow_like_model_provider::provider::{ModelApiSurface, ModelProvider, is_hosted_provider_name};
use flow_like_types::{
    Result, json,
    tokio::{sync::Mutex as AsyncMutex, time::interval},
};
use local::LocalModel;
use mlx::MlxModel;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

/// Local engines load one at a time, so each llama-server's `--fit on` sizes its GPU offload
/// against the memory the engines loaded before it hold.
pub(crate) static LOCAL_ENGINE_LOADS: LazyLock<AsyncMutex<()>> = LazyLock::new(AsyncMutex::default);

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct ExecutionSettings {
    pub gpu_mode: bool,
    pub max_context_size: usize,
}

pub const DEFAULT_MAX_CONTEXT_SIZE: usize = 32_000;

impl Default for ExecutionSettings {
    fn default() -> Self {
        ExecutionSettings::new()
    }
}

impl ExecutionSettings {
    pub fn new() -> Self {
        Self {
            gpu_mode: true,
            max_context_size: DEFAULT_MAX_CONTEXT_SIZE,
        }
    }
}

/// Shared by every run without an outer lock: builds of one cache key coalesce, builds of other
/// keys proceed in parallel, so a cold start or an unlock prompt never stalls unrelated models.
pub struct ModelFactory {
    models: FactoryCache<dyn ModelLogic>,
    execution_settings: RwLock<ExecutionSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelUsageContext {
    pub app_id: Option<String>,
    pub run_id: Option<String>,
    pub api_base_url: Option<String>,
}

/// `custom:vertex` falls back to Google application-default credentials when
/// the Bit carries neither a service-account key nor an access token — i.e. to
/// the host process's own identity. Refuse that in a server-side state.
fn ensure_no_ambient_model_credentials(
    app_state: &FlowLikeState,
    provider: &str,
    model_provider: &flow_like_model_provider::provider::ModelProvider,
) -> Result<()> {
    if provider != "custom:vertex" {
        return Ok(());
    }
    let has_explicit = model_provider.params.as_ref().is_some_and(|params| {
        [
            "service_account_json",
            "service_account_key",
            "access_token",
        ]
        .iter()
        .any(|key| {
            params
                .get(*key)
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty())
        })
    });
    if has_explicit {
        return Ok(());
    }
    #[cfg(feature = "flow-metadata")]
    {
        app_state
            .execution_environment
            .ensure_no_ambient_credentials(provider, "application_default")
    }
    #[cfg(not(feature = "flow-metadata"))]
    {
        let _ = app_state;
        Ok(())
    }
}

fn insert_usage_headers(
    params: &mut HashMap<String, flow_like_types::Value>,
    usage_context: Option<&ModelUsageContext>,
) {
    // Hosted proxy headers must come only from the trusted execution context.
    // Bit metadata is admin-controlled and must not override Authorization or
    // inject other headers into the authenticated proxy request.
    let mut headers = json::Map::new();

    if let Some(usage_context) = usage_context {
        if let Some(app_id) = usage_context
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|app_id| !app_id.is_empty())
        {
            headers.insert(
                "x-flow-like-app-id".to_string(),
                flow_like_types::Value::String(app_id.to_string()),
            );
        }

        if let Some(run_id) = usage_context
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|run_id| !run_id.is_empty())
        {
            headers.insert(
                "x-flow-like-run-id".to_string(),
                flow_like_types::Value::String(run_id.to_string()),
            );
        }
    }

    if headers.is_empty() {
        params.remove("headers");
    } else {
        params.insert(
            "headers".to_string(),
            flow_like_types::Value::Object(headers),
        );
    }
}

/// Surface for a directly reachable OpenAI-compatible endpoint.
///
/// Rig's OpenAI client has always defaulted to the Responses API for `openai`,
/// `azure` and `custom:openai` Bits, so an undeclared surface keeps that. Only
/// gateways that cannot serve `/responses` need the explicit declaration.
fn direct_openai_surface(
    model_provider: &flow_like_model_provider::provider::ModelProvider,
) -> ModelApiSurface {
    model_provider
        .api_surface
        .unwrap_or(ModelApiSurface::Responses)
}

fn ensure_hosted_proxy_endpoint(
    params: &mut HashMap<String, flow_like_types::Value>,
    api_base_url: &str,
) {
    let api_base_url =
        flow_like_model_provider::embedding::proxy_config::normalize_base_url(api_base_url)
            .unwrap_or_default();
    let endpoint = if api_base_url.ends_with("/api/v1") {
        api_base_url
    } else {
        format!("{api_base_url}/api/v1")
    };
    params.insert(
        "endpoint".to_string(),
        flow_like_types::Value::String(endpoint),
    );
}

/// Why this host cannot run a Local or MLX Bit. When the runtime's executable is all the host
/// lacks, the refusal names that runtime and where the host looks for it.
async fn local_runtime_refusal(
    bit: &Bit,
    app_state: &Arc<FlowLikeState>,
) -> flow_like_types::Error {
    let installable = FlowLikeState::installable_completion_runtimes(app_state).await;
    runtime_refusal(bit, installable, &app_state.runtime_locator)
}

fn runtime_refusal(
    bit: &Bit,
    installable: CompletionModelCapabilities,
    locator: &RuntimeLocator,
) -> flow_like_types::Error {
    if bit.is_mlx_model() {
        if installable.mlx
            && let Err(missing) = locator.installed_mlx_service()
        {
            return flow_like_types::anyhow!(
                "MLX model {} cannot execute on this host: {missing}",
                bit.id
            );
        }
        return flow_like_types::anyhow!(
            "MLX model {} cannot execute on this host; it requires local ML, a local Bit store, and supported Apple-silicon hardware",
            bit.id
        );
    }
    if installable.local_server
        && let Err(missing) = locator.installed_llama_server()
    {
        return flow_like_types::anyhow!("Model {} cannot execute on this host: {missing}", bit.id);
    }
    flow_like_types::anyhow!(
        "Model {} cannot execute on this host; local llama-server models require local ML, a local Bit store, and a non-mobile target",
        bit.id
    )
}

impl Default for ModelFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelFactory {
    pub fn new() -> Self {
        Self {
            models: FactoryCache::default(),
            execution_settings: RwLock::new(ExecutionSettings::new()),
        }
    }

    pub fn set_execution_settings(&self, settings: ExecutionSettings) {
        *self.execution_settings.write() = settings;
    }

    async fn build_standard_model(
        &self,
        bit: &Bit,
        provider: &str,
        model_provider: &ModelProvider,
        provider_config: &flow_like_model_provider::provider::ModelProviderConfiguration,
    ) -> Result<Arc<dyn ModelLogic>> {
        self.models
            .get_or_build(&bit.id, || {
                Self::standard_model(provider, model_provider, provider_config)
            })
            .await
    }

    #[allow(clippy::cognitive_complexity)]
    async fn standard_model(
        provider: &str,
        model_provider: &ModelProvider,
        provider_config: &flow_like_model_provider::provider::ModelProviderConfiguration,
    ) -> Result<Arc<dyn ModelLogic>> {
        let model: Arc<dyn ModelLogic> = match provider {
            "azure" | "openai" => Arc::new(
                OpenAIModel::new(model_provider, provider_config)
                    .await?
                    .with_api_surface(direct_openai_surface(model_provider)),
            ),
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
            "lmstudio" => Arc::new(LMStudioModel::from_provider(model_provider).await?),
            "llama.cpp" | "llamacpp" => {
                Arc::new(LlamaCppModel::from_provider(model_provider).await?)
            }
            "hyperbolic" => Arc::new(HyperbolicModel::new(model_provider, provider_config).await?),
            "moonshot" => Arc::new(MoonshotModel::new(model_provider, provider_config).await?),
            "galadriel" => Arc::new(GaladrielModel::new(model_provider, provider_config).await?),
            "mira" => Arc::new(MiraModel::new(model_provider, provider_config).await?),
            "xai" => Arc::new(XAIModel::new(model_provider, provider_config).await?),
            "vertex" => Arc::new(VertexModel::new(model_provider, provider_config).await?),
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported standard provider: {}",
                    provider
                ));
            }
        };
        Ok(model)
    }

    async fn build_custom_model(
        &self,
        bit: &Bit,
        provider: &str,
        model_provider: &ModelProvider,
    ) -> Result<Arc<dyn ModelLogic>> {
        self.models
            .get_or_build(&bit.id, || Self::custom_model(provider, model_provider))
            .await
    }

    #[allow(clippy::cognitive_complexity)]
    async fn custom_model(
        provider: &str,
        model_provider: &ModelProvider,
    ) -> Result<Arc<dyn ModelLogic>> {
        let model: Arc<dyn ModelLogic> = match provider {
            "custom:openai" => Arc::new(
                OpenAIModel::from_provider_with_surface(
                    model_provider,
                    direct_openai_surface(model_provider),
                )
                .await?,
            ),
            "custom:bedrock" => Arc::new(BedrockModel::from_provider(model_provider).await?),
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
            "custom:vertex" => Arc::new(VertexModel::from_provider(model_provider).await?),
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported custom provider: {}",
                    provider
                ));
            }
        };
        Ok(model)
    }

    /// MLX and Local GGUF Bits run in a runtime this host starts: the MLX bridge or a
    /// llama-server sidecar.
    async fn build_local_runtime_model(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
    ) -> Result<Arc<dyn ModelLogic>> {
        let capabilities = FlowLikeState::completion_model_capabilities(&app_state).await;
        let settings = self.execution_settings.read().clone();
        if bit.is_mlx_model() {
            if !capabilities.mlx {
                return Err(local_runtime_refusal(bit, &app_state).await);
            }
            return self
                .models
                .get_or_build(&bit.mlx_runtime_model_cache_key()?, || async {
                    MlxModel::new(bit, app_state.clone(), &settings)
                        .await
                        .map(|model| model as Arc<dyn ModelLogic>)
                })
                .await;
        }

        if !capabilities.local_server {
            return Err(local_runtime_refusal(bit, &app_state).await);
        }
        self.models
            .get_or_build(&bit.runtime_model_cache_key(), || async {
                LocalModel::new(bit, app_state.clone(), &settings)
                    .await
                    .map(|model| model as Arc<dyn ModelLogic>)
            })
            .await
    }

    /// An OpenAI chat-completions client for a model served elsewhere, cached per endpoint.
    async fn build_endpoint_model(&self, endpoint: ModelEndpoint) -> Result<Arc<dyn ModelLogic>> {
        self.models
            .get_or_build(&endpoint.cache_key(), || async {
                let provider = ModelProvider {
                    provider_name: "openai".to_string(),
                    model_id: Some(endpoint.model.clone()),
                    version: None,
                    api_surface: Some(ModelApiSurface::ChatCompletions),
                    params: Some(HashMap::from([
                        (
                            "api_key".to_string(),
                            flow_like_types::Value::String(endpoint.bearer.clone()),
                        ),
                        (
                            "endpoint".to_string(),
                            flow_like_types::Value::String(
                                endpoint.base_url.trim_end_matches('/').to_string(),
                            ),
                        ),
                        (
                            "model_id".to_string(),
                            flow_like_types::Value::String(endpoint.model.clone()),
                        ),
                    ])),
                };
                let model = OpenAIModel::from_provider_with_surface(
                    &provider,
                    ModelApiSurface::ChatCompletions,
                )
                .await?;
                Ok(Arc::new(model) as Arc<dyn ModelLogic>)
            })
            .await
    }

    /// Wraps every model except MLX in [`RemoteMediaModel`], outside the cache, so fetches use
    /// the caller's execution environment and inline only media the provider's client reads.
    /// MLX hands out its cached runtime itself, whose identity `mlx_e2e` asserts.
    pub async fn build(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Arc<dyn ModelLogic>> {
        let environment = app_state.execution_environment;
        let model = self
            .build_inner(bit, app_state, access_token, usage_context)
            .await?;
        if bit.is_mlx_model() {
            return Ok(model);
        }
        let reader = bit
            .try_to_provider()
            .map_or(MediaReader::TextOnly, |provider| {
                MediaReader::for_provider(&provider.provider_name)
            });
        Ok(Arc::new(RemoteMediaModel::new(model, environment, reader)))
    }

    #[allow(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_lines)]
    async fn build_inner(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Arc<dyn ModelLogic>> {
        let provider_config = app_state.model_provider_config.clone();
        let mut model_provider = bit
            .try_to_provider()
            .ok_or_else(|| flow_like_types::anyhow!("Model type not supported"))?;
        let provider = model_provider.provider_name.trim().to_ascii_lowercase();
        model_provider.provider_name = provider.clone();

        if let Some(endpoint) =
            device::resolve_endpoint(bit, &app_state, usage_context.as_ref()).await?
        {
            return self.build_endpoint_model(endpoint).await;
        }

        if bit.is_mlx_model() || provider.eq_ignore_ascii_case("local") {
            return self.build_local_runtime_model(bit, app_state).await;
        }

        if provider.starts_with("custom:") {
            if flow_like_model_provider::llm::external::ExternalProvider::from_provider_name(
                &provider,
            )
            .is_some()
            {
                let capabilities = FlowLikeState::completion_model_capabilities(&app_state).await;
                return flow_like_model_provider::llm::external::build(
                    &model_provider,
                    capabilities.local_credentials,
                )
                .await;
            }
            ensure_no_ambient_model_credentials(&app_state, &provider, &model_provider)?;
            return self
                .build_custom_model(bit, &provider, &model_provider)
                .await;
        }

        if is_hosted_provider_name(&provider) {
            // Legacy callers still supply a token snapshot. Live-authorized
            // clients resolve their lease at dispatch, including retained agents.
            self.models.remove(&bit.id);

            let authorizer = app_state.request_authorizer.clone();
            let access_token = app_state
                .hosted_model_token
                .as_deref()
                .or(access_token.as_deref())
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(ToOwned::to_owned);
            if authorizer.is_none() && access_token.is_none() {
                return Err(flow_like_types::anyhow!(
                    "Hosted model {} requires an access token for the API proxy",
                    bit.id
                ));
            }
            tracing::debug!(
                bit_id = %bit.id,
                provider = %provider,
                "Building hosted model (no-cache)"
            );

            let mut model_provider = model_provider.clone();
            // Only fields required by the trusted Flow-Like proxy are passed to
            // the OpenAI-compatible client. Provider params stored on a Bit are
            // catalog metadata and are never forwarded as request options.
            let mut params = HashMap::new();

            if authorizer.is_none()
                && let Some(access_token) = access_token
            {
                params.insert(
                    "api_key".into(),
                    flow_like_types::Value::String(access_token),
                );
            }

            params.insert(
                "model_id".into(),
                flow_like_types::Value::String(bit.id.clone()),
            );
            insert_usage_headers(&mut params, usage_context.as_ref());
            let api_base_url = usage_context
                .as_ref()
                .and_then(|context| context.api_base_url.as_deref())
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(flow_like_model_provider::embedding::proxy_config::api_base_url);
            if let Some(resource_base) = authorizer.as_ref().and_then(|provider| {
                provider.resource_base_url(
                    flow_like_types::authorization::ResourceAudience::HostedModels,
                )
            }) {
                params.insert(
                    "endpoint".into(),
                    flow_like_types::Value::String(resource_base),
                );
            } else {
                ensure_hosted_proxy_endpoint(&mut params, &api_base_url);
            }
            if authorizer.as_ref().is_some_and(|provider| {
                provider.attribution()
                    == flow_like_types::authorization::AuthorizationAttribution::InstanceGrant
            }) {
                params.remove("headers");
            }
            params.remove("is_azure");

            model_provider.model_id = Some(bit.id.clone());
            model_provider.params = Some(params.clone());

            let endpoint = params
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("<none>");
            tracing::debug!(
                bit_id = %bit.id,
                hosted_type = %provider,
                endpoint = %endpoint,
                "Hosted model endpoint resolved"
            );

            let normalized_provider = provider.trim().to_ascii_lowercase();
            let hosted_type = normalized_provider
                .strip_prefix("hosted:")
                .unwrap_or("openrouter");

            // The Bit declares which proxy surface it speaks. `/responses` is
            // only reachable through Rig's OpenAI Responses client, so every
            // other hosted client rejects the declaration instead of silently
            // relaying an incompatible request shape.
            let api_surface = model_provider.api_surface_or_default();
            let reject_responses = |hosted_type: &str| {
                flow_like_types::anyhow!(
                    "hosted:{hosted_type} has no Responses API; set the Bit's api_surface to ChatCompletions or move it to hosted:openai"
                )
            };

            let model: Arc<dyn ModelLogic> = match hosted_type {
                "openrouter" if api_surface.is_responses() => {
                    return Err(reject_responses("openrouter"));
                }
                "openrouter" => Arc::new(
                    OpenRouterModel::from_provider_with_authorizer(
                        &model_provider,
                        authorizer.clone(),
                    )
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!(
                            "Failed to create hosted:openrouter proxy model: {}",
                            e
                        )
                    })?,
                ),
                "openai" => Arc::new(
                    OpenAIModel::from_provider_with_surface_and_authorizer(
                        &model_provider,
                        api_surface,
                        authorizer.clone(),
                    )
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!(
                            "Failed to create hosted:openai proxy model ({}): {}",
                            api_surface.as_str(),
                            e
                        )
                    })?,
                ),
                "anthropic" => {
                    return Err(flow_like_types::anyhow!(
                        "hosted:anthropic requires a native Messages API proxy adapter; the Flow-Like API exposes only /chat/completions and /responses"
                    ));
                }
                "azure" => {
                    return Err(flow_like_types::anyhow!(
                        "hosted:azure requires a native Azure deployment proxy adapter; the Flow-Like API exposes only /chat/completions and /responses"
                    ));
                }
                "bedrock" if api_surface.is_responses() => {
                    return Err(reject_responses("bedrock"));
                }
                "bedrock" => Arc::new(
                    OpenAIModel::from_provider_with_surface_and_authorizer(
                        &model_provider,
                        ModelApiSurface::ChatCompletions,
                        authorizer.clone(),
                    )
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!(
                            "Failed to create hosted:bedrock proxy model: {}",
                            e
                        )
                    })?,
                ),
                "vertex" => {
                    return Err(flow_like_types::anyhow!(
                        "hosted:vertex requires a native Vertex proxy adapter; Rig's Vertex client cannot target the Flow-Like HTTP proxy"
                    ));
                }
                _ => {
                    return Err(flow_like_types::anyhow!(
                        "Unsupported hosted provider type: {}",
                        hosted_type
                    ));
                }
            };

            return Ok(model);
        }

        self.build_standard_model(bit, &provider, &model_provider, &provider_config)
            .await
    }

    /// Evicts models unused for five minutes. A model someone still holds is in use, so a long
    /// turn never loses its server; its idle time starts when the last holder lets go.
    pub fn gc(&self) {
        self.models.gc(Instant::now(), MODEL_IDLE_TTL);
    }
}

pub async fn start_gc(state: Arc<ModelFactory>) {
    let mut interval = interval(Duration::from_secs(1));

    loop {
        interval.tick().await;
        state.gc();
    }
}

#[cfg(test)]
mod tests {
    use std::str;

    use super::*;
    use crate::{
        bit::{BitModelClassification, BitTypes, LLMParameters},
        state::FlowLikeConfig,
    };
    use flow_like_model_provider::history::{History, HistoryMessage, Role};
    use flow_like_model_provider::llm::{LLMCallback, UsageReportingMode};
    use flow_like_model_provider::provider::{
        ModelProvider, ModelProviderConfiguration, OllamaConfig,
    };
    use flow_like_storage::files::store::FlowLikeStore;
    use std::time::SystemTime;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    struct CapturedRequest {
        request_line: String,
        headers: String,
        body: flow_like_types::Value,
    }

    async fn capture_one_http_request(listener: TcpListener) -> CapturedRequest {
        let (mut stream, _) = listener.accept().await.expect("accept proxy request");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];

        let (header_end, content_length) = loop {
            let read = stream.read(&mut buffer).await.expect("read proxy request");
            assert!(read > 0, "connection closed before request headers");
            bytes.extend_from_slice(&buffer[..read]);

            let Some(header_end) = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let headers = str::from_utf8(&bytes[..header_end]).expect("UTF-8 request headers");
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| {
                        value
                            .trim()
                            .parse::<usize>()
                            .expect("numeric content length")
                    })
                })
                .unwrap_or_default();
            break (header_end, content_length);
        };

        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.expect("read request body");
            assert!(read > 0, "connection closed before request body");
            bytes.extend_from_slice(&buffer[..read]);
        }

        let headers = str::from_utf8(&bytes[..header_end])
            .expect("UTF-8 request headers")
            .to_string();
        let request_line = headers.lines().next().expect("request line").to_string();
        let body =
            flow_like_types::json::from_slice(&bytes[header_end..header_end + content_length])
                .expect("JSON request body");

        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nstop",
            )
            .await
            .expect("write mock response");

        CapturedRequest {
            request_line,
            headers,
            body,
        }
    }

    fn completion_bit(id: &str, provider_name: &str) -> Bit {
        completion_bit_with_surface(id, provider_name, None)
    }

    fn completion_bit_with_surface(
        id: &str,
        provider_name: &str,
        api_surface: Option<ModelApiSurface>,
    ) -> Bit {
        Bit {
            id: id.to_string(),
            bit_type: BitTypes::Llm,
            parameters: flow_like_types::json::to_value(LLMParameters {
                context_length: 20_000,
                model_classification: BitModelClassification::default(),
                provider: ModelProvider {
                    api_surface,
                    provider_name: provider_name.to_string(),
                    model_id: Some(id.to_string()),
                    version: None,
                    params: None,
                },
            })
            .unwrap(),
            ..Bit::default()
        }
    }

    fn usage_headers(
        usage_context: Option<ModelUsageContext>,
    ) -> HashMap<String, flow_like_types::Value> {
        let mut params = HashMap::from([(
            "headers".to_string(),
            flow_like_types::json::json!({
                "Authorization": "Bearer admin-controlled-token",
                "x-existing-header": "discarded",
                "X-Flow-Like-App-Id": "stale-app",
                "X-FLOW-LIKE-RUN-ID": "stale-run"
            }),
        )]);
        insert_usage_headers(&mut params, usage_context.as_ref());
        params
    }

    #[test]
    fn offline_usage_context_omits_app_header_but_keeps_run_header() {
        let params = usage_headers(Some(ModelUsageContext {
            app_id: None,
            run_id: Some("run-1".to_string()),
            api_base_url: None,
        }));
        let headers = params["headers"].as_object().expect("headers object");

        assert!(
            !headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("x-flow-like-app-id"))
        );
        assert_eq!(headers["x-flow-like-run-id"], "run-1");
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn server_backed_usage_context_includes_app_and_run_headers() {
        let params = usage_headers(Some(ModelUsageContext {
            app_id: Some("app-1".to_string()),
            run_id: Some("run-1".to_string()),
            api_base_url: None,
        }));
        let headers = params["headers"].as_object().expect("headers object");

        assert_eq!(headers["x-flow-like-app-id"], "app-1");
        assert_eq!(headers["x-flow-like-run-id"], "run-1");
        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn missing_usage_context_discards_all_bit_supplied_headers() {
        let params = usage_headers(None);
        assert!(!params.contains_key("headers"));
    }

    #[test]
    fn hosted_proxy_endpoint_uses_the_trusted_api_base() {
        let mut params = HashMap::new();
        ensure_hosted_proxy_endpoint(&mut params, "https://api.example.test/");
        assert_eq!(params["endpoint"], "https://api.example.test/api/v1");

        let mut versioned = HashMap::new();
        ensure_hosted_proxy_endpoint(&mut versioned, "https://api.example.test/api/v1");
        assert_eq!(versioned["endpoint"], "https://api.example.test/api/v1");

        let mut schemeless = HashMap::new();
        ensure_hosted_proxy_endpoint(&mut schemeless, "api.flow-like.com");
        assert_eq!(schemeless["endpoint"], "https://api.flow-like.com/api/v1");

        let mut overridden = HashMap::from([(
            "endpoint".to_string(),
            flow_like_types::Value::String("https://proxy.example.test/v1".to_string()),
        )]);
        ensure_hosted_proxy_endpoint(&mut overridden, "https://api.example.test");
        assert_eq!(overridden["endpoint"], "https://api.example.test/api/v1");
    }

    #[tokio::test]
    async fn factory_rejects_local_and_mlx_models_for_object_backed_bits() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        for (id, provider) in [
            ("local-model", "Local"),
            ("lowercase-local-model", "local"),
            ("mlx-model", "mlx"),
        ] {
            let result = factory
                .build(&completion_bit(id, provider), state.clone(), None, None)
                .await;
            let error = match result {
                Ok(_) => panic!("{provider} model should be rejected"),
                Err(error) => error,
            };
            let message = error.to_string();
            assert!(message.contains(id));
            assert!(message.contains("cannot execute on this host"));
            assert!(message.contains("local Bit store"));
            if provider.eq_ignore_ascii_case("local") {
                assert!(message.contains("non-mobile target"));
            } else {
                assert!(message.contains("supported Apple-silicon hardware"));
            }
        }
    }

    #[test]
    fn a_host_that_lacks_only_the_runtime_names_it() {
        let locator = RuntimeLocator::bundled_in(std::path::Path::new("/missing/flow-like"));
        let entrypoint = |runtime: Option<std::path::PathBuf>| {
            runtime.expect("a runtime path").display().to_string()
        };
        let installable = CompletionModelCapabilities {
            local_server: true,
            mlx: true,
            ..CompletionModelCapabilities::default()
        };

        let local = runtime_refusal(
            &completion_bit("local-model", "Local"),
            installable,
            &locator,
        )
        .to_string();
        let llama_server = entrypoint(locator.llama_server.clone().map(|r| r.entrypoint));
        assert!(local.starts_with("Model local-model cannot execute on this host"));
        assert!(local.contains("llama-server is not installed"), "{local}");
        assert!(local.contains(&llama_server), "{local}");

        let mlx =
            runtime_refusal(&completion_bit("mlx-model", "MLX"), installable, &locator).to_string();
        let helper = entrypoint(locator.mlx_service.clone().map(|r| r.entrypoint));
        assert!(mlx.contains("The MLX service is not installed"), "{mlx}");
        assert!(mlx.contains(&helper), "{mlx}");

        let unsupported = runtime_refusal(
            &completion_bit("local-model", "Local"),
            CompletionModelCapabilities::default(),
            &locator,
        )
        .to_string();
        assert!(unsupported.contains("non-mobile target"), "{unsupported}");
        assert!(!unsupported.contains("not installed"), "{unsupported}");
    }

    #[tokio::test]
    async fn factory_accepts_endpoint_backed_legacy_local_providers() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let model_provider_config = ModelProviderConfiguration {
            ollama_config: vec![OllamaConfig { endpoint: None }],
            ..ModelProviderConfiguration::default()
        };
        let state = Arc::new(FlowLikeState::new_with_model_config(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
            model_provider_config,
        ));
        let factory = ModelFactory::new();

        for provider in [
            "Ollama",
            "LMStudio",
            "Llama.cpp",
            "LLAMACPP",
            "Custom:Ollama",
            "CUSTOM:LMSTUDIO",
        ] {
            let result = factory
                .build(
                    &completion_bit(provider, provider),
                    state.clone(),
                    None,
                    None,
                )
                .await;
            assert!(result.is_ok(), "{provider} should build an endpoint client");
        }
    }

    #[tokio::test]
    async fn factory_routes_openrouter_hosted_labels_to_the_openrouter_client() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        for provider in ["Premium", "Internal", "Hosted", "hosted:openrouter"] {
            let model = factory
                .build(
                    &completion_bit(provider, provider),
                    state.clone(),
                    Some("token".to_string()),
                    None,
                )
                .await
                .unwrap_or_else(|error| panic!("{provider} should use OpenRouter: {error}"));
            assert_eq!(
                model.usage_reporting(),
                UsageReportingMode::OpenRouterUsageInclude,
                "{provider} should use the Rig OpenRouter client"
            );
            assert_eq!(model.default_model().await.as_deref(), Some(provider));
            assert!(!factory.models.contains(provider));
        }
    }

    #[tokio::test]
    async fn hosted_openrouter_streams_to_chat_completions_with_the_bit_id_and_current_token() {
        assert_hosted_openrouter_token(None, "current-jwt").await;
    }

    #[tokio::test]
    async fn frontend_models_use_run_authority_while_retaining_the_visitor_token() {
        assert_hosted_openrouter_token(Some("scoped-executor-jwt"), "scoped-executor-jwt").await;
    }

    async fn assert_hosted_openrouter_token(model_token: Option<&str>, expected_token: &str) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock proxy");
        let proxy_url = format!("http://{}", listener.local_addr().expect("proxy address"));
        let capture = tokio::spawn(capture_one_http_request(listener));

        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let mut state = FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        );
        state.hosted_model_token = model_token.map(ToOwned::to_owned);
        let state = Arc::new(state.for_execution_run());
        let factory = ModelFactory::new();
        let model = factory
            .build(
                &completion_bit("bit_opaque_123", "hosted:openrouter"),
                state,
                Some("current-jwt".to_string()),
                Some(ModelUsageContext {
                    app_id: Some("app-123".to_string()),
                    run_id: Some("run-456".to_string()),
                    api_base_url: Some(proxy_url),
                }),
            )
            .await
            .expect("build hosted OpenRouter model");

        let mut history = History::new(
            "ignored-upstream-model".to_string(),
            vec![HistoryMessage::from_string(Role::User, "hello")],
        );
        history.set_stream(true);
        let callback: LLMCallback = Arc::new(|_| Box::pin(async { Ok(()) }));
        let result = model.invoke(&history, Some(callback)).await;
        assert!(result.is_err(), "mock proxy deliberately returns HTTP 400");

        let request = capture.await.expect("capture task");
        assert_eq!(
            request.request_line,
            "POST /api/v1/chat/completions HTTP/1.1"
        );
        let headers = request.headers.to_ascii_lowercase();
        assert!(headers.contains(&format!("authorization: bearer {expected_token}\r\n")));
        assert!(headers.contains("x-flow-like-app-id: app-123\r\n"));
        assert!(headers.contains("x-flow-like-run-id: run-456\r\n"));
        assert_eq!(request.body["model"], "bit_opaque_123");
        assert_eq!(request.body["usage"]["include"], true);
        assert_eq!(request.body["stream"], true);
    }

    #[tokio::test]
    async fn instance_factory_preserves_exact_broker_endpoint_and_ignores_app_headers() {
        check_factory_live_billing(true).await;
    }

    #[tokio::test]
    async fn desktop_factory_preserves_trusted_endpoint_and_user_billing_headers() {
        check_factory_live_billing(false).await;
    }

    async fn check_factory_live_billing(instance: bool) {
        use flow_like_types::authorization::{
            AuthorizationFuture, AuthorizationRequest, RequestAuthorization, RequestAuthorizer,
            ResourceAudience,
        };
        struct ScopedAuthorizer {
            base: String,
            instance: bool,
        }
        impl RequestAuthorizer for ScopedAuthorizer {
            fn attribution(&self) -> flow_like_types::authorization::AuthorizationAttribution {
                if self.instance {
                    flow_like_types::authorization::AuthorizationAttribution::InstanceGrant
                } else {
                    flow_like_types::authorization::AuthorizationAttribution::User
                }
            }
            fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
                (audience == ResourceAudience::HostedModels).then(|| self.base.clone())
            }
            fn authorize<'a>(
                &'a self,
                request: AuthorizationRequest<'a>,
            ) -> AuthorizationFuture<'a> {
                Box::pin(async move {
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.url, format!("{}/chat/completions", self.base));
                    RequestAuthorization::new(
                        if self.instance {
                            "DPoP workload-lease"
                        } else {
                            "Bearer current-user-token"
                        }
                        .into(),
                        self.instance.then(|| "fresh-workload-proof".into()),
                        SystemTime::now() + Duration::from_secs(60),
                    )
                })
            }
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let path = if instance {
            "/api/v1/instances"
        } else {
            "/api/v1"
        };
        let base = format!("http://{}{path}", listener.local_addr().unwrap());
        let capture = tokio::spawn(capture_one_http_request(listener));
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let mut state = FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        );
        state.request_authorizer = Some(Arc::new(ScopedAuthorizer { base, instance }));
        let factory = ModelFactory::new();
        let model = factory
            .build(
                &completion_bit("offline-bit", "hosted:openrouter"),
                Arc::new(state),
                Some("obsolete-human-token".into()),
                Some(ModelUsageContext {
                    app_id: Some("offline-project-is-not-a-hosted-app".into()),
                    run_id: Some("local-run".into()),
                    api_base_url: Some("https://wrong.example/api/v1".into()),
                }),
            )
            .await
            .unwrap();
        let mut history = History::new(
            "ignored-model".into(),
            vec![HistoryMessage::from_string(Role::User, "hello")],
        );
        history.set_stream(true);
        let callback: LLMCallback = Arc::new(|_| Box::pin(async { Ok(()) }));
        assert!(model.invoke(&history, Some(callback)).await.is_err());
        let request = tokio::time::timeout(Duration::from_secs(10), capture)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            request.request_line,
            format!("POST {path}/chat/completions HTTP/1.1")
        );
        let headers = request.headers.to_ascii_lowercase();
        if instance {
            assert!(headers.contains("authorization: dpop workload-lease\r\n"));
            assert!(headers.contains("dpop: fresh-workload-proof\r\n"));
            assert!(!headers.contains("x-flow-like-app-id"));
            assert!(!headers.contains("x-flow-like-run-id"));
        } else {
            assert!(headers.contains("authorization: bearer current-user-token\r\n"));
            assert!(!headers.contains("dpop:"));
            assert!(
                headers.contains("x-flow-like-app-id: offline-project-is-not-a-hosted-app\r\n")
            );
            assert!(headers.contains("x-flow-like-run-id: local-run\r\n"));
        }
        assert!(!headers.contains("obsolete-human-token"));
    }

    #[tokio::test]
    async fn factory_routes_hosted_openai_to_chat_completions() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        let model = factory
            .build(
                &completion_bit("openai-bit", "hosted:openai"),
                state,
                Some("token".to_string()),
                None,
            )
            .await
            .expect("hosted:openai should use Rig's OpenAI Chat Completions client");

        assert_eq!(
            model.usage_reporting(),
            UsageReportingMode::OpenAIStreamOptions
        );
        assert_eq!(model.default_model().await.as_deref(), Some("openai-bit"));
        assert!(!factory.models.contains("openai-bit"));
    }

    #[tokio::test]
    async fn hosted_openai_streams_to_responses_when_the_bit_declares_it() {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock proxy");
        let proxy_url = format!("http://{}", listener.local_addr().expect("proxy address"));
        let capture = tokio::spawn(capture_one_http_request(listener));

        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();
        let model = factory
            .build(
                &completion_bit_with_surface(
                    "bit_responses_1",
                    "hosted:openai",
                    Some(ModelApiSurface::Responses),
                ),
                state,
                Some("current-jwt".to_string()),
                Some(ModelUsageContext {
                    app_id: Some("app-123".to_string()),
                    run_id: Some("run-456".to_string()),
                    api_base_url: Some(proxy_url),
                }),
            )
            .await
            .expect("build hosted OpenAI Responses model");

        // The Responses API reports usage on `response.completed`; asking for it
        // through `stream_options` would be rejected as an unknown parameter.
        assert_eq!(model.usage_reporting(), UsageReportingMode::None);

        let mut history = History::new(
            "ignored-upstream-model".to_string(),
            vec![HistoryMessage::from_string(Role::User, "hello")],
        );
        history.set_stream(true);
        let callback: LLMCallback = Arc::new(|_| Box::pin(async { Ok(()) }));
        let result = model.invoke(&history, Some(callback)).await;
        assert!(result.is_err(), "mock proxy deliberately returns HTTP 400");

        let request = capture.await.expect("capture task");
        assert_eq!(request.request_line, "POST /api/v1/responses HTTP/1.1");
        let headers = request.headers.to_ascii_lowercase();
        assert!(headers.contains("authorization: bearer current-jwt\r\n"));
        assert!(headers.contains("x-flow-like-app-id: app-123\r\n"));
        assert_eq!(request.body["model"], "bit_responses_1");
        assert!(request.body.get("stream_options").is_none());
    }

    #[tokio::test]
    async fn factory_rejects_responses_on_hosted_providers_without_a_responses_api() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        for provider in ["hosted:openrouter", "hosted:bedrock"] {
            let Err(error) = factory
                .build(
                    &completion_bit_with_surface(
                        provider,
                        provider,
                        Some(ModelApiSurface::Responses),
                    ),
                    state.clone(),
                    Some("token".to_string()),
                    None,
                )
                .await
            else {
                panic!("{provider} has no Responses API and must not build");
            };
            assert!(
                error.to_string().contains("no Responses API"),
                "{provider} should explain why Responses is unavailable: {error}"
            );
        }
    }

    #[tokio::test]
    async fn factory_routes_hosted_bedrock_to_its_chat_completions_wrapper() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        let model = factory
            .build(
                &completion_bit("bedrock-bit", "hosted:bedrock"),
                state,
                Some("token".to_string()),
                None,
            )
            .await
            .expect("hosted:bedrock should use its OpenAI-compatible Chat Completions wrapper");

        assert_eq!(
            model.usage_reporting(),
            UsageReportingMode::OpenAIStreamOptions
        );
        assert_eq!(model.default_model().await.as_deref(), Some("bedrock-bit"));
        assert!(!factory.models.contains("bedrock-bit"));
    }

    #[tokio::test]
    async fn factory_rejects_hosted_providers_without_native_proxy_adapters() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        for provider in ["hosted:anthropic", "hosted:azure", "hosted:vertex"] {
            let error = match factory
                .build(
                    &completion_bit(provider, provider),
                    state.clone(),
                    Some("token".to_string()),
                    None,
                )
                .await
            {
                Ok(_) => panic!("{provider} should require a native proxy adapter"),
                Err(error) => error,
            };

            assert!(
                error.to_string().contains("proxy adapter"),
                "unexpected error for {provider}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn factory_rejects_hosted_models_without_an_access_token() {
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = ModelFactory::new();

        let error = match factory
            .build(&completion_bit("hosted-model", "Hosted"), state, None, None)
            .await
        {
            Ok(_) => panic!("hosted model without a token should be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("requires an access token"));
    }

    mod device_models {
        use super::*;
        use crate::models::device::{
            DeviceModelConnector, DeviceModelTarget, Interaction, ModelUnavailable,
            ModelUnavailableReason, model_unavailable,
            testing::{FakeConnector, FakeRouter, device_llm_bit, endpoint},
        };
        use flow_like_types::async_trait;
        use tokio::sync::Barrier;

        fn memory_state() -> FlowLikeState {
            let store = FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ));
            FlowLikeState::new(
                FlowLikeConfig::with_default_store(store),
                crate::utils::http::HTTPClient::new_without_refetch(),
            )
        }

        fn run_context(run_id: &str) -> Option<ModelUsageContext> {
            Some(ModelUsageContext {
                run_id: Some(run_id.to_string()),
                ..ModelUsageContext::default()
            })
        }

        async fn build_error(
            factory: &ModelFactory,
            bit: &Bit,
            state: FlowLikeState,
        ) -> flow_like_types::Error {
            match factory
                .build(bit, Arc::new(state), None, run_context("run-7"))
                .await
            {
                Ok(_) => panic!("{} must not build", bit.id),
                Err(error) => error,
            }
        }

        #[tokio::test]
        async fn a_declined_hard_coded_device_bit_fails_with_a_typed_sentence() {
            let connector = FakeConnector::failing(ModelUnavailableReason::DeviceLockedDeclined);
            let mut state = memory_state();
            state.device_model_connector = Some(connector.clone());

            let error = build_error(
                &ModelFactory::new(),
                &device_llm_bit("device-bit", "dev-1"),
                state,
            )
            .await;

            assert_eq!(
                model_unavailable(&error).map(|unavailable| unavailable.reason),
                Some(ModelUnavailableReason::DeviceLockedDeclined)
            );
            assert_eq!(
                error.to_string(),
                "Model 'Qwen3 8B' runs on device 'GPU box', which stayed locked."
            );
            assert_eq!(
                *connector.interactions.lock(),
                vec![Interaction::Allowed {
                    run_label: Some("run-7".to_string())
                }]
            );
        }

        #[tokio::test]
        async fn without_a_connector_a_device_bit_is_a_server_execution_error() {
            let error = build_error(
                &ModelFactory::new(),
                &device_llm_bit("device-bit", "dev-1"),
                memory_state(),
            )
            .await;

            assert_eq!(
                model_unavailable(&error).map(|unavailable| unavailable.reason),
                Some(ModelUnavailableReason::ServerExecution)
            );
            let message = error.to_string();
            assert!(message.contains("'Qwen3 8B'") && message.contains("'dev-1'"));
        }

        #[tokio::test]
        async fn device_bits_stream_chat_completions_through_the_connector_endpoint() {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let capture = tokio::spawn(capture_one_http_request(listener));
            let mut state = memory_state();
            state.device_model_connector = Some(FakeConnector::serving(endpoint(&base)));

            let model = ModelFactory::new()
                .build(
                    &device_llm_bit("device-bit", "dev-1"),
                    Arc::new(state),
                    None,
                    None,
                )
                .await
                .expect("device model through the connector");
            let mut history = History::new(
                "ignored".to_string(),
                vec![HistoryMessage::from_string(Role::User, "hello")],
            );
            history.set_stream(true);
            let callback: LLMCallback = Arc::new(|_| Box::pin(async { Ok(()) }));
            assert!(model.invoke(&history, Some(callback)).await.is_err());

            let request = tokio::time::timeout(Duration::from_secs(10), capture)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(request.request_line, "POST /v1/chat/completions HTTP/1.1");
            assert!(
                request
                    .headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer loopback-token\r\n")
            );
            assert_eq!(request.body["model"], "qwen3-8b");
        }

        #[tokio::test]
        async fn device_models_are_cached_per_endpoint_not_per_bit() {
            let served = endpoint("http://127.0.0.1:9/v1");
            let mut state = memory_state();
            state.device_model_connector = Some(FakeConnector::serving(served.clone()));
            let state = Arc::new(state);
            let factory = ModelFactory::new();

            let first = factory
                .build_inner(&device_llm_bit("bit-a", "dev-1"), state.clone(), None, None)
                .await
                .unwrap();
            let second = factory
                .build_inner(&device_llm_bit("bit-b", "dev-1"), state, None, None)
                .await
                .unwrap();

            assert!(Arc::ptr_eq(&first, &second));
            assert!(factory.models.contains(&served.cache_key()));
            assert!(!factory.models.contains("bit-a"));
        }

        #[tokio::test]
        async fn the_router_serves_local_and_own_device_bits_without_a_sidecar() {
            let served = endpoint("http://127.0.0.1:9/v1");
            let mut state = memory_state();
            state.local_model_router = Some(Arc::new(FakeRouter(served.clone())));
            let state = Arc::new(state);
            let factory = ModelFactory::new();

            let model = factory
                .build(
                    &completion_bit("local-model", "Local"),
                    state.clone(),
                    None,
                    None,
                )
                .await
                .expect("a routed Local Bit needs no local runtime");
            assert_eq!(model.default_model().await.as_deref(), Some("qwen3-8b"));
            assert!(factory.models.contains(&served.cache_key()));
            assert!(!factory.models.contains("local-model"));

            factory
                .build(&device_llm_bit("own-bit", "this-device"), state, None, None)
                .await
                .expect("the router answers before any connector is needed");
        }

        #[tokio::test]
        async fn gc_keeps_a_held_model_and_evicts_it_a_ttl_after_release() {
            let served = endpoint("http://127.0.0.1:9/v1");
            let mut state = memory_state();
            state.local_model_router = Some(Arc::new(FakeRouter(served.clone())));
            let factory = ModelFactory::new();
            let held = factory
                .build(
                    &completion_bit("local-model", "Local"),
                    Arc::new(state),
                    None,
                    None,
                )
                .await
                .unwrap();
            let key = served.cache_key();
            let start = Instant::now();

            factory
                .models
                .gc(start + MODEL_IDLE_TTL * 2, MODEL_IDLE_TTL);
            assert!(factory.models.contains(&key), "a held model survives GC");

            drop(held);
            let released = start + MODEL_IDLE_TTL * 2 + Duration::from_secs(1);
            factory.models.gc(released, MODEL_IDLE_TTL);
            assert!(factory.models.contains(&key));

            factory.models.gc(
                released + MODEL_IDLE_TTL + Duration::from_secs(1),
                MODEL_IDLE_TTL,
            );
            assert!(!factory.models.contains(&key));
        }

        /// Answers only once both builds are connecting at the same time.
        struct RendezvousConnector(Barrier);

        #[async_trait]
        impl DeviceModelConnector for RendezvousConnector {
            async fn connect(
                &self,
                target: &DeviceModelTarget,
                _interaction: Interaction,
            ) -> std::result::Result<ModelEndpoint, ModelUnavailable> {
                self.0.wait().await;
                Ok(ModelEndpoint {
                    model: target.model.clone(),
                    ..endpoint(&format!("http://127.0.0.1:9/{}", target.device_id))
                })
            }
        }

        #[tokio::test]
        async fn builds_of_different_bits_run_in_parallel() {
            let mut state = memory_state();
            state.device_model_connector = Some(Arc::new(RendezvousConnector(Barrier::new(2))));
            let state = Arc::new(state);
            let factory = ModelFactory::new();
            let first_bit = device_llm_bit("bit-a", "dev-a");
            let second_bit = device_llm_bit("bit-b", "dev-b");

            let (first, second) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(
                    factory.build(&first_bit, state.clone(), None, None),
                    factory.build(&second_bit, state.clone(), None, None),
                )
            })
            .await
            .expect("one build waiting on its device must not block another");

            assert!(first.is_ok() && second.is_ok());
        }
    }
}
