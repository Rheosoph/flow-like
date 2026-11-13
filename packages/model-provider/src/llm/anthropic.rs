use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, body_params, drop_body_param, drop_temperature,
    extract_headers,
    media::{self, MediaDialect},
    prompt_cache::{CacheBreakpoints, CacheDialect},
    unforce_tool_choice, user_media,
};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    authorization::AuthorizedHttpClient,
    egress::ProviderEgress,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::{CompletionError, CompletionRequest};
use rig::message::ToolChoice;
use serde_json::{Value, json};

pub struct AnthropicModel {
    client: rig::providers::anthropic::Client<CacheBreakpoints>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl AnthropicModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let anthropic_config = random_provider(&config.anthropic_config)?;
        let api_key = anthropic_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::anthropic::Client::builder()
            .api_key(&api_key)
            .http_client(CacheBreakpoints::new(
                AuthorizedHttpClient::default(),
                CacheDialect::Messages,
            ));
        if let Some(endpoint) = anthropic_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        if let Some(beta) = anthropic_config.beta.as_deref() {
            builder = builder.anthropic_beta(beta);
        }

        if let Some(version) = anthropic_config.version.as_deref() {
            builder = builder.anthropic_version(version);
        }

        let client = builder.build()?;

        Ok(AnthropicModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::anthropic::Client::builder()
            .api_key(api_key)
            .http_client(CacheBreakpoints::new(
                AuthorizedHttpClient::for_provider(provider, egress)?,
                CacheDialect::Messages,
            ));
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }
        if let Some(beta) = params.get("beta").and_then(|v| v.as_str()) {
            builder = builder.anthropic_beta(beta);
        }

        if let Some(version) = params.get("version").and_then(|v| v.as_str()) {
            builder = builder.anthropic_version(version);
        }

        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(AnthropicModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for AnthropicModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for AnthropicModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(
            user_media::constructor(self.client.clone(), anthropic_request)
                .with_messages_api_usage(),
        )
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        Some(messages_params(history, Claude::parse(model)))
    }
}

