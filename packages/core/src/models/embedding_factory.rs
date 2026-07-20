use std::{collections::HashMap, sync::Arc, time::SystemTime};

use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, openai::OpenAIEmbeddingModel},
    image_embedding::ImageEmbeddingModelLogic,
};

use crate::{bit::Bit, state::FlowLikeState};

#[cfg(any(feature = "remote-ml", test))]
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
/// models. Proxying those would make an otherwise offline-capable run depend on
/// the backend, so hosts with `local-ml` always keep them local; the proxy stays
/// reserved for models that have no local implementation.
pub fn prefers_local_execution(bit: &Bit) -> bool {
    if !cfg!(feature = "local-ml") {
        return false;
    }

    bit.try_to_embedding_provider()
        .is_some_and(|provider| is_local_provider(&provider.provider_name))
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

        let proxy_model = ProxyEmbeddingModel::new(
            embedding_provider,
            bit.id.clone(),
            access_token,
            usage_headers,
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

    fn headers(usage_context: Option<&ModelUsageContext>) -> HashMap<String, String> {
        embedding_usage_headers(usage_context).into_iter().collect()
    }

    #[test]
    fn offline_embedding_usage_omits_app_header_but_keeps_run_header() {
        let usage_context = ModelUsageContext {
            app_id: None,
            run_id: Some("run-1".to_string()),
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
        };
        let headers = headers(Some(&usage_context));

        assert_eq!(headers["x-flow-like-app-id"], "app-1");
        assert_eq!(headers["x-flow-like-run-id"], "run-1");
    }

    #[test]
    fn missing_embedding_usage_context_adds_no_headers() {
        assert!(headers(None).is_empty());
    }
}
