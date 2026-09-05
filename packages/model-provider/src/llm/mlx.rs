use std::{any::Any, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use serde_json::Value;

use super::{
    ModelConstructor, ModelLogic, UsageReportingMode, llamacpp::LlamaCppClient,
    merge_additional_params,
};
use crate::history::History;
use crate::provider::ModelProvider;

/// MLX exposes the same local OpenAI-compatible endpoint as llama.cpp to the
/// rest of the model-provider layer. The endpoint itself is backed by the
/// native MLX Swift bridge rather than by llama-server.
pub struct MlxModel {
    client: LlamaCppClient,
    _provider: ModelProvider,
    additional_params: Option<Value>,
    default_model: Option<String>,
    port: u16,
}

impl MlxModel {
    pub async fn new(provider: &ModelProvider, port: u16, bearer_token: &str) -> Result<Self> {
        Self::new_inner(provider, port, bearer_token, None).await
    }

    pub async fn new_with_keepalive(
        provider: &ModelProvider,
        port: u16,
        bearer_token: &str,
        keepalive: Arc<dyn Send + Sync>,
    ) -> Result<Self> {
        Self::new_inner(provider, port, bearer_token, Some(keepalive)).await
    }

    async fn new_inner(
        provider: &ModelProvider,
        port: u16,
        bearer_token: &str,
        keepalive: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Self> {
        let additional_params = provider.params.clone().and_then(|mut params| {
            // These fields are owned by the local OpenAI transport and must not
            // be replaced by persistent provider configuration.
            for reserved in ["messages", "model", "stream", "tool_choice", "tools"] {
                params.remove(reserved);
            }
            (!params.is_empty()).then(|| Value::Object(params.into_iter().collect()))
        });

        let mut client = LlamaCppClient::new_with_bearer_token(
            &format!("http://127.0.0.1:{port}"),
            bearer_token,
        );
        if let Some(keepalive) = keepalive {
            client = client.with_keepalive(keepalive);
        }

        Ok(Self {
            client,
            _provider: provider.clone(),
            additional_params,
            default_model: provider.model_id.clone(),
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Cacheable for MlxModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MlxModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history_params = history
            .as_ref()
            .and_then(|history| history.build_additional_params().ok().flatten());
        // Provider params are defaults; explicit request/history values win.
        merge_additional_params(self.additional_params.clone(), history_params)
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::OpenAIStreamOptions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn forwards_generation_defaults_but_filters_transport_fields() {
        let provider = ModelProvider {
            api_surface: None,
            provider_name: "MLX".to_string(),
            model_id: Some("test-model".to_string()),
            version: None,
            params: Some(HashMap::from([
                ("max_kv_size".to_string(), json!(4096)),
                ("stream".to_string(), json!(true)),
                ("messages".to_string(), json!([])),
            ])),
        };
        let model = MlxModel::new(&provider, 1, "test-token").await.unwrap();
        let params = model.additional_params(&None).expect("provider defaults");

        assert_eq!(params["max_kv_size"], 4096);
        assert!(params.get("stream").is_none());
        assert!(params.get("messages").is_none());
    }

    #[tokio::test]
    async fn returned_client_keeps_runtime_lease_alive() {
        let provider = ModelProvider {
            api_surface: None,
            provider_name: "MLX".to_string(),
            model_id: Some("test-model".to_string()),
            version: None,
            params: None,
        };
        let dropped = Arc::new(AtomicBool::new(false));
        let keepalive: Arc<dyn Send + Sync> = Arc::new(DropProbe(dropped.clone()));
        let model = MlxModel::new_with_keepalive(&provider, 1, "test-token", keepalive)
            .await
            .unwrap();
        let client = model.provider().await.unwrap().into_client();

        drop(model);
        assert!(!dropped.load(Ordering::Acquire));

        drop(client);
        assert!(dropped.load(Ordering::Acquire));
    }
}