const MESSAGES_API: ParamDialect = ParamDialect {
    provider: "anthropic",
    renames: &[("stop", "stop_sequences")],
    unsupported: &[
        "seed",
        "presence_penalty",
        "frequency_penalty",
        "n",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &[],
};

const MIN_THINKING_BUDGET: u64 = 1024;

/// The largest output any current Claude model accepts.
const MAX_OUTPUT_TOKENS: u64 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeFamily {
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Mythos,
}

/// How a model accepts thinking configuration (Anthropic's per-model thinking table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeThinking {
    /// Thinking stays on and `output_config.effort` is the control. Opus 5 accepts
    /// `disabled`, but its docs warn it then leaks tool calls into text.
    EffortOnly,
    /// `adaptive` or `disabled`, with `output_config.effort`.
    Adaptive,
    /// Only `enabled` with `budget_tokens`; incompatible with forced tool use.
    Budget,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Claude {
    family: ClaudeFamily,
    version: (u32, u32),
}

impl Claude {
    /// Parses ids like `claude-opus-4-7`, `claude-sonnet-4-5-20250929`,
    /// `claude-3-7-sonnet-latest` and `anthropic.claude-haiku-4-5@…`.
    fn parse(model: &str) -> Option<Self> {
        let model = model.to_ascii_lowercase();
        let rest = &model[model.find("claude")? + "claude".len()..];
        let mut family = None;
        let mut version = Vec::with_capacity(2);
        for token in rest
            .split(['-', '@', ':', '.', '_'])
            .filter(|t| !t.is_empty())
        {
            let named = match token {
                "opus" => Some(ClaudeFamily::Opus),
                "sonnet" => Some(ClaudeFamily::Sonnet),
                "haiku" => Some(ClaudeFamily::Haiku),
                "fable" => Some(ClaudeFamily::Fable),
                "mythos" => Some(ClaudeFamily::Mythos),
                _ => None,
            };
            if named.is_some() {
                family = named;
                continue;
            }
            match token.parse::<u32>() {
                Ok(number) if number < 100 && version.len() < 2 => version.push(number),
                _ => break,
            }
        }
        let family = family?;
        let major = version.first().copied();
        if major.is_none() && !matches!(family, ClaudeFamily::Fable | ClaudeFamily::Mythos) {
            return None;
        }
        Some(Self {
            family,
            version: (major.unwrap_or(0), version.get(1).copied().unwrap_or(0)),
        })
    }

    fn thinking(self) -> ClaudeThinking {
        use ClaudeFamily::*;
        match (self.family, self.version) {
            (Fable | Mythos, _) => ClaudeThinking::EffortOnly,
            (Opus, version) if version >= (5, 0) => ClaudeThinking::EffortOnly,
            (Opus | Sonnet, version) if version >= (4, 6) => ClaudeThinking::Adaptive,
            (Opus | Sonnet, version) if version >= (4, 0) => ClaudeThinking::Budget,
            (Sonnet, (3, 7)) => ClaudeThinking::Budget,
            (Haiku, version) if version >= (4, 5) => ClaudeThinking::Budget,
            _ => ClaudeThinking::Unsupported,
        }
    }

    /// Forced `tool_choice` (`any`/`tool`) returns 400 on these models, on every request.
    fn rejects_forced_tool_use(self) -> bool {
        use ClaudeFamily::*;
        match self.family {
            Fable => self.version >= (5, 1),
            Mythos => self.version >= (5, 1) || self.version == (0, 0),
            Opus => self.version >= (5, 5),
            Sonnet | Haiku => false,
        }
    }

    /// Anthropic's published output ceiling. Later generations keep the newest one.
    fn max_output_tokens(self) -> u64 {
        use ClaudeFamily::*;
        match (self.family, self.version) {
            (Fable | Mythos, _) => MAX_OUTPUT_TOKENS,
            (Opus | Sonnet, version) if version >= (4, 6) => MAX_OUTPUT_TOKENS,
            (Haiku, version) if version >= (5, 0) => MAX_OUTPUT_TOKENS,
            (Opus, version) if version >= (4, 5) => 64_000,
            (Opus, version) if version >= (4, 0) => 32_000,
            (Sonnet, version) if version >= (3, 7) => 64_000,
            (Haiku, version) if version >= (4, 5) => 64_000,
            (Sonnet | Haiku, version) if version >= (3, 5) => 8_192,
            _ => 4_096,
        }
    }

    /// Non-default `temperature`/`top_p`/`top_k` return 400 on these models, thinking or not.
    fn rejects_sampling(self) -> bool {
        use ClaudeFamily::*;
        match self.family {
            Fable | Mythos => true,
            Opus => self.version >= (4, 7),
            Sonnet => self.version >= (5, 0),
            Haiku => false,
        }
    }
}

fn effort(thinking: HistoryThinking) -> &'static str {
    match thinking {
        HistoryThinking::Off | HistoryThinking::Low => "low",
        HistoryThinking::Mid => "medium",
        HistoryThinking::High => "high",
    }
}

fn thinking_budget(thinking: HistoryThinking) -> u64 {
    match thinking {
        HistoryThinking::Off | HistoryThinking::Low => MIN_THINKING_BUDGET,
        HistoryThinking::Mid => 4096,
        HistoryThinking::High => 16_384,
    }
}

/// OpenAI's `response_format: {type: "json_schema", json_schema: {schema}}` as Anthropic's
/// `output_config.format`. Other formats have no Messages API equivalent.
fn output_format(response_format: &Value) -> Option<Value> {
    let schema = response_format
        .get("json_schema")
        .and_then(|json_schema| json_schema.get("schema"))
        .filter(|_| response_format.get("type").and_then(Value::as_str) == Some("json_schema"))?;
    Some(json!({ "type": "json_schema", "schema": schema }))
}

fn messages_params(history: &History, claude: Option<Claude>) -> Value {
    let mut params = MESSAGES_API.params(history);
    let Some(object) = params.as_object_mut() else {
        return params;
    };
    if let Some(user) = object.remove("user") {
        object.insert("metadata".into(), json!({ "user_id": user }));
    }

    let mut output_config = serde_json::Map::new();
    if let Some(response_format) = object.remove("response_format") {
        match output_format(&response_format) {
            Some(format) => {
                output_config.insert("format".into(), format);
            }
            None => tracing::warn!(
                provider = "anthropic",
                "Only json_schema response formats map to output_config.format; dropping it"
            ),
        }
    }

    if let (Some(claude), Some(thinking)) = (claude, history.thinking) {
        match claude.thinking() {
            ClaudeThinking::EffortOnly => {
                output_config.insert("effort".into(), json!(effort(thinking)));
            }
            ClaudeThinking::Adaptive if thinking == HistoryThinking::Off => {
                object.insert("thinking".into(), json!({ "type": "disabled" }));
            }
            ClaudeThinking::Adaptive => {
                object.insert("thinking".into(), json!({ "type": "adaptive" }));
                output_config.insert("effort".into(), json!(effort(thinking)));
            }
            ClaudeThinking::Budget if thinking != HistoryThinking::Off => {
                object.insert(
                    "thinking".into(),
                    json!({ "type": "enabled", "budget_tokens": thinking_budget(thinking) }),
                );
            }
            ClaudeThinking::Budget | ClaudeThinking::Unsupported => {}
        }
    }

    if !output_config.is_empty() {
        object.insert("output_config".into(), Value::Object(output_config));
    }
    params
}

