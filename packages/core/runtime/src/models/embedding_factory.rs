use std::{collections::HashMap, sync::Arc, time::SystemTime};

use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, openai::OpenAIEmbeddingModel},
    image_embedding::ImageEmbeddingModelLogic,
    provider::is_hosted_provider_name,
};

use crate::{bit::Bit, state::FlowLikeState};

use super::llm::ModelUsageContext;
#[cfg(feature = "local-ml")]
use super::{
    embedding::local::LocalEmbeddingModel, image_embedding::local::LocalImageEmbeddingModel,
};

#[cfg(feature = "remote-ml")]
use flow_like_model_provider::embedding::proxy::ProxyEmbeddingModel;

pub struct EmbeddingFactory {
    pub cached_text_models: HashMap<String, Arc<dyn EmbeddingModelLogic>>,
    pub cached_image_models: HashMap<String, Arc<dyn ImageEmbeddingModelLogic>>,
    pub ttl_list: HashMap<String, SystemTime>,
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
            cached_text_models: HashMap::new(),
            cached_image_models: HashMap::new(),
            ttl_list: HashMap::new(),
        }
    }

    pub async fn build_text(
        &mut self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModelLogic>> {
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
                if let Some(model) = self.cached_text_models.get(&bit.id) {
                    // update last used time
                    self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                    return Ok(model.clone());
                }

                let local_model = LocalEmbeddingModel::new(bit, app_state).await?;
                self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                self.cached_text_models
                    .insert(bit.id.clone(), local_model.clone());
                return Ok(local_model);
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
    /// A filesystem-backed host with local ML support keeps locally runnable
    /// Bits local. Other hosts proxy remote-capable Bits when an access token is
    /// available. A remote-capable Local Bit fails with a routing error when the
    /// proxy is unavailable; standard providers use their normal factory path.
    pub async fn build_text_routed(
        &mut self,
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

    pub async fn build_image(
        &mut self,
        bit: &Bit,
        _app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Arc<dyn ImageEmbeddingModelLogic>> {
        let provider = bit.try_to_image_embedding();
        if provider.is_none() {
            return Err(flow_like_types::anyhow!("Model type not supported"));
        }

        let provider = provider.ok_or(flow_like_types::anyhow!("Model type not supported"))?;
        let provider = provider.provider.provider_name;

        if is_local_provider(&provider) {
            #[cfg(feature = "local-ml")]
            {
                if let Some(model) = self.cached_image_models.get(&bit.id) {
                    self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                    return Ok(model.clone());
                }

                let local_model = LocalImageEmbeddingModel::new(bit, _app_state, self).await?;
                self.ttl_list.insert(bit.id.clone(), SystemTime::now());
                self.cached_image_models
                    .insert(bit.id.clone(), local_model.clone());
                return Ok(local_model);
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
        &mut self,
        bit: &Bit,
        access_token: String,
        usage_context: Option<ModelUsageContext>,
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
        let model: Arc<dyn EmbeddingModelLogic> = Arc::new(proxy_model);

        Ok(model)
    }

    pub fn gc(&mut self) {
        let mut to_remove = Vec::new();
        for id in self.cached_image_models.keys() {
            // check if the model was not used for 5 minutes
            let ttl = self.ttl_list.get(id).unwrap();
            if ttl.elapsed().unwrap().as_secs() > 300 {
                to_remove.push(id.clone());
            }
        }

        for id in self.cached_text_models.keys() {
            // check if the model was not used for 5 minutes
            let ttl = self.ttl_list.get(id).unwrap();
            if ttl.elapsed().unwrap().as_secs() > 300 {
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            self.cached_text_models.remove(&id);
            self.cached_image_models.remove(&id);
            self.ttl_list.remove(&id);
        }
    }
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
}
