//! Clients for `custom:*` Bits, built from the provider parameters their owner stored.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail, ensure};

use super::{
    ModelLogic, anthropic::AnthropicModel, bedrock::BedrockModel, cohere::CohereModel,
    deepseek::DeepseekModel, galadriel::GaladrielModel, gemini::GeminiModel, groq::GroqModel,
    huggingface::HuggingfaceModel, hyperbolic::HyperbolicModel, lmstudio::LMStudioModel,
    mira::MiraModel, mistral::MistralModel, moonshot::MoonshotModel, mozilla::MozillaModel,
    ollama::OllamaModel, openai::OpenAIModel, openrouter::OpenRouterModel,
    perplexity::PerplexityModel, together::TogetherModel, vertex::VertexModel,
    voyageai::VoyageAIModel, xai::XAIModel,
};
use crate::{
    egress::ProviderEgress,
    history::History,
    llm::{
        AgentSettings, LLMCallback, ModelConstructor, UsageReportingMode, with_output_budget,
        without_output_budget,
    },
    provider::{ModelApiSurface, ModelProvider},
    response::Response,
    systemone::SystemOneClient,
};
use async_trait::async_trait;
use serde_json::Value;

#[cfg(test)]
mod request_params_tests;

/// Whether a client may fall back to the host process's own cloud identity when the Bit
/// carries no credentials. Server-side that identity belongs to the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientCredentials {
    Allowed,
    Forbidden,
}

/// Builds the provider's own client for a `custom:*` chat Bit.
pub async fn build(
    provider_name: &str,
    provider: &ModelProvider,
    egress: ProviderEgress,
    ambient: AmbientCredentials,
) -> Result<Arc<dyn ModelLogic>> {
    ensure_no_ambient_credentials(provider_name, provider, ambient)?;
    let name = provider_name.trim().to_ascii_lowercase();
    let model: Arc<dyn ModelLogic> = match name.as_str() {
        // Rig's OpenAI client has always defaulted to the Responses API here.
        "custom:openai" => Arc::new(
            OpenAIModel::from_provider(provider, egress)
                .await?
                .with_api_surface(provider.api_surface.unwrap_or(ModelApiSurface::Responses)),
        ),
        "custom:bedrock" => Arc::new(BedrockModel::from_provider(provider, egress).await?),
        "custom:anthropic" => Arc::new(AnthropicModel::from_provider(provider, egress).await?),
        "custom:gemini" => Arc::new(GeminiModel::from_provider(provider, egress).await?),
        "custom:groq" => Arc::new(GroqModel::from_provider(provider, egress).await?),
        "custom:cohere" => Arc::new(CohereModel::from_provider(provider, egress).await?),
        "custom:perplexity" => Arc::new(PerplexityModel::from_provider(provider, egress).await?),
        "custom:xai" => Arc::new(XAIModel::from_provider(provider, egress).await?),
        "custom:deepseek" => Arc::new(DeepseekModel::from_provider(provider, egress).await?),
        "custom:mistral" => Arc::new(MistralModel::from_provider(provider, egress).await?),
        "custom:ollama" => Arc::new(OllamaModel::from_provider(provider, egress).await?),
        "custom:huggingface" => Arc::new(HuggingfaceModel::from_provider(provider, egress).await?),
        "custom:together" => Arc::new(TogetherModel::from_provider(provider, egress).await?),
        "custom:openrouter" => Arc::new(OpenRouterModel::from_provider(provider, egress).await?),
        "custom:voyageai" => Arc::new(VoyageAIModel::from_provider(provider, egress).await?),
        "custom:hyperbolic" => Arc::new(HyperbolicModel::from_provider(provider, egress).await?),
        "custom:moonshot" => Arc::new(MoonshotModel::from_provider(provider, egress).await?),
        "custom:galadriel" => Arc::new(GaladrielModel::from_provider(provider, egress).await?),
        "custom:mira" => Arc::new(MiraModel::from_provider(provider, egress).await?),
        "custom:mozilla" => Arc::new(MozillaModel::from_provider(provider, egress).await?),
        "custom:lmstudio" => Arc::new(LMStudioModel::from_provider(provider, egress).await?),
        "custom:vertex" => Arc::new(VertexModel::from_provider(provider, egress).await?),
        _ => bail!("Unsupported custom provider: {provider_name}"),
    };
    Ok(match configured_output_budget(&name, provider) {
        Some(budget) => Arc::new(ConfiguredBudgetModel {
            inner: model,
            budget,
        }),
        None => model,
    })
}

