use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, extract_headers, merge_additional_params,
    output_budget_as_body_param,
};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use serde_json::{Value, json};

/// Rig merges these parameters into Ollama's `options`, which skips unknown names with a
/// warning; `think` stays top-level.
const OLLAMA: ParamDialect = ParamDialect {
    provider: "ollama",
    renames: &[],
    unsupported: &[
        "user",
        "n",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &["stream"],
};

/// `think: true` on a model without thinking returns 400, so it is sent only for known
/// thinking families. gpt-oss takes levels and cannot turn thinking off.
fn think(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    if model.contains("gpt-oss") {
        let level = match thinking {
            HistoryThinking::Off | HistoryThinking::Low => "low",
            HistoryThinking::Mid => "medium",
            HistoryThinking::High => "high",
        };
        return Some(json!({ "think": level }));
    }
    let name = model.rsplit('/').next().unwrap_or(&model);
    let thinks = name == "qwen3"
        || name.starts_with("qwen3:")
        || ["deepseek-r1", "deepseek-v3.1", "magistral", "thinking"]
            .iter()
            .any(|family| name.contains(family));
    thinks.then(|| json!({ "think": thinking != HistoryThinking::Off }))
}

pub struct OllamaModel {
    client: rig::providers::ollama::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl OllamaModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let ollama_config = random_provider(&config.ollama_config)?;
        let model_id = provider.model_id.clone();
        let endpoint = ollama_config
            .endpoint
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let mut builder = rig::providers::ollama::Client::builder().api_key(rig::client::Nothing);
        builder = builder.base_url(&endpoint);

        let client = builder.build()?;

        Ok(OllamaModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:11434");
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::ollama::Client::builder().api_key(rig::client::Nothing);
        builder = builder.base_url(endpoint);
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(OllamaModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for OllamaModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OllamaModel {
    #[allow(deprecated)]
    /// `/api/chat` ignores a top-level `max_tokens`; the budget is `options.num_predict`.
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_request_fixup(
            self.client.clone(),
            |_, request| output_budget_as_body_param(request, "num_predict"),
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(OLLAMA.params(history)),
            history.thinking.and_then(|thinking| think(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{json_response, serve_once};

    #[test]
    fn think_only_for_known_thinking_models() {
        assert_eq!(
            think("gpt-oss:20b", HistoryThinking::Mid),
            Some(json!({"think": "medium"}))
        );
        assert_eq!(
            think("qwen3:8b", HistoryThinking::High),
            Some(json!({"think": true}))
        );
        assert_eq!(
            think("deepseek-r1:14b", HistoryThinking::Off),
            Some(json!({"think": false}))
        );
        assert_eq!(think("qwen3-coder:30b", HistoryThinking::High), None);
        assert_eq!(think("llama3.2", HistoryThinking::High), None);
    }

    #[tokio::test]
    async fn output_budget_reaches_num_predict() {
        let (endpoint, server) = serve_once(json_response(json!({
            "model": "qwen3:8b",
            "created_at": "2026-09-24T00:00:00Z",
            "message": {"role": "assistant", "content": "ok"},
            "done": true,
            "done_reason": "length",
            "prompt_eval_count": 12,
            "eval_count": 64,
        })))
        .await;
        let model = OllamaModel::from_provider(&ModelProvider {
            provider_name: "ollama".to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(std::collections::HashMap::from([
                ("model_id".to_string(), json!("qwen3:8b")),
                ("endpoint".to_string(), json!(endpoint)),
            ])),
        })
        .await
        .unwrap();
        let mut history = History::new(
            "qwen3:8b".to_string(),
            vec![crate::history::HistoryMessage::from_string(
                crate::history::Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(64);
        history.thinking = Some(HistoryThinking::High);
        history.top_p = Some(0.9);

        let response = model.invoke(&history, None).await.unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["options"]["num_predict"], 64);
        assert!(body["options"]["top_p"].as_f64().is_some());
        assert_eq!(body["think"], true);
        assert!(body["options"].get("stream").is_none());
        assert_eq!(response.choices[0].finish_reason, "length");
    }
}
