use std::{any::Any, sync::Arc};

use flow_like_model_provider::{
    provider::{is_hosted_provider_name, random_provider},
    systemone::{SystemOneClient, SystemOneModelLogic, SystemOneRequest, SystemOneResponse},
};
use flow_like_types::{Cacheable, Result, anyhow, async_trait, bail};

use super::llm::{ModelFactory, ModelUsageContext, local::LocalModel};
use crate::{bit::Bit, state::FlowLikeState};

struct LocalSystemOneModel {
    client: SystemOneClient,
    // Keep the model process alive for the full lifetime of every retained client.
    _server: Arc<LocalModel>,
}

impl Cacheable for LocalSystemOneModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl SystemOneModelLogic for LocalSystemOneModel {
    async fn invoke(&self, request: &SystemOneRequest) -> Result<SystemOneResponse> {
        self.client.invoke(request).await
    }
}

impl ModelFactory {
    /// Build a native typed-decision model. Chat models cannot be substituted for this API.
    pub async fn build_systemone(
        &self,
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        access_token: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Arc<dyn SystemOneModelLogic>> {
        let parameters = bit
            .try_to_systemone()
            .ok_or_else(|| anyhow!("Expected a SystemOne Bit with valid model parameters"))?;
        if parameters.context_length == 0 {
            bail!("SystemOne context length must be positive");
        }
        let provider = parameters.provider;
        let name = provider.provider_name.trim().to_ascii_lowercase();

        if let Some(endpoint) =
            super::device::resolve_endpoint(bit, &app_state, usage_context.as_ref()).await?
        {
            return self
                .systemone_models
                .get_or_build(&endpoint.cache_key(), || async {
                    Ok(Arc::new(SystemOneClient::new(
                        &endpoint.base_url,
                        &endpoint.model,
                        &endpoint.bearer,
                    )?) as Arc<dyn SystemOneModelLogic>)
                })
                .await;
        }

        if name == "local" {
            let capabilities = FlowLikeState::completion_model_capabilities(&app_state).await;
            if !capabilities.local_server {
                app_state.runtime_locator.installed_llama_server()?;
                bail!(
                    "Local SystemOne inference requires local model support and a filesystem-backed Bit store"
                );
            }
            let settings = self.execution_settings.read().clone();
            return self
                .systemone_models
                .get_or_build(&bit.runtime_model_cache_key(), || async {
                    let server = LocalModel::new(bit, app_state, &settings).await?;
                    let model = provider
                        .model_id
                        .as_deref()
                        .filter(|id| !id.trim().is_empty())
                        .unwrap_or(&bit.id);
                    let client = SystemOneClient::new(
                        &format!("http://127.0.0.1:{}/v1", server.port),
                        model,
                        "",
                    )?;
                    Ok(Arc::new(LocalSystemOneModel {
                        client,
                        _server: server,
                    }) as Arc<dyn SystemOneModelLogic>)
                })
                .await;
        }

        if is_hosted_provider_name(&name) {
            if !matches!(
                name.as_str(),
                "hosted:openrouter"
                    | "hosted:typesafe"
                    | "hosted:cloudflare"
                    | "hosted:systemone_compatible"
                    | "hosted"
                    | "premium"
                    | "internal"
            ) {
                bail!("Provider {name} does not support native SystemOne inference");
            }
            let authorizer = app_state.request_authorizer.clone();
            let token = app_state
                .hosted_model_token
                .as_deref()
                .or(access_token.as_deref())
                .map(str::trim)
                .filter(|token| !token.is_empty());
            if token.is_none() && authorizer.is_none() {
                bail!("Hosted SystemOne inference requires an access token for the API proxy");
            }
            let api_base = usage_context
                .as_ref()
                .and_then(|context| context.api_base_url.as_deref())
                .map(str::trim)
                .filter(|base| !base.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(flow_like_model_provider::embedding::proxy_config::api_base_url);
            let base = api_base.trim_end_matches('/');
            let base = if base.ends_with("/api/v1") {
                base.to_owned()
            } else {
                format!("{base}/api/v1")
            };
            // Catalog metadata must never choose where a user's proxy credential is sent.
            let mut client = SystemOneClient::new(&base, &bit.id, token.unwrap_or_default())?
                .with_usage_headers(usage_headers(usage_context.as_ref()));
            if let Some(authorizer) = authorizer {
                client = client.with_authorizer(authorizer)?;
            }
            return Ok(Arc::new(client));
        }

        let params = provider.params.as_ref();
        let param = |key: &str| {
            params
                .and_then(|params| params.get(key))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };
        let model = provider
            .model_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .or_else(|| param("model_id"))
            .ok_or_else(|| anyhow!("SystemOne provider requires a model_id"))?;
        let client = match name.as_str() {
            "custom:systemone" | "custom:typesafe" | "custom:openrouter" => {
                let default_endpoint = match name.as_str() {
                    "custom:typesafe" => Some("https://api.typesafe.ai/v1"),
                    "custom:openrouter" => Some("https://openrouter.ai/api/v1"),
                    _ => None,
                };
                let endpoint = param("endpoint")
                    .or(default_endpoint)
                    .ok_or_else(|| anyhow!("Custom SystemOne providers require an endpoint"))?;
                SystemOneClient::new(endpoint, model, param("api_key").unwrap_or_default())?
            }
            "openrouter" => {
                let config = random_provider(&app_state.model_provider_config.openrouter_config)?;
                SystemOneClient::new(
                    config
                        .endpoint
                        .as_deref()
                        .unwrap_or("https://openrouter.ai/api/v1"),
                    model,
                    config.api_key.as_deref().unwrap_or_default(),
                )?
            }
            _ => bail!("Provider {name} does not support native SystemOne inference"),
        };
        Ok(Arc::new(client))
    }
}

fn usage_headers(context: Option<&ModelUsageContext>) -> Vec<(String, String)> {
    let Some(context) = context else {
        return Vec::new();
    };
    [
        ("x-flow-like-app-id", context.app_id.as_deref()),
        ("x-flow-like-run-id", context.run_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| (name.to_string(), value.to_owned()))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bit::BitTypes, state::FlowLikeConfig, utils::http::HTTPClient};
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_types::json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::{Duration, timeout},
    };

    fn state() -> FlowLikeState {
        FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ))),
            HTTPClient::new_without_refetch(),
        )
    }

    fn bit(provider: &str) -> Bit {
        Bit {
            id: "opaque-decision-bit".into(),
            bit_type: BitTypes::SystemOne,
            parameters: json!({
                "context_length": 512,
                "provider": {
                    "provider_name": provider,
                    "model_id": "upstream-model",
                    "params": {
                        "endpoint": "https://untrusted.invalid/v1",
                        "api_key": "catalog-key",
                        "headers": {"x-flow-like-app-id": "catalog-app"}
                    }
                }
            }),
            ..Bit::default()
        }
    }

    #[tokio::test]
    async fn decision_bits_cannot_be_used_as_chat_and_chat_bits_cannot_be_decision_models() {
        let factory = ModelFactory::new();
        assert!(
            factory
                .build(
                    &bit("hosted:openrouter"),
                    Arc::new(state()),
                    Some("token".into()),
                    None
                )
                .await
                .is_err()
        );
        let mut chat = bit("hosted:openrouter");
        chat.bit_type = BitTypes::Llm;
        assert!(
            factory
                .build_systemone(&chat, Arc::new(state()), Some("token".into()), None)
                .await
                .is_err()
        );
        assert!(
            factory
                .build_systemone(
                    &bit("hosted:openai"),
                    Arc::new(state()),
                    Some("token".into()),
                    None
                )
                .await
                .is_err()
        );
        assert!(
            factory
                .build_systemone(&bit("hosted:openrouter"), Arc::new(state()), None, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn hosted_decisions_use_the_trusted_proxy_bit_id_and_run_credential() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let captured = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (start, len) = loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    break (end + 4, len);
                }
            };
            while bytes.len() < start + len {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&buffer[..read]);
            }
            let body = json!({"model":"upstream-model", "answers":{"refund":{"type":"noul","noul":0.9}}, "usage":{"input_tokens":50,"output_tokens":0}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            (
                String::from_utf8(bytes[..start].to_vec()).unwrap(),
                serde_json::from_slice::<serde_json::Value>(&bytes[start..start + len]).unwrap(),
            )
        });
        let mut state = state();
        state.hosted_model_token = Some("run-token".into());
        let model = ModelFactory::new()
            .build_systemone(
                &bit("hosted:openrouter"),
                Arc::new(state),
                Some("visitor-token".into()),
                Some(ModelUsageContext {
                    api_base_url: Some(endpoint),
                    app_id: Some("trusted-app".into()),
                    run_id: Some("trusted-run".into()),
                }),
            )
            .await
            .unwrap();
        let request: SystemOneRequest = serde_json::from_value(json!({"state":"Refund please", "questions":{"refund":{"type":"noul","instructions":"Does the customer want a refund?"}}})).unwrap();
        timeout(Duration::from_secs(5), model.invoke(&request))
            .await
            .unwrap()
            .unwrap();
        let (headers, body) = captured.await.unwrap();
        let headers = headers.to_ascii_lowercase();
        assert!(headers.starts_with("post /api/v1/systemone http/1.1"));
        assert!(headers.contains("authorization: bearer run-token"));
        assert!(headers.contains("x-flow-like-app-id: trusted-app"));
        assert!(!headers.contains("catalog-"));
        assert!(!headers.contains("visitor-token"));
        assert_eq!(body["model"], "opaque-decision-bit");
        assert_eq!(body["questions"]["refund"]["type"], "noul");
        assert!(body.get("stream").is_none());
    }
}