/// A configured null (or non-numeric value) removes local budget inference. The exact value
/// still reaches the provider, which owns validation of its native request parameters.
fn configured_output_budget(name: &str, provider: &ModelProvider) -> Option<Option<u64>> {
    let params = provider.params.as_ref()?.get("request_params")?;
    let nested = |key: &str, limit: &str| {
        let value = params.get(key)?;
        if !value.is_object() {
            return Some(None);
        }
        value.get(limit).map(Value::as_u64)
    };
    match name {
        "custom:gemini" | "custom:vertex" => nested("generationConfig", "maxOutputTokens"),
        "custom:ollama" => nested("options", "num_predict"),
        "custom:xai" | "custom:mozilla" => params.get("max_output_tokens").map(Value::as_u64),
        "custom:openai"
            if !provider.params.as_ref().is_some_and(|params| {
                params.get("is_azure").and_then(Value::as_bool) == Some(true)
            }) && provider.api_surface.unwrap_or(ModelApiSurface::Responses)
                == ModelApiSurface::Responses =>
        {
            params.get("max_output_tokens").map(Value::as_u64)
        }
        "custom:openai" | "custom:bedrock" => params
            .get("max_completion_tokens")
            .or_else(|| params.get("max_tokens"))
            .map(Value::as_u64),
        _ => params.get("max_tokens").map(Value::as_u64),
    }
}

struct ConfiguredBudgetModel {
    inner: Arc<dyn ModelLogic>,
    budget: Option<u64>,
}

#[async_trait]
impl ModelLogic for ConfiguredBudgetModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(self.inner.provider().await?.with_output_budget(self.budget))
    }

    async fn default_model(&self) -> Option<String> {
        self.inner.default_model().await
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        self.inner.additional_params(history)
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        self.inner.usage_reporting()
    }

    fn transform_history(&self, history: &mut History) {
        self.inner.transform_history(history);
    }

    fn request_params(&self, history: &History, streaming: bool) -> Result<Option<Value>> {
        self.inner.request_params(history, streaming)
    }

    fn agent_settings(&self, history: &Option<History>) -> Result<AgentSettings> {
        let mut settings = self.inner.agent_settings(history)?;
        settings.max_tokens = self.budget;
        Ok(settings)
    }

    async fn invoke(&self, history: &History, callback: Option<LLMCallback>) -> Result<Response> {
        let mut history = history.clone();
        // History stores u32, while native bodies and response accounting support u64. The
        // final HTTP overlay keeps the full value; this bounded hint prepares model constraints.
        history.max_completion_tokens =
            self.budget.map(|budget| budget.min(u32::MAX as u64) as u32);
        // A caller callback may invoke another model. Its request must not inherit this limit.
        let callback = callback.map(|callback| -> LLMCallback {
            Arc::new(move |chunk| {
                let callback = callback.clone();
                Box::pin(without_output_budget(async move { callback(chunk).await }))
            })
        });
        with_output_budget(self.budget, async {
            // Keep provider-specific invocation, including Gemini's media preparation.
            self.inner.invoke(&history, callback).await
        })
        .await
    }
}

