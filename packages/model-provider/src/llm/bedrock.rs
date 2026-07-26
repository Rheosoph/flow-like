use flow_like_types::Value;

use super::openai::OpenAIModel;
use crate::provider::ModelProvider;

/// AWS Bedrock via its OpenAI-compatible chat completions endpoint
/// (`https://bedrock-runtime.{region}.amazonaws.com/openai/v1`) authenticated
/// with a Bedrock API key. HTTP-only — no SigV4 signing — so it works
/// identically on desktop, server, and proxied browser execution.
pub struct BedrockModel;

impl BedrockModel {
    /// Expected params: `api_key` (required), `region` (defaults to
    /// `us-east-1`), optional `endpoint` override, `model_id`.
    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<OpenAIModel> {
        let mut params = provider.params.clone().unwrap_or_default();

        let endpoint = params
            .get("endpoint")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| {
                let endpoint = endpoint.trim_end_matches('/');
                if endpoint.contains("/openai") {
                    endpoint.to_string()
                } else {
                    format!("{endpoint}/openai/v1")
                }
            })
            .unwrap_or_else(|| {
                let region = params
                    .get("region")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|region| !region.is_empty())
                    .unwrap_or("us-east-1");
                format!("https://bedrock-runtime.{region}.amazonaws.com/openai/v1")
            });

        params.insert("endpoint".to_string(), Value::String(endpoint));

        let mut provider = provider.clone();
        provider.params = Some(params);
        OpenAIModel::from_provider(&provider).await
    }
}
