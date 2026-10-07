use std::{sync::Arc, time::Instant};

use flow_like_model_provider::{
    embedding::{
        EmbeddingModelLogic,
        adapters::LegacyEmbeddingAdapter,
        endpoint::EndpointEmbeddingModel,
        interface::{
            EmbeddingDescriptor, EmbeddingLimits, EmbeddingMetric, EmbeddingModality,
            EmbeddingModel, EmbeddingPurpose, EmbeddingSpace, EmbeddingSpec,
        },
        openai::OpenAIEmbeddingModel,
    },
    image_embedding::ImageEmbeddingModelLogic,
    provider::is_hosted_provider_name,
};

use crate::{bit::Bit, state::FlowLikeState};

use super::{
    device::{self, ModelEndpoint},
    factory_cache::{FactoryCache, MODEL_IDLE_TTL},
    llm::ModelUsageContext,
};
#[cfg(feature = "local-ml")]
use super::{
    embedding::local::LocalEmbeddingModel, embedding::multimodal::LocalMultimodalEmbeddingModel,
    image_embedding::local::LocalImageEmbeddingModel,
};

#[cfg(feature = "remote-ml")]
use flow_like_model_provider::embedding::proxy::ProxyEmbeddingModel;

/// Shared by every run without an outer lock, like [`super::llm::ModelFactory`].
pub struct EmbeddingFactory {
    text_models: FactoryCache<dyn EmbeddingModelLogic>,
    image_models: FactoryCache<dyn ImageEmbeddingModelLogic>,
    #[cfg(feature = "local-ml")]
    multimodal_models: FactoryCache<LocalMultimodalEmbeddingModel>,
}

pub fn is_local_provider(provider_name: &str) -> bool {
    provider_name.trim().eq_ignore_ascii_case("local")
}

/// Whether this host should execute the embedding model itself instead of
/// proxying it through the API.
///
/// Bits carrying a remote gateway config can still be locally runnable ONNX
/// models. Local execution also requires a filesystem-backed Bit store because
/// the ONNX loader reads model files directly. A server may compile `local-ml`
/// for other nodes while keeping Bits in object storage, so compiled capability
/// alone is not enough to select this path.
pub async fn prefers_local_execution(bit: &Bit, app_state: &Arc<FlowLikeState>) -> bool {
    let has_local_embedding_provider = is_local_embedding_provider(bit);
    if !has_local_embedding_provider {
        return false;
    }

    should_prefer_local_execution(
        has_local_embedding_provider,
        FlowLikeState::can_execute_local_bit_models(app_state).await,
    )
}

fn is_local_embedding_provider(bit: &Bit) -> bool {
    bit.try_to_embedding_provider()
        .is_some_and(|provider| is_local_provider(&provider.provider_name))
}

fn should_prefer_local_execution(
    has_local_embedding_provider: bool,
    can_execute_local_bit_models: bool,
) -> bool {
    has_local_embedding_provider && can_execute_local_bit_models
}

#[cfg(any(feature = "remote-ml", test))]
fn embedding_usage_headers(usage_context: Option<&ModelUsageContext>) -> Vec<(String, String)> {
    let Some(context) = usage_context else {
        return Vec::new();
    };

    let mut headers = Vec::new();
    if let Some(app_id) = context
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    {
        headers.push(("x-flow-like-app-id".to_string(), app_id.to_string()));
    }
    if let Some(run_id) = context
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|run_id| !run_id.is_empty())
    {
        headers.push(("x-flow-like-run-id".to_string(), run_id.to_string()));
    }
    headers
}

impl Default for EmbeddingFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingFactory {
    pub fn new() -> Self {
        Self {
            text_models: FactoryCache::default(),
            image_models: FactoryCache::default(),
            #[cfg(feature = "local-ml")]
            multimodal_models: FactoryCache::default(),
        }
    }