/// `custom:vertex` falls back to Google application-default credentials when the Bit
/// carries neither a service-account key nor an access token, i.e. to the host's identity.
pub fn ensure_no_ambient_credentials(
    provider_name: &str,
    provider: &ModelProvider,
    ambient: AmbientCredentials,
) -> Result<()> {
    if ambient == AmbientCredentials::Allowed
        || !provider_name.trim().eq_ignore_ascii_case("custom:vertex")
    {
        return Ok(());
    }
    let explicit = provider.params.as_ref().is_some_and(|params| {
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
    ensure!(
        explicit,
        "custom:vertex: auth mode 'application_default' resolves the host's ambient \
         credentials, which is not permitted in server-side execution. Provide explicit \
         credentials (service-account key or access token) instead."
    );
    Ok(())
}

/// The native SystemOne client for `custom:systemone`, `custom:typesafe` and
/// `custom:openrouter` Bits.
pub fn systemone(
    provider_name: &str,
    provider: &ModelProvider,
    egress: ProviderEgress,
) -> Result<SystemOneClient> {
    let name = provider_name.trim().to_ascii_lowercase();
    let default_endpoint = match name.as_str() {
        "custom:systemone" => None,
        "custom:typesafe" => Some("https://api.typesafe.ai/v1"),
        "custom:openrouter" => Some("https://openrouter.ai/api/v1"),
        _ => bail!("Provider {name} does not support native SystemOne inference"),
    };
    let param = |key: &str| {
        provider
            .params
            .as_ref()
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
    let endpoint = param("endpoint")
        .or(default_endpoint)
        .ok_or_else(|| anyhow!("Custom SystemOne providers require an endpoint"))?;
    let client = SystemOneClient::new(endpoint, model, param("api_key").unwrap_or_default())?
        .with_egress(egress)?
        .with_request_params(provider)?;
    Ok(if name == "custom:openrouter" {
        client.with_openrouter_state_images()
    } else {
        client
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{History, HistoryMessage, Role};
    use serde_json::json;
    use std::{collections::HashMap, sync::atomic::Ordering};

    fn provider(name: &str, params: serde_json::Value) -> ModelProvider {
        ModelProvider {
            provider_name: name.to_string(),
            model_id: Some("model".to_string()),
            version: None,
            api_surface: None,
            params: serde_json::from_value::<HashMap<String, serde_json::Value>>(params).ok(),
        }
    }

    #[tokio::test]
    async fn vertex_without_credentials_is_refused_when_ambient_identity_is_forbidden() {
        let bare = provider("custom:vertex", json!({"project_id": "my-project"}));
        let error = build(
            "custom:vertex",
            &bare,
            ProviderEgress::Public,
            AmbientCredentials::Forbidden,
        )
        .await
        .err()
        .expect("ambient credentials must be refused")
        .to_string();
        assert!(error.contains("ambient"), "{error}");
        assert!(
            ensure_no_ambient_credentials("custom:vertex", &bare, AmbientCredentials::Allowed)
                .is_ok()
        );

        let explicit = provider(
            "custom:vertex",
            json!({"project_id": "my-project", "access_token": "ya29.token"}),
        );
        assert!(
            build(
                "custom:vertex",
                &explicit,
                ProviderEgress::Public,
                AmbientCredentials::Forbidden,
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn builds_every_custom_provider_and_refuses_unknown_ones() {
        let params = json!({"api_key": "key", "model_id": "model", "endpoint": "https://models.example.com/v1"});
        for name in [
            "custom:openai",
            "custom:bedrock",
            "custom:anthropic",
            "custom:gemini",
            "custom:groq",
            "custom:cohere",
            "custom:perplexity",
            "custom:xai",
            "custom:deepseek",
            "custom:mistral",
            "custom:ollama",
            "custom:huggingface",
            "custom:together",
            "custom:openrouter",
            "custom:voyageai",
            "custom:hyperbolic",
            "custom:moonshot",
            "custom:galadriel",
            "custom:mira",
            "custom:mozilla",
            "custom:lmstudio",
        ] {
            let built = build(
                name,
                &provider(name, params.clone()),
                ProviderEgress::Public,
                AmbientCredentials::Forbidden,
            )
            .await;
            assert!(built.is_ok(), "{name}: {:?}", built.err());
        }
        for name in ["custom:codex", "openai", "hosted:openai"] {
            assert!(
                build(
                    name,
                    &provider(name, params.clone()),
                    ProviderEgress::Open,
                    AmbientCredentials::Allowed,
                )
                .await
                .is_err(),
                "{name}"
            );
        }
    }

    /// A client built without the egress policy would connect to the loopback listener.
    #[tokio::test]
    async fn custom_providers_never_connect_to_a_refused_endpoint() {
        let (addr, served) = crate::egress::tests::answer_every_connection(
            "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        let params = json!({"api_key": "key", "model_id": "model", "endpoint": format!("https://{addr}/v1")});
        let mut azure = params.clone();
        azure["is_azure"] = json!(true);
        let mut history = History::new(
            "model".to_string(),
            vec![HistoryMessage::from_string(Role::User, "Hello")],
        );
        history.max_completion_tokens = Some(64);
        let cases = [
            "custom:openai",
            "custom:bedrock",
            "custom:anthropic",
            "custom:gemini",
            "custom:groq",
            "custom:cohere",
            "custom:perplexity",
            "custom:xai",
            "custom:deepseek",
            "custom:mistral",
            "custom:ollama",
            "custom:huggingface",
            "custom:together",
            "custom:openrouter",
            "custom:hyperbolic",
            "custom:moonshot",
            "custom:galadriel",
            "custom:mira",
            "custom:mozilla",
            "custom:lmstudio",
        ]
        .map(|name| (name, params.clone()));
        for (name, params) in cases.into_iter().chain([("custom:openai", azure)]) {
            let error = match build(
                name,
                &provider(name, params),
                ProviderEgress::Public,
                AmbientCredentials::Forbidden,
            )
            .await
            {
                Ok(model) => model
                    .invoke(&history, None)
                    .await
                    .expect_err("a loopback endpoint is refused"),
                Err(error) => error,
            };
            assert!(
                format!("{error:#}").contains("Model endpoint"),
                "{name}: {error:#}"
            );
            assert_eq!(
                served.load(Ordering::SeqCst),
                0,
                "{name} connected to a refused endpoint"
            );
        }
    }

    #[tokio::test]
    async fn guarded_providers_refuse_private_endpoints_before_sending() {
        let private =
            json!({"api_key": "key", "model_id": "model", "endpoint": "http://10.0.0.5:8000/v1"});
        let lmstudio = build(
            "custom:lmstudio",
            &provider("custom:lmstudio", private.clone()),
            ProviderEgress::Public,
            AmbientCredentials::Forbidden,
        )
        .await;
        assert!(lmstudio.is_err());
        assert!(
            build(
                "custom:lmstudio",
                &provider("custom:lmstudio", private),
                ProviderEgress::Private,
                AmbientCredentials::Forbidden,
            )
            .await
            .is_ok()
        );
    }

    #[test]
    fn systemone_resolves_endpoints_under_egress() {
        let typesafe = provider("custom:typesafe", json!({"api_key": "key"}));
        assert!(systemone("custom:typesafe", &typesafe, ProviderEgress::Public).is_ok());

        let plain = provider(
            "custom:systemone",
            json!({"endpoint": "http://10.0.0.7:8080/v1"}),
        );
        assert!(systemone("custom:systemone", &plain, ProviderEgress::Public).is_err());
        assert!(systemone("custom:systemone", &plain, ProviderEgress::Private).is_ok());
        assert!(
            systemone(
                "custom:systemone",
                &provider("custom:systemone", json!({})),
                ProviderEgress::Open
            )
            .is_err(),
            "custom:systemone has no default endpoint"
        );
        assert!(systemone("custom:anthropic", &typesafe, ProviderEgress::Open).is_err());
    }
}
