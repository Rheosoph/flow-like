pub mod local;
pub mod mlx;
pub mod mlx_pack;

use crate::{bit::Bit, state::FlowLikeState};
use flow_like_model_provider::llm::{
    ModelLogic, anthropic::AnthropicModel, bedrock::BedrockModel, cohere::CohereModel,
    deepseek::DeepseekModel, galadriel::GaladrielModel, gemini::GeminiModel, groq::GroqModel,
    huggingface::HuggingfaceModel, hyperbolic::HyperbolicModel, llamacpp::LlamaCppModel,
    lmstudio::LMStudioModel, mira::MiraModel, mistral::MistralModel, moonshot::MoonshotModel,
    mozilla::MozillaModel, ollama::OllamaModel, openai::OpenAIModel, openrouter::OpenRouterModel,
    perplexity::PerplexityModel, together::TogetherModel, vertex::VertexModel,
    voyageai::VoyageAIModel, xai::XAIModel,
};
use flow_like_model_provider::provider::{ModelApiSurface, is_hosted_provider_name};
use flow_like_types::{Result, json, sync::Mutex, tokio::time::interval};
use local::LocalModel;
use mlx::MlxModel;
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

// TODO: implement DashMap
pub struct ModelFactory {
    pub cached_models: HashMap<String, Arc<dyn ModelLogic>>,
    pub ttl_list: HashMap<String, SystemTime>,
    pub execution_settings: ExecutionSettings,
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
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Arc<dyn ModelLogic>> {
        let provider_config = app_state.model_provider_config.clone();
        let settings = self.execution_settings.clone();
        let provider = bit.try_to_provider();
        if provider.is_none() {
            return Err(flow_like_types::anyhow!("Model type not supported"));
        }

        let mut model_provider =
            provider.ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let provider = model_provider.provider_name.trim().to_ascii_lowercase();
        model_provider.provider_name = provider.clone();

        if bit.is_mlx_model() || provider.eq_ignore_ascii_case("local") {
            let capabilities = FlowLikeState::completion_model_capabilities(&app_state).await;
            if bit.is_mlx_model() && !capabilities.mlx {
                return Err(flow_like_types::anyhow!(
                    "MLX model {} cannot execute on this host; it requires local ML, a local Bit store, and supported Apple-silicon hardware",
                    bit.id
                ));
            }
            if provider.eq_ignore_ascii_case("local") && !capabilities.local_server {
                return Err(flow_like_types::anyhow!(
                    "Model {} cannot execute on this host; local llama-server models require local ML, a local Bit store, and a non-mobile target",
                    bit.id
                ));
            }
        }

        if bit.is_mlx_model() {
            let cache_key = bit.mlx_runtime_model_cache_key()?;
            if let Some(model) = self.cached_models.get(&cache_key) {
                self.ttl_list.insert(cache_key.clone(), SystemTime::now());
                return Ok(model.clone());
            }

            let mlx_model: Arc<MlxModel> =
                Arc::new(MlxModel::new(bit, app_state.clone(), &settings).await?);
            self.ttl_list.insert(cache_key.clone(), SystemTime::now());
            self.cached_models.insert(cache_key, mlx_model.clone());
            return Ok(mlx_model);
        }

        if provider.eq_ignore_ascii_case("local") {
            let cache_key = bit.runtime_model_cache_key();
            if let Some(model) = self.cached_models.get(&cache_key) {
                self.ttl_list.insert(cache_key.clone(), SystemTime::now());
                return Ok(model.clone());
            }

            let local_model = LocalModel::new(bit, app_state, &settings).await?;
            let local_model: Arc<LocalModel> = Arc::new(local_model);
            self.ttl_list.insert(cache_key.clone(), SystemTime::now());
            self.cached_models.insert(cache_key, local_model.clone());
            return Ok(local_model);
        }

        if provider.starts_with("custom:") {
            ensure_no_ambient_model_credentials(&app_state, &provider, &model_provider)?;
            return self
                .build_custom_model(bit, &provider, &model_provider)
                .await;
        }

        if is_hosted_provider_name(&provider) {
            // Never serve hosted models from cache — the user's JWT (used as
            // api_key) is ephemeral and will expire. A cached model would carry
            // a stale token, causing "Unauthorized" errors on the API proxy.
            self.cached_models.remove(&bit.id);
            self.ttl_list.remove(&bit.id);

            let access_token = access_token
                .as_deref()
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Hosted model {} requires an access token for the API proxy",
                        bit.id
                    )
                })?
                .to_string();
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

            params.insert(
                "api_key".into(),
                flow_like_types::Value::String(access_token),
            );

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
            ensure_hosted_proxy_endpoint(&mut params, &api_base_url);
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
                    OpenRouterModel::from_provider(&model_provider)
                        .await
                        .map_err(|e| {
                            flow_like_types::anyhow!(
                                "Failed to create hosted:openrouter proxy model: {}",
                                e
                            )
                        })?,
                ),
                "openai" => Arc::new(
                    OpenAIModel::from_provider_with_surface(&model_provider, api_surface)
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
                "bedrock" => Arc::new(BedrockModel::from_proxy(&model_provider).await.map_err(
                    |e| {
                        flow_like_types::anyhow!(
                            "Failed to create hosted:bedrock proxy model: {}",
                            e
                        )
                    },
                )?),
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
        let mut factory = ModelFactory::new();

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
        let mut factory = ModelFactory::new();

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
        let mut factory = ModelFactory::new();

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
            assert!(!factory.cached_models.contains_key(provider));
        }
    }

    #[tokio::test]
    async fn hosted_openrouter_streams_to_chat_completions_with_the_bit_id_and_current_token() {
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
        let mut factory = ModelFactory::new();
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
        assert!(headers.contains("authorization: bearer current-jwt\r\n"));
        assert!(headers.contains("x-flow-like-app-id: app-123\r\n"));
        assert!(headers.contains("x-flow-like-run-id: run-456\r\n"));
        assert_eq!(request.body["model"], "bit_opaque_123");
        assert_eq!(request.body["usage"]["include"], true);
        assert_eq!(request.body["stream"], true);
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
        let mut factory = ModelFactory::new();

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
        assert!(!factory.cached_models.contains_key("openai-bit"));
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
        let mut factory = ModelFactory::new();
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
        let mut factory = ModelFactory::new();

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
        let mut factory = ModelFactory::new();

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
        assert!(!factory.cached_models.contains_key("bedrock-bit"));
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
        let mut factory = ModelFactory::new();

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
        let mut factory = ModelFactory::new();

        let error = match factory
            .build(&completion_bit("hosted-model", "Hosted"), state, None, None)
            .await
        {
            Ok(_) => panic!("hosted model without a token should be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("requires an access token"));
    }
}