    /// Resolve a Bit to the common embedding contract while preserving legacy model behavior.
    pub async fn build(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModel>> {
        if versioned_embedding_spec(bit)?.is_some() {
            #[cfg(feature = "local-ml")]
            {
                return Ok(self.build_multimodal(bit, app_state).await?);
            }
            #[cfg(not(feature = "local-ml"))]
            {
                return Err(flow_like_types::anyhow!(
                    "This embedding adapter requires local ML execution"
                ));
            }
        }
        if bit.bit_type == crate::bit::BitTypes::ImageEmbedding {
            let model = self.build_image(bit, app_state.clone()).await?;
            let parameters = bit
                .try_to_image_embedding()
                .ok_or(flow_like_types::anyhow!("Invalid image embedding Bit"))?;
            let dimensions = model
                .output_dimensions()
                .unwrap_or(parameters.vector_length as usize);
            // Paired image/text Bits intentionally identify the same vector space.
            let pack = bit.pack(app_state).await?;
            let text_bit = pack
                .bits
                .iter()
                .find(|dependency| dependency.bit_type == crate::bit::BitTypes::Embedding)
                .ok_or(flow_like_types::anyhow!(
                    "Image embedding Bit has no paired text model"
                ))?;
            let mut descriptor =
                legacy_descriptor(bit, dimensions, model.context_tokens().unwrap_or(0), true)?;
            descriptor.space.id = legacy_space_id(text_bit, dimensions);
            Ok(Arc::new(LegacyEmbeddingAdapter::image(model, descriptor)))
        } else {
            let model = self
                .build_text_routed(bit, app_state, access_token, usage_context)
                .await?;
            let parameters = bit
                .try_to_embedding()
                .ok_or(flow_like_types::anyhow!("Invalid text embedding Bit"))?;
            let dimensions = model
                .output_dimensions()
                .unwrap_or(parameters.vector_length as usize);
            let mut descriptor = legacy_descriptor(
                bit,
                dimensions,
                model
                    .context_tokens()
                    .unwrap_or(parameters.input_length as usize),
                false,
            )?;
            // Remote adapters forward provider vectors without imposing normalization.
            descriptor.space.normalized = model.output_dimensions().is_some();
            Ok(Arc::new(LegacyEmbeddingAdapter::text(model, descriptor)))
        }
    }

    #[cfg(feature = "local-ml")]
    pub async fn build_multimodal(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<LocalMultimodalEmbeddingModel>> {
        let spec = versioned_embedding_spec(bit)?
            .ok_or(flow_like_types::anyhow!("Bit has no embedding recipe"))?;
        let key = blake3::hash(&serde_json::to_vec(&(
            bit.hub.as_str(),
            bit.id.as_str(),
            bit.hash.as_str(),
            bit.dependency_tree_hash.as_str(),
            spec,
        ))?)
        .to_hex()
        .to_string();
        self.multimodal_models
            .get_or_build(&key, || async move {
                LocalMultimodalEmbeddingModel::new(bit, app_state).await
            })
            .await
    }

    pub async fn build_text(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        if versioned_embedding_spec(bit)?.is_some() {
            #[cfg(feature = "local-ml")]
            {
                return Ok(self.build_multimodal(bit, app_state).await?);
            }
            #[cfg(not(feature = "local-ml"))]
            {
                return Err(flow_like_types::anyhow!(
                    "This embedding adapter requires local ML execution"
                ));
            }
        }
        let provider_config = app_state.model_provider_config.clone();

        let provider = bit
            .try_to_embedding_provider()
            .ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let embedding_provider = bit
            .try_to_embedding()
            .ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let provider_name = provider.provider_name;

        if is_local_provider(&provider_name) {
            #[cfg(feature = "local-ml")]
            {
                let key = local_embedding_cache_key(bit)?;
                return self
                    .text_models
                    .get_or_build(&key, || async move {
                        let model = LocalEmbeddingModel::new(bit, app_state).await?;
                        Ok(model as Arc<dyn EmbeddingModelLogic>)
                    })
                    .await;
            }

            #[cfg(not(feature = "local-ml"))]
            {
                return Err(flow_like_types::anyhow!(
                    "Local models are not supported. Please enable the 'local-ml' feature."
                ));
            }
        }

        if provider_name == "openai" || provider_name == "azure" {
            let local_model =
                OpenAIEmbeddingModel::new(&embedding_provider, &provider_config).await?;
            return Ok(Arc::new(local_model));
        }

        Err(flow_like_types::anyhow!("Model type not supported"))
    }

    /// Build a text embedding model using the capabilities of the current host.
    ///
    /// Device Bits, and Local Bits a device's model host serves, use that
    /// endpoint. Every other Bit runs locally or through the API proxy.
    pub async fn build_text_routed(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        if versioned_embedding_spec(bit)?.is_some() {
            return self.build_text(bit, app_state).await;
        }
        if let Some(endpoint) =
            device::resolve_endpoint(bit, &app_state, usage_context.as_ref()).await?
        {
            return self.build_endpoint_text(bit, endpoint).await;
        }
        self.build_text_local_or_proxy(bit, app_state, access_token, usage_context)
            .await
    }

    /// A filesystem-backed host with local ML support keeps locally runnable
    /// Bits local. Other hosts proxy remote-capable Bits when an access token is
    /// available. A remote-capable Local Bit fails with a routing error when the
    /// proxy is unavailable; standard providers use their normal factory path.
    async fn build_text_local_or_proxy(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        let prefers_local = prefers_local_execution(bit, &app_state).await;
        let is_local_provider = is_local_embedding_provider(bit);
        let is_hosted_provider = bit
            .try_to_embedding_provider()
            .is_some_and(|provider| is_hosted_provider_name(&provider.provider_name));
        let supports_remote = bit
            .try_to_embedding()
            .is_some_and(|provider| provider.supports_remote());

        if is_hosted_provider && !supports_remote {
            return Err(flow_like_types::anyhow!(
                "Hosted embedding {} requires a non-empty model_id for remote execution",
                bit.id
            ));
        }

        #[cfg(feature = "remote-ml")]
        if !prefers_local && supports_remote {
            if let Some(authorizer) = &app_state.request_authorizer {
                return self
                    .build_text_proxy_authorized(
                        bit,
                        String::new(),
                        usage_context,
                        Some(authorizer.clone()),
                    )
                    .await;
            }
            let access_token = app_state.hosted_model_token.clone().or(access_token);
            if let Some(access_token) = access_token.filter(|token| !token.trim().is_empty()) {
                return self
                    .build_text_proxy(bit, access_token, usage_context)
                    .await;
            }
            if is_local_provider || is_hosted_provider {
                return Err(flow_like_types::anyhow!(
                    "Remote embedding requires an access token when local execution is unavailable"
                ));
            }
        }

        #[cfg(feature = "remote-ml")]
        if is_local_provider && !prefers_local {
            return Err(flow_like_types::anyhow!(
                "Local embedding cannot execute on this host and the Bit does not provide remote execution configuration"
            ));
        }

        #[cfg(not(feature = "remote-ml"))]
        {
            let _ = (access_token, usage_context);
            if (is_local_provider || is_hosted_provider) && !prefers_local {
                if supports_remote {
                    return Err(flow_like_types::anyhow!(
                        "Remote embedding requires the 'remote-ml' feature when local execution is unavailable"
                    ));
                }
                return Err(flow_like_types::anyhow!(
                    "Local embedding cannot execute on this host and the Bit does not provide remote execution configuration"
                ));
            }
        }

        self.build_text(bit, app_state).await
    }

    /// An embedding client for a model served elsewhere, cached per endpoint and Bit because the
    /// Bit sets prefixes and chunk sizes.
    async fn build_endpoint_text(
        &self,
        bit: &Bit,
        endpoint: ModelEndpoint,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        let provider = bit.try_to_embedding().ok_or_else(|| {
            flow_like_types::anyhow!("Bit {} is not a text embedding model", bit.id)
        })?;
        let key = format!("{}#{}", endpoint.cache_key(), bit.id);
        self.text_models
            .get_or_build(&key, || async move {
                let model = EndpointEmbeddingModel::new(
                    &endpoint.base_url,
                    endpoint.bearer,
                    endpoint.model,
                    provider,
                )?;
                Ok(Arc::new(model) as Arc<dyn EmbeddingModelLogic>)
            })
            .await
    }

    pub async fn build_image(
        &self,
        bit: &Bit,
        _app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<dyn ImageEmbeddingModelLogic>> {
        if versioned_embedding_spec(bit)?.is_some() {
            #[cfg(feature = "local-ml")]
            {
                let model = self.build_multimodal(bit, _app_state).await?;
                if !model
                    .descriptor()
                    .modalities
                    .contains(&EmbeddingModality::Image)
                {
                    return Err(flow_like_types::anyhow!(
                        "Embedding model has no vision encoder"
                    ));
                }
                return Ok(model);
            }
            #[cfg(not(feature = "local-ml"))]
            {
                return Err(flow_like_types::anyhow!(
                    "This embedding adapter requires local ML execution"
                ));
            }
        }
        let provider = bit
            .try_to_image_embedding()
            .ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let provider = provider.provider.provider_name;

        if is_local_provider(&provider) {
            #[cfg(feature = "local-ml")]
            {
                let key = local_embedding_cache_key(bit)?;
                return self
                    .image_models
                    .get_or_build(&key, || async move {
                        let model = LocalImageEmbeddingModel::new(bit, _app_state, self).await?;
                        Ok(model as Arc<dyn ImageEmbeddingModelLogic>)
                    })
                    .await;
            }
            #[cfg(not(feature = "local-ml"))]
            {
                return Err(flow_like_types::anyhow!(
                    "Local models are not supported. Please enable the 'local-ml' feature."
                ));
            }
        }

        Err(flow_like_types::anyhow!("Model type not supported"))
    }

    /// Build a text embedding model that proxies through the API
    /// Used in executors (AWS Lambda, Kubernetes) where secrets are not available
    #[cfg(feature = "remote-ml")]
    pub async fn build_text_proxy(
        &self,
        bit: &Bit,
        access_token: String,
        usage_context: Option<ModelUsageContext>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        self.build_text_proxy_authorized(bit, access_token, usage_context, None)
            .await
    }

    #[cfg(feature = "remote-ml")]
    async fn build_text_proxy_authorized(
        &self,
        bit: &Bit,
        access_token: String,
        usage_context: Option<ModelUsageContext>,
        authorizer: Option<Arc<dyn flow_like_types::authorization::RequestAuthorizer>>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
        let embedding_provider = bit
            .try_to_embedding()
            .ok_or(flow_like_types::anyhow!("Model type not supported"))?;

        // Check if the model supports remote execution
        if !embedding_provider.supports_remote() {
            return Err(flow_like_types::anyhow!(
                "Model does not support remote execution"
            ));
        }

        let usage_headers = embedding_usage_headers(usage_context.as_ref());
        let api_base_url = usage_context
            .as_ref()
            .and_then(|context| context.api_base_url.as_deref())
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(flow_like_model_provider::embedding::proxy_config::api_base_url);

        let proxy_model = ProxyEmbeddingModel::new(
            embedding_provider,
            bit.id.clone(),
            access_token,
            usage_headers,
            api_base_url,
        );
        let proxy_model = match authorizer {
            Some(authorizer) => proxy_model.with_authorizer(authorizer)?,
            None => proxy_model,
        };
        let model: Arc<dyn EmbeddingModelLogic> = Arc::new(proxy_model);

        Ok(model)
    }

    /// Evicts models unused for five minutes; a model someone still holds is in use.
    pub fn gc(&self) {
        let now = Instant::now();
        self.image_models.gc(now, MODEL_IDLE_TTL);
        self.text_models.gc(now, MODEL_IDLE_TTL);
        #[cfg(feature = "local-ml")]
        self.multimodal_models.gc(now, MODEL_IDLE_TTL);
    }
}

#[cfg(feature = "local-ml")]
fn local_embedding_cache_key(bit: &Bit) -> flow_like_types::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(&(
        &bit.hub,
        &bit.id,
        &bit.hash,
        &bit.dependency_tree_hash,
        &bit.parameters,
    ))?)
    .to_hex()
    .to_string())
}