/// Anthropic requires `max_tokens`, so an unset budget sends the model's whole output ceiling.
/// An endpoint serving a non-Claude ID with a lower ceiling rejects it; its provider then needs
/// `max_tokens` in the request parameters.
fn max_output_tokens(model: &str) -> u64 {
    Claude::parse(model).map_or(MAX_OUTPUT_TOKENS, Claude::max_output_tokens)
}

fn thinking_type(request: &CompletionRequest) -> Option<&str> {
    request
        .additional_params
        .as_ref()?
        .pointer("/thinking/type")
        .and_then(Value::as_str)
}

fn anthropic_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let request = media::retain_supported(MediaDialect::Anthropic, model, request)?;
    Ok(enforce_model_constraints(model, request))
}

/// Applies Anthropic's per-model limits to the final request, including the fields agents set
/// on the builder (`tool_choice`, `temperature`) that the History mapping cannot see.
fn enforce_model_constraints(model: &str, mut request: CompletionRequest) -> CompletionRequest {
    let max_tokens = *request
        .max_tokens
        .get_or_insert_with(|| max_output_tokens(model));
    let Some(claude) = Claude::parse(model) else {
        return request;
    };

    if claude.rejects_forced_tool_use() {
        unforce_tool_choice(&mut request, model);
    }

    if thinking_type(&request) == Some("enabled") {
        let forced = matches!(
            request.tool_choice,
            Some(ToolChoice::Required | ToolChoice::Specific { .. })
        );
        let ceiling = max_tokens.saturating_sub(1);
        let params = body_params(&mut request);
        let requested = params
            .get("thinking")
            .and_then(|thinking| thinking.get("budget_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(MIN_THINKING_BUDGET);
        let budget = requested.min(ceiling);
        if forced || budget < MIN_THINKING_BUDGET {
            params.remove("thinking");
            tracing::warn!(
                model,
                forced_tool_choice = forced,
                max_tokens_ceiling = ceiling,
                "Budget thinking needs tool_choice auto/none and max_tokens above {MIN_THINKING_BUDGET}; sending the request without thinking"
            );
        } else {
            params.insert(
                "thinking".into(),
                json!({ "type": "enabled", "budget_tokens": budget }),
            );
        }
    }

    let thinking_on = claude.thinking() == ClaudeThinking::EffortOnly
        || matches!(thinking_type(&request), Some("adaptive" | "enabled"));
    let top_p = request
        .additional_params
        .as_ref()
        .and_then(|params| params.get("top_p"))
        .and_then(Value::as_f64);

    if claude.rejects_sampling() {
        if request
            .temperature
            .is_some_and(|temperature| temperature != 1.0)
        {
            drop_temperature(&mut request, model, "rejected by this model");
        }
        if top_p.is_some_and(|top_p| top_p != 1.0) {
            drop_body_param(&mut request, "top_p", model, "rejected by this model");
        }
    } else if thinking_on {
        drop_temperature(&mut request, model, "incompatible with thinking");
        if top_p.is_some_and(|top_p| !(0.95..=1.0).contains(&top_p)) {
            drop_body_param(
                &mut request,
                "top_p",
                model,
                "must be between 0.95 and 1 with thinking",
            );
        }
    } else if claude.version.0 >= 4 && request.temperature.is_some() && top_p.is_some() {
        drop_body_param(
            &mut request,
            "top_p",
            model,
            "Claude 4 models accept temperature or top_p, not both",
        );
    }

    if request.output_schema.is_some()
        && request
            .additional_params
            .as_ref()
            .is_some_and(|params| params.get("output_config").is_some())
    {
        drop_body_param(
            &mut request,
            "output_config",
            model,
            "the structured output schema already sets output_config",
        );
    }

    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryMessage, ResponseFormat, Role};
    use crate::llm::test_support::{json_response, serve_once, sse_response};
    use futures::StreamExt;
    use rig::completion::Completion;
    use std::collections::HashMap;

    fn history(model: &str) -> History {
        History::new(
            model.to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Plan the next step.",
            )],
        )
    }

    fn request(params: Value) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: Some(0.3),
            max_tokens: Some(8192),
            tool_choice: None,
            additional_params: Some(params),
            output_schema: None,
        }
    }

    #[test]
    fn missing_output_limits_send_the_model_output_ceiling() {
        for (model, expected) in [
            ("claude-opus-5-5", 128_000),
            ("claude-sonnet-5-5", 128_000),
            ("claude-haiku-5-5", 128_000),
            ("claude-fable-5-1", 128_000),
            ("claude-mythos-preview", 128_000),
            ("claude-sonnet-4-6", 128_000),
            ("claude-opus-4-7", 128_000),
            ("claude-opus-4-5", 64_000),
            ("claude-sonnet-4-5-20250929", 64_000),
            ("anthropic.claude-haiku-4-5@20251001", 64_000),
            ("claude-opus-4-1-20250805", 32_000),
            ("claude-3-5-haiku-20241022", 8_192),
            ("claude-3-opus-20240229", 4_096),
            ("claude-opus-7", 128_000),
            ("hosted-assistant-bit", 128_000),
        ] {
            let mut unbounded = request(json!({}));
            unbounded.max_tokens = None;
            let fixed = anthropic_request(model, unbounded).unwrap();
            assert_eq!(fixed.max_tokens, Some(expected), "{model}");

            for limit in [0, 8192] {
                let mut explicit = request(json!({}));
                explicit.max_tokens = Some(limit);
                let fixed = anthropic_request(model, explicit).unwrap();
                assert_eq!(fixed.max_tokens, Some(limit), "{model}");
            }
        }
    }

    #[test]
    fn thinking_budget_fits_the_model_output_ceiling() {
        let mut unbounded = request(json!({
            "thinking": {"type": "enabled", "budget_tokens": 100_000},
        }));
        unbounded.max_tokens = None;

        let fixed = anthropic_request("claude-haiku-4-5", unbounded).unwrap();

        assert_eq!(fixed.max_tokens, Some(64_000));
        assert_eq!(
            fixed.additional_params.unwrap()["thinking"]["budget_tokens"],
            63_999
        );
    }

    #[test]
    fn parses_every_claude_id_style() {
        use ClaudeFamily::*;
        for (id, family, version) in [
            ("claude-opus-4-7", Opus, (4, 7)),
            ("claude-sonnet-4-5-20250929", Sonnet, (4, 5)),
            ("claude-sonnet-4-20250514", Sonnet, (4, 0)),
            ("claude-3-7-sonnet-latest", Sonnet, (3, 7)),
            ("anthropic.claude-haiku-4-5-20251001-v1:0", Haiku, (4, 5)),
            ("claude-opus-4-5@20251101", Opus, (4, 5)),
            ("claude-opus-5-5", Opus, (5, 5)),
            ("claude-fable-5-1", Fable, (5, 1)),
            ("claude-mythos-preview", Mythos, (0, 0)),
        ] {
            assert_eq!(Claude::parse(id), Some(Claude { family, version }), "{id}");
        }
        assert_eq!(Claude::parse("gpt-4o"), None);
        assert_eq!(Claude::parse("claude-instant-1"), None);
    }

    #[test]
    fn thinking_mode_follows_the_model_table() {
        for (id, expected) in [
            ("claude-fable-5-1", ClaudeThinking::EffortOnly),
            ("claude-opus-5-5", ClaudeThinking::EffortOnly),
            ("claude-opus-5", ClaudeThinking::EffortOnly),
            ("claude-sonnet-5", ClaudeThinking::Adaptive),
            ("claude-opus-4-8", ClaudeThinking::Adaptive),
            ("claude-sonnet-4-6", ClaudeThinking::Adaptive),
            ("claude-opus-4-5", ClaudeThinking::Budget),
            ("claude-haiku-4-5", ClaudeThinking::Budget),
            ("claude-3-7-sonnet-latest", ClaudeThinking::Budget),
            ("claude-3-5-haiku-latest", ClaudeThinking::Unsupported),
        ] {
            assert_eq!(Claude::parse(id).unwrap().thinking(), expected, "{id}");
        }
    }

    #[test]
    fn history_fields_use_messages_api_names() {
        let mut history = history("claude-opus-4-7");
        history.thinking = Some(HistoryThinking::High);
        history.stop = Some(vec!["END".into()]);
        history.user = Some("user-1".into());
        history.seed = Some(7);
        history.presence_penalty = Some(0.5);
        history.response_format = Some(ResponseFormat::Object(json!({
            "type": "json_schema",
            "json_schema": {"name": "plan", "schema": {"type": "object"}},
        })));

        let params = messages_params(&history, Claude::parse("claude-opus-4-7"));

        assert_eq!(params["stop_sequences"], json!(["END"]));
        assert_eq!(params["metadata"], json!({"user_id": "user-1"}));
        assert_eq!(params["thinking"], json!({"type": "adaptive"}));
        assert_eq!(
            params["output_config"],
            json!({"effort": "high", "format": {"type": "json_schema", "schema": {"type": "object"}}})
        );
        for rejected in [
            "stop",
            "user",
            "seed",
            "presence_penalty",
            "response_format",
        ] {
            assert!(
                params.get(rejected).is_none(),
                "{rejected} must not reach Anthropic"
            );
        }
    }

    #[test]
    fn thinking_off_never_disables_effort_only_models() {
        let mut history = history("claude-opus-5-5");
        history.thinking = Some(HistoryThinking::Off);

        let effort_only = messages_params(&history, Claude::parse("claude-opus-5-5"));
        let adaptive = messages_params(&history, Claude::parse("claude-sonnet-4-6"));
        let budget = messages_params(&history, Claude::parse("claude-haiku-4-5"));

        assert!(effort_only.get("thinking").is_none());
        assert_eq!(effort_only["output_config"]["effort"], "low");
        assert_eq!(adaptive["thinking"], json!({"type": "disabled"}));
        assert!(budget.get("thinking").is_none());
    }

    #[test]
    fn current_models_drop_rejected_sampling() {
        let fixed = enforce_model_constraints(
            "claude-opus-4-7",
            request(json!({"top_p": 0.9, "output_config": {"effort": "high"}})),
        );

        assert_eq!(fixed.temperature, None);
        assert!(fixed.additional_params.unwrap().get("top_p").is_none());
    }

    #[test]
    fn budget_thinking_yields_to_forced_tool_use_and_small_budgets() {
        let thinking = json!({"thinking": {"type": "enabled", "budget_tokens": 16_384}});

        let mut forced = request(thinking.clone());
        forced.tool_choice = Some(ToolChoice::Required);
        let forced = enforce_model_constraints("claude-sonnet-4-5", forced);
        assert!(forced.additional_params.unwrap().get("thinking").is_none());
        assert_eq!(forced.temperature, Some(0.3));

        let clamped = enforce_model_constraints("claude-sonnet-4-5", request(thinking.clone()));
        assert_eq!(
            clamped.additional_params.unwrap()["thinking"]["budget_tokens"],
            8191
        );
        assert_eq!(
            clamped.temperature, None,
            "temperature is incompatible with thinking"
        );

        let mut tiny = request(thinking);
        tiny.max_tokens = Some(1024);
        let tiny = enforce_model_constraints("claude-sonnet-4-5", tiny);
        assert!(tiny.additional_params.unwrap().get("thinking").is_none());
    }

    #[test]
    fn forced_tool_use_becomes_an_instruction_where_it_is_rejected() {
        let forced = |model| {
            let mut request = request(json!({}));
            request.preamble = Some("Extract the fields.".into());
            request.tool_choice = Some(ToolChoice::Specific {
                function_names: vec!["submit_extraction".into()],
            });
            enforce_model_constraints(model, request)
        };

        for model in [
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-mythos-preview",
        ] {
            let fixed = forced(model);
            assert!(
                matches!(fixed.tool_choice, Some(ToolChoice::Auto)),
                "{model}"
            );
            assert_eq!(
                fixed.preamble.as_deref(),
                Some("Extract the fields.\n\nRespond by calling the `submit_extraction` tool."),
                "{model}"
            );
        }
        for model in ["claude-opus-5", "claude-fable-5", "claude-sonnet-4-6"] {
            assert!(
                matches!(forced(model).tool_choice, Some(ToolChoice::Specific { .. })),
                "{model} still accepts forced tool use"
            );
        }
    }

    #[test]
    fn unreadable_attachments_are_replaced_for_unrecognised_model_ids_too() {
        let mut attached = request(json!({}));
        attached.chat_history = rig::OneOrMany::one(rig::completion::Message::User {
            content: rig::OneOrMany::one(rig::message::UserContent::document_url(
                "https://bucket.s3.amazonaws.com/report.docx?X-Amz-Signature=secret",
                None,
            )),
        });

        for model in ["claude-sonnet-4-6", "hosted-assistant-bit"] {
            let fixed = anthropic_request(model, attached.clone()).unwrap();
            let rig::completion::Message::User { content } = fixed.chat_history.first() else {
                panic!("{model}: the user message changed role");
            };
            assert!(
                matches!(
                    content.first(),
                    rig::message::UserContent::Text(text)
                        if text.text == "[attachment omitted: unsupported file type document]"
                ),
                "{model}"
            );
        }
    }

    #[test]
    fn a_pdf_link_the_run_did_not_inline_fails_naming_its_host() {
        let mut attached = request(json!({}));
        attached.chat_history = rig::OneOrMany::one(rig::completion::Message::User {
            content: rig::OneOrMany::one(rig::message::UserContent::document_url(
                "https://bucket.s3.amazonaws.com/report.pdf?X-Amz-Signature=secret",
                Some(rig::message::DocumentMediaType::PDF),
            )),
        });

        let error = anthropic_request("claude-sonnet-4-6", attached)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains(
                "The document (application/pdf) from host 'bucket.s3.amazonaws.com' cannot be \
                 sent to Anthropic as a link"
            ),
            "{error}"
        );
        assert!(
            !error.contains("X-Amz") && !error.contains("report.pdf"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn custom_agents_send_missing_output_limits_for_both_transports() {
        for model_id in ["claude-opus-5-5", "hosted-assistant-bit"] {
            for streaming in [false, true] {
                let message = json!({
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "model": model_id,
                    "content": [{"type": "text", "text": "ok"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 12, "output_tokens": 1},
                });
                let response = if streaming {
                    sse_response(&[
                        json!({"type": "message_start", "message": message}),
                        json!({
                            "type": "content_block_delta", "index": 0,
                            "delta": {"type": "text_delta", "text": "ok"},
                        }),
                        json!({
                            "type": "message_delta", "delta": {"stop_reason": "end_turn"},
                            "usage": {"output_tokens": 1},
                        }),
                    ])
                } else {
                    json_response(message)
                };
                let (endpoint, server) = serve_once(response).await;
                let model = super::super::custom::build(
                    "custom:anthropic",
                    &ModelProvider {
                        provider_name: "custom:anthropic".into(),
                        model_id: Some(model_id.into()),
                        version: None,
                        api_surface: None,
                        params: Some(HashMap::from([
                            ("api_key".into(), json!("test-key")),
                            ("model_id".into(), json!(model_id)),
                            ("endpoint".into(), json!(endpoint)),
                        ])),
                    },
                    ProviderEgress::Open,
                    super::super::custom::AmbientCredentials::Forbidden,
                )
                .await
                .unwrap();
                let agent = model
                    .provider()
                    .await
                    .unwrap()
                    .into_client()
                    .agent(model_id)
                    .build();
                let request = agent
                    .completion("hi", Vec::<rig::completion::Message>::new())
                    .await
                    .unwrap();
                if streaming {
                    let mut stream = request.stream().await.unwrap();
                    while let Some(chunk) = stream.next().await {
                        chunk.unwrap();
                    }
                } else {
                    request.send().await.unwrap();
                }

                let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
                assert_eq!(body["model"], model_id);
                assert_eq!(
                    body["max_tokens"], 128_000,
                    "{model_id}, stream={streaming}"
                );
                assert_eq!(body["stream"].as_bool().unwrap_or(false), streaming);
            }
        }
    }

    const STABLE: &str = "You are Ada, one of your owner's bots.\n\n## WHAT YOU CAN DO\nSearch.";
    const CONTEXT: &str = "## CONTEXT\nIt is Friday, 09:30.";

    fn cached_message(usage: Value) -> Value {
        json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-5-5",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "usage": usage,
        })
    }

    async fn messages_api(api_key: &str, model_id: &str, endpoint: Option<&str>) -> AnthropicModel {
        let mut params = HashMap::from([
            ("api_key".to_string(), json!(api_key)),
            ("model_id".to_string(), json!(model_id)),
        ]);
        params.extend(endpoint.map(|endpoint| ("endpoint".to_string(), json!(endpoint))));
        let provider = ModelProvider {
            provider_name: "anthropic".to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(params),
        };
        AnthropicModel::from_provider(&provider, ProviderEgress::Open)
            .await
            .unwrap()
    }

    async fn streamed_usage(
        request: rig::completion::CompletionRequestBuilder<crate::llm::CompletionModelHandle<'_>>,
    ) -> rig::completion::Usage {
        use rig::completion::GetTokenUsage;
        use rig::streaming::StreamedAssistantContent;

        let mut stream = request.stream().await.unwrap();
        let mut usage = None;
        while let Some(chunk) = stream.next().await {
            if let StreamedAssistantContent::Final(response) = chunk.unwrap() {
                usage = response.token_usage();
            }
        }
        usage.expect("the stream closes with its usage")
    }

    /// A reply that read 4096 prompt tokens from cache and wrote 8. Its stream states the cache
    /// counts in `message_start` only.
    fn cached_reply(streaming: bool) -> String {
        let usage = json!({
            "input_tokens": 40,
            "cache_read_input_tokens": 4096,
            "cache_creation_input_tokens": 8,
            "output_tokens": 3,
        });
        if !streaming {
            return json_response(cached_message(usage));
        }
        sse_response(&[
            json!({"type": "message_start", "message": cached_message(usage)}),
            json!({
                "type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "ok"},
            }),
            json!({
                "type": "message_delta", "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": 3},
            }),
        ])
    }

    /// One bot turn against a recorded Messages API: the body it sent and the usage it read.
    async fn bot_turn(streaming: bool, stable: Option<&str>) -> (Value, rig::completion::Usage) {
        let (endpoint, server) = serve_once(cached_reply(streaming)).await;
        let agent = messages_api("test-key", "claude-opus-5-5", Some(&endpoint))
            .await
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent("claude-opus-5-5")
            .preamble(&format!("{STABLE}\n\n{CONTEXT}"))
            .build();
        let call = async {
            let request = agent
                .completion("Hi", Vec::<rig::completion::Message>::new())
                .await
                .unwrap()
                .tools(vec![crate::llm::test_support::typeless_tool()]);
            if streaming {
                streamed_usage(request).await
            } else {
                request.send().await.unwrap().usage
            }
        };
        let usage = match stable {
            Some(stable) => {
                super::super::prompt_cache::with_stable_system_prefix(stable.to_string(), call)
                    .await
            }
            None => call.await,
        };
        (serde_json::from_str(&server.await.unwrap()).unwrap(), usage)
    }

    #[tokio::test]
    async fn bot_turns_cache_the_stable_head_and_growing_conversation() {
        for streaming in [false, true] {
            let (body, _) = bot_turn(streaming, Some(STABLE)).await;

            assert_eq!(
                body["system"],
                json!([
                    {"type": "text", "text": STABLE, "cache_control": {"type": "ephemeral"}},
                    {"type": "text", "text": CONTEXT},
                ]),
                "stream={streaming}"
            );
            assert_eq!(
                body.to_string().matches("cache_control").count(),
                2,
                "a fixed head and an automatic conversation breakpoint: {body}"
            );
            assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
            assert_eq!(body["tools"][0]["name"], "interact_app_page");
        }
    }

    #[tokio::test]
    async fn calls_without_a_stable_head_send_one_unmarked_system_block() {
        for streaming in [false, true] {
            let (body, _) = bot_turn(streaming, None).await;
            assert_eq!(
                body["system"],
                json!([{"type": "text", "text": format!("{STABLE}\n\n{CONTEXT}")}]),
                "stream={streaming}"
            );
            assert!(!body.to_string().contains("cache_control"), "{body}");
        }
    }

    #[tokio::test]
    async fn usage_counts_cached_prompt_tokens_as_input() {
        for streaming in [false, true] {
            let (_, usage) = bot_turn(streaming, Some(STABLE)).await;
            assert_eq!(usage.input_tokens, 40 + 4096 + 8, "stream={streaming}");
            assert_eq!(usage.cached_input_tokens, 4096, "stream={streaming}");
            assert_eq!(usage.cache_creation_input_tokens, 8, "stream={streaming}");
            assert_eq!(usage.total_tokens, 40 + 4096 + 8 + 3, "stream={streaming}");
            let metered = crate::response::Usage::from_rig(usage);
            assert_eq!(metered.prompt_tokens, 4144, "stream={streaming}");
            assert_eq!(
                metered.prompt_tokens + metered.completion_tokens,
                metered.total_tokens
            );
        }
    }

    /// `ANTHROPIC_API_KEY=… cargo test -p flow-like-model-provider --lib -- --ignored
    /// live_second_call_reads_the_stable_head_from_cache --nocapture`; `ANTHROPIC_CACHE_MODEL`
    /// picks the model (default `claude-opus-5-5`).
    #[tokio::test]
    #[ignore = "calls the live Anthropic API set by ANTHROPIC_API_KEY"]
    async fn live_second_call_reads_the_stable_head_from_cache() {
        dotenv::dotenv().ok();
        let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") else {
            println!("Skipping the Anthropic cache test: ANTHROPIC_API_KEY is not set");
            return;
        };
        let model_id =
            std::env::var("ANTHROPIC_CACHE_MODEL").unwrap_or_else(|_| "claude-opus-5-5".into());
        let client = messages_api(&api_key, &model_id, None)
            .await
            .provider()
            .await
            .unwrap()
            .into_client();
        let stable: String = std::iter::once(
            "You are Ada, a terse assistant. Answer in one short sentence.".to_string(),
        )
        .chain((1..=600).map(|rule| {
            format!("Rule {rule}: when asked about item {rule}, say it is stocked in aisle {rule}.")
        }))
        .collect::<Vec<_>>()
        .join("\n");
        let mut reads = Vec::new();
        for (turn, clock) in ["09:30", "09:31"].into_iter().enumerate() {
            let agent = client
                .agent(&model_id)
                .preamble(&format!(
                    "{stable}\n\n## CONTEXT\nIt is {clock}. Turn {turn}."
                ))
                .max_tokens(256)
                .build();
            let usage =
                super::super::prompt_cache::with_stable_system_prefix(stable.clone(), async {
                    let request = agent
                        .completion("Where is item 7?", Vec::<rig::completion::Message>::new())
                        .await
                        .unwrap();
                    streamed_usage(request).await
                })
                .await;
            println!("turn {turn}: {usage:?}");
            reads.push(usage);
        }
        assert!(
            reads[1].cached_input_tokens > 0,
            "the second call read nothing from cache: {:?}",
            reads[1]
        );
        assert!(reads[1].input_tokens > reads[1].cached_input_tokens);
    }

    #[tokio::test]
    async fn agent_request_body_is_accepted_by_the_messages_api_schema() {
        let (endpoint, server) = serve_once(json_response(json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-7",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 12, "output_tokens": 4096},
        })))
        .await;
        let model = AnthropicModel::from_provider(
            &ModelProvider {
                provider_name: "anthropic".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), json!("test-key")),
                    ("model_id".to_string(), json!("claude-opus-4-7")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();
        let mut history = history("claude-opus-4-7");
        history.thinking = Some(HistoryThinking::Mid);
        history.temperature = Some(0.2);
        history.stop = Some(vec!["END".into()]);
        history.max_completion_tokens = Some(100_000);
        let settings = model.agent_settings(&Some(history)).unwrap();
        let mut builder = model
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent("claude-opus-4-7")
            .additional_params(settings.params.unwrap())
            .max_tokens(settings.max_tokens.unwrap());
        if let Some(temperature) = settings.temperature {
            builder = builder.temperature(temperature);
        }

        let response = builder
            .build()
            .completion(
                "Plan the next step.",
                Vec::<rig::completion::Message>::new(),
            )
            .await
            .unwrap()
            .send()
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 100_000);
        assert_eq!(body["thinking"], json!({"type": "adaptive"}));
        assert_eq!(body["output_config"], json!({"effort": "medium"}));
        assert_eq!(body["stop_sequences"], json!(["END"]));
        for rejected in ["temperature", "stop", "stream"] {
            assert!(body.get(rejected).is_none(), "{rejected} in {body}");
        }
        assert_eq!(
            response.raw_response.finish_reason.as_deref(),
            Some("length")
        );
    }
}
