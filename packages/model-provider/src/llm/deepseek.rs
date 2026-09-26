use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, UsageReportingMode, body_params, extract_headers,
    merge_additional_params, output_budget_as_body_param,
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
use rig::completion::CompletionRequest;
use rig::message::ToolChoice;
use serde_json::{Value, json};

const DEEPSEEK: ParamDialect = ParamDialect {
    provider: "deepseek",
    renames: &[("user", "user_id")],
    unsupported: &OPENROUTER_ONLY,
    rig_owned: &[],
};

/// `deepseek-flash` and `deepseek-v4-pro` think by default and take `thinking` plus
/// `reasoning_effort` (none/low/high/max).
fn supports_thinking(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("deepseek-flash") || model.contains("deepseek-v4")
}

fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    if !supports_thinking(model) {
        return None;
    }
    Some(match thinking {
        HistoryThinking::Off => json!({ "thinking": { "type": "disabled" } }),
        HistoryThinking::Low => json!({ "reasoning_effort": "low" }),
        HistoryThinking::Mid | HistoryThinking::High => json!({ "reasoning_effort": "high" }),
    })
}

/// Thinking mode returns 400 for required or named tool choices, so a forced call runs
/// without thinking.
fn deepseek_constraints(model: &str, request: CompletionRequest) -> CompletionRequest {
    let mut request = output_budget_as_body_param(request, "max_tokens");
    let forced = matches!(
        request.tool_choice,
        Some(ToolChoice::Required | ToolChoice::Specific { .. })
    );
    if !forced || !supports_thinking(model) {
        return request;
    }
    let params = body_params(&mut request);
    let disabled = params
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        == Some("disabled")
        || params.get("reasoning_effort").and_then(Value::as_str) == Some("none");
    if !disabled {
        params.remove("reasoning_effort");
        params.insert("thinking".into(), json!({ "type": "disabled" }));
        tracing::warn!(
            model,
            "DeepSeek rejects forced tool_choice in thinking mode; disabling thinking for this request"
        );
    }
    request
}

pub struct DeepseekModel {
    client: rig::providers::deepseek::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl DeepseekModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let deepseek_config = random_provider(&config.deepseek_config)?;
        let api_key = deepseek_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::deepseek::Client::builder().api_key(&api_key);

        if let Some(endpoint) = deepseek_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(DeepseekModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::deepseek::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(DeepseekModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for DeepseekModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for DeepseekModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_request_fixup(
            self.client.clone(),
            deepseek_constraints,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::OpenAIStreamOptions
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(DEEPSEEK.params(history)),
            history
                .thinking
                .and_then(|thinking| reasoning(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forced_request(params: Value) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(2048),
            tool_choice: Some(ToolChoice::Required),
            additional_params: Some(params),
            output_schema: None,
        }
    }

    #[test]
    fn thinking_maps_to_documented_controls() {
        assert_eq!(
            reasoning("deepseek-v4-pro", HistoryThinking::Off),
            Some(json!({"thinking": {"type": "disabled"}}))
        );
        assert_eq!(
            reasoning("deepseek-flash", HistoryThinking::Mid),
            Some(json!({"reasoning_effort": "high"}))
        );
        assert_eq!(reasoning("some-other-model", HistoryThinking::High), None);
    }

    #[test]
    fn forced_tool_choice_turns_default_thinking_off() {
        let fixed = deepseek_constraints(
            "deepseek-v4-pro",
            forced_request(json!({"reasoning_effort": "high"})),
        );
        let params = fixed.additional_params.unwrap();

        assert_eq!(params["thinking"], json!({"type": "disabled"}));
        assert!(params.get("reasoning_effort").is_none());
        assert_eq!(params["max_tokens"], 2048);
    }

    #[test]
    fn user_id_is_deepseeks_name_for_user() {
        let mut history = History::new("deepseek-flash".to_string(), Vec::new());
        history.user = Some("user-1".into());
        let params = DEEPSEEK.params(&history);

        assert_eq!(params["user_id"], "user-1");
        assert!(params.get("user").is_none());
    }
}