pub fn versioned_embedding_spec(bit: &Bit) -> flow_like_types::Result<Option<EmbeddingSpec>> {
    match bit.parameters.get("embedding") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let spec: EmbeddingSpec = serde_json::from_value(value.clone())?;
            spec.validate()?;
            Ok(Some(spec))
        }
    }
}

fn legacy_space_id(bit: &Bit, dimensions: usize) -> String {
    let recipe = blake3::hash(bit.parameters.to_string().as_bytes());
    format!(
        "legacy:{}:{}:{}:{}:{}:{dimensions}",
        bit.hub,
        bit.id,
        bit.hash,
        bit.dependency_tree_hash,
        recipe.to_hex()
    )
}

fn legacy_descriptor(
    bit: &Bit,
    dimensions: usize,
    max_tokens: usize,
    image: bool,
) -> flow_like_types::Result<EmbeddingDescriptor> {
    let fingerprint = blake3::hash(&serde_json::to_vec(&(
        bit.hash.as_str(),
        bit.dependency_tree_hash.as_str(),
        &bit.parameters,
    ))?)
    .to_hex()
    .to_string();
    Ok(EmbeddingDescriptor {
        model_id: bit.id.clone(),
        adapter: if image { "legacy_image" } else { "legacy_text" }.into(),
        modalities: if image {
            vec![EmbeddingModality::Text, EmbeddingModality::Image]
        } else {
            vec![EmbeddingModality::Text]
        },
        joint_combinations: vec![],
        purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
        space: EmbeddingSpace {
            id: legacy_space_id(bit, dimensions),
            dimensions,
            normalized: true,
            metric: EmbeddingMetric::Cosine,
        },
        supported_dimensions: vec![dimensions],
        limits: EmbeddingLimits {
            max_tokens,
            max_images_per_item: Some(1),
            ..Default::default()
        },
        pipeline_fingerprint: fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bit::BitTypes;
    use flow_like_model_provider::provider::{
        EmbeddingModelProvider, ModelProvider, Pooling, Prefix, RemoteEmbeddingProvider,
        RemoteExecutionConfig,
    };
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_types::{json, tokio};
    use std::collections::HashMap;

    fn embedding_bit(provider_name: &str) -> Bit {
        let parameters = EmbeddingModelProvider {
            languages: vec!["en".to_string()],
            vector_length: 384,
            input_length: 512,
            prefix: Prefix {
                query: String::new(),
                paragraph: String::new(),
            },
            pooling: Pooling::Mean,
            provider: ModelProvider {
                api_surface: None,
                provider_name: provider_name.to_string(),
                model_id: Some("embedding-model".to_string()),
                version: None,
                params: None,
            },
            remote: Some(RemoteExecutionConfig {
                endpoint: None,
                secret_name: None,
                implementation: Some(RemoteEmbeddingProvider::Internal),
                model_id: Some("embedding-model".to_string()),
                ..Default::default()
            }),
        };

        Bit {
            bit_type: BitTypes::Embedding,
            parameters: json::to_value(parameters).expect("embedding parameters serialize"),
            ..Bit::default()
        }
    }

    fn headers(usage_context: Option<&ModelUsageContext>) -> HashMap<String, String> {
        embedding_usage_headers(usage_context).into_iter().collect()
    }

    #[test]
    fn offline_embedding_usage_omits_app_header_but_keeps_run_header() {
        let usage_context = ModelUsageContext {
            app_id: None,
            run_id: Some("run-1".to_string()),
            api_base_url: None,
        };
        let headers = headers(Some(&usage_context));

        assert!(!headers.contains_key("x-flow-like-app-id"));
        assert_eq!(headers["x-flow-like-run-id"], "run-1");
    }

    #[test]
    fn server_backed_embedding_usage_includes_app_and_run_headers() {
        let usage_context = ModelUsageContext {
            app_id: Some("app-1".to_string()),
            run_id: Some("run-1".to_string()),
            api_base_url: None,
        };
        let headers = headers(Some(&usage_context));

        assert_eq!(headers["x-flow-like-app-id"], "app-1");
        assert_eq!(headers["x-flow-like-run-id"], "run-1");
    }

    #[test]
    fn missing_embedding_usage_context_adds_no_headers() {
        assert!(headers(None).is_empty());
    }

    #[test]
    fn local_embedding_requires_local_execution_capability() {
        let bit = embedding_bit("Local");
        let has_local_embedding_provider = is_local_embedding_provider(&bit);

        assert!(should_prefer_local_execution(
            has_local_embedding_provider,
            true
        ));
        assert!(!should_prefer_local_execution(
            has_local_embedding_provider,
            false
        ));
    }

    #[test]
    fn non_local_embedding_provider_never_prefers_local_execution() {
        let bit = embedding_bit("hosted");

        assert!(!should_prefer_local_execution(
            is_local_embedding_provider(&bit),
            true
        ));
    }

    #[tokio::test]
    async fn object_backed_bit_store_does_not_prefer_local_embedding_execution() {
        let bit = embedding_bit("Local");
        assert!(
            bit.try_to_embedding()
                .expect("embedding parameters deserialize")
                .supports_remote()
        );

        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));

        assert!(!prefers_local_execution(&bit, &state).await);
    }

    #[cfg(feature = "remote-ml")]
    #[tokio::test]
    async fn routed_builder_proxies_remote_capable_bit_for_object_backed_store() {
        use flow_like_model_provider::embedding::proxy::ProxyEmbeddingModel;

        let bit = embedding_bit("Local");
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let usage_context = ModelUsageContext {
            app_id: None,
            run_id: Some("memory-run".to_string()),
            api_base_url: Some("https://api.example.test".to_string()),
        };

        let model = EmbeddingFactory::new()
            .build_text_routed(
                &bit,
                state,
                Some("user-token".to_string()),
                Some(usage_context),
            )
            .await
            .expect("remote-capable embedding should use the proxy");

        assert!(model.as_cacheable().as_any().is::<ProxyEmbeddingModel>());
    }

    #[cfg(feature = "remote-ml")]
    #[tokio::test]
    async fn routed_builder_requires_token_for_remote_capable_local_bit() {
        let bit = embedding_bit("Local");
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));

        let error = match EmbeddingFactory::new()
            .build_text_routed(&bit, state, None, None)
            .await
        {
            Ok(_) => panic!("proxy routing without a token must fail explicitly"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("requires an access token"));
    }

    #[cfg(feature = "remote-ml")]
    #[tokio::test]
    async fn routed_builder_requires_token_for_hosted_embedding_bit() {
        let bit = embedding_bit("Hosted");
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));

        let error = match EmbeddingFactory::new()
            .build_text_routed(&bit, state, None, None)
            .await
        {
            Ok(_) => panic!("hosted embedding without a token must fail explicitly"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("requires an access token"));
    }

    #[tokio::test]
    async fn routed_builder_requires_model_id_for_hosted_embedding_bit() {
        let mut bit = embedding_bit("Hosted");
        let mut parameters = bit
            .try_to_embedding()
            .expect("embedding parameters deserialize");
        parameters.provider.model_id = None;
        parameters.remote.as_mut().expect("remote config").model_id = Some("   ".to_string());
        bit.parameters = json::to_value(parameters).expect("embedding parameters serialize");

        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));

        let error = match EmbeddingFactory::new()
            .build_text_routed(&bit, state, Some("user-token".to_string()), None)
            .await
        {
            Ok(_) => panic!("hosted embedding without a model ID must fail explicitly"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("requires a non-empty model_id"));
    }

    #[cfg(not(feature = "remote-ml"))]
    #[tokio::test]
    async fn routed_builder_reports_missing_remote_capability() {
        let bit = embedding_bit("Local");
        let store = FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            crate::state::FlowLikeConfig::with_default_store(store),
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));

        let error = match EmbeddingFactory::new()
            .build_text_routed(&bit, state, Some("user-token".to_string()), None)
            .await
        {
            Ok(_) => panic!("a build without remote ML cannot create the proxy"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("requires the 'remote-ml' feature")
        );
    }

    mod endpoints {
        use super::*;
        use crate::models::device::{
            Interaction, ModelUnavailableReason, model_unavailable,
            testing::{FakeConnector, FakeRouter, device_embedding_bit, endpoint},
        };

        fn memory_state() -> FlowLikeState {
            let store = FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ));
            FlowLikeState::new(
                crate::state::FlowLikeConfig::with_default_store(store),
                crate::utils::http::HTTPClient::new_without_refetch(),
            )
        }

        fn is_endpoint_model(model: &Arc<dyn EmbeddingModelLogic>) -> bool {
            model.as_cacheable().as_any().is::<EndpointEmbeddingModel>()
        }

        #[tokio::test]
        async fn device_embeddings_use_the_connector_endpoint() {
            let connector = FakeConnector::serving(endpoint("http://127.0.0.1:9/v1"));
            let mut state = memory_state();
            state.device_model_connector = Some(connector.clone());
            let usage_context = ModelUsageContext {
                run_id: Some("run-3".to_string()),
                ..ModelUsageContext::default()
            };

            let model = EmbeddingFactory::new()
                .build_text_routed(
                    &device_embedding_bit("device-embedding", "dev-1"),
                    Arc::new(state),
                    None,
                    Some(usage_context),
                )
                .await
                .expect("device embedding through the connector");

            assert!(is_endpoint_model(&model));
            assert_eq!(
                *connector.interactions.lock(),
                vec![Interaction::Allowed {
                    run_label: Some("run-3".to_string())
                }]
            );
        }

        #[tokio::test]
        async fn device_embeddings_without_a_connector_are_server_execution_errors() {
            let error = match EmbeddingFactory::new()
                .build_text_routed(
                    &device_embedding_bit("device-embedding", "dev-1"),
                    Arc::new(memory_state()),
                    Some("user-token".to_string()),
                    None,
                )
                .await
            {
                Ok(_) => panic!("a server cannot reach device embeddings"),
                Err(error) => error,
            };

            assert_eq!(
                model_unavailable(&error).map(|unavailable| unavailable.reason),
                Some(ModelUnavailableReason::ServerExecution)
            );
        }

        #[tokio::test]
        async fn the_router_serves_local_embeddings_from_the_model_host() {
            let mut state = memory_state();
            state.local_model_router =
                Some(Arc::new(FakeRouter(endpoint("http://127.0.0.1:9/v1"))));

            let model = EmbeddingFactory::new()
                .build_text_routed(&embedding_bit("Local"), Arc::new(state), None, None)
                .await
                .expect("a routed Local embedding needs no local runtime");

            assert!(is_endpoint_model(&model));
        }
    }
}
