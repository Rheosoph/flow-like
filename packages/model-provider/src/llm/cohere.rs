use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, body_params, merge_additional_params,
    output_budget_as_body_param,
    user_media::{self, MediaSplice, Part},
};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    egress::ProviderEgress,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::{CompletionError, CompletionRequest};
use rig::message::UserContent;
use serde_json::{Value, json};

const NAME: &str = "Cohere";

/// Cohere v2 rejects unknown fields and names its sampling fields differently.
const COHERE: ParamDialect = ParamDialect {
    provider: "cohere",
    renames: &[("top_p", "p"), ("stop", "stop_sequences")],
    unsupported: &[
        "user",
        "n",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &[],
};

/// Cohere recommends leaving at least 1K tokens of `max_tokens` for the response.
const RESPONSE_RESERVE: u64 = 1024;

fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    if !model.to_ascii_lowercase().contains("reasoning") {
        return None;
    }
    let token_budget = match thinking {
        HistoryThinking::Off => return Some(json!({ "thinking": { "type": "disabled" } })),
        HistoryThinking::Low => 1024,
        HistoryThinking::Mid => 4096,
        HistoryThinking::High => 16_384,
    };
    Some(json!({ "thinking": { "type": "enabled", "token_budget": token_budget } }))
}

fn cohere_constraints(model: &str, request: CompletionRequest) -> CompletionRequest {
    let mut request = output_budget_as_body_param(request, "max_tokens");
    let Some(max_tokens) = request.max_tokens else {
        return request;
    };
    let params = body_params(&mut request);
    let budget = params
        .get("thinking")
        .and_then(|thinking| thinking.get("token_budget"))
        .and_then(Value::as_u64);
    if let Some(budget) = budget
        && budget.saturating_add(RESPONSE_RESERVE) > max_tokens
        && let Some(thinking) = params.get_mut("thinking").and_then(Value::as_object_mut)
    {
        thinking.remove("token_budget");
        tracing::warn!(
            model,
            budget,
            max_tokens,
            "Thinking budget leaves no room for the response; using the model's default budget"
        );
    }
    request
}

/// Rig's converter refuses every user part but text; images travel past it as `image_url`
/// parts, which Cohere's vision models read from public links and `data:` URLs.
fn cohere_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    user_media::deliver(NAME, model, cohere_constraints(model, request), media_part)
}

fn media_part(part: &UserContent) -> Result<Part, CompletionError> {
    match part {
        UserContent::Image(image) => user_media::web_image(NAME, image, true),
        part => user_media::text_only(NAME, part),
    }
}

pub struct CohereModel {
    client: rig::providers::cohere::Client<MediaSplice>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl CohereModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let cohere_config = random_provider(&config.cohere_config)?;
        let api_key = cohere_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::cohere::Client::builder()
            .api_key(&api_key)
            .http_client(MediaSplice::default());

        if let Some(endpoint) = cohere_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(CohereModel {
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

        let mut builder = rig::providers::cohere::Client::builder()
            .api_key(api_key)
            .http_client(MediaSplice::for_provider(provider, egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(CohereModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for CohereModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for CohereModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(self.client.clone(), cohere_request))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(COHERE.params(history)),
            history
                .thinking
                .and_then(|thinking| reasoning(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_fields_use_cohere_names() {
        let mut history = History::new("command-a-03-2025".to_string(), Vec::new());
        history.top_p = Some(0.9);
        history.stop = Some(vec!["END".into()]);
        history.user = Some("user-1".into());
        let params = COHERE.params(&history);

        assert_eq!(params["stop_sequences"], json!(["END"]));
        assert!(params["p"].as_f64().is_some());
        for rejected in ["top_p", "stop", "user"] {
            assert!(params.get(rejected).is_none(), "{rejected} in {params}");
        }
    }

    #[test]
    fn thinking_budget_leaves_room_for_the_response() {
        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(4096),
            tool_choice: None,
            additional_params: reasoning("command-a-reasoning-08-2025", HistoryThinking::High),
            output_schema: None,
        };

        let params = cohere_constraints("command-a-reasoning-08-2025", request)
            .additional_params
            .unwrap();

        assert_eq!(params["thinking"], json!({"type": "enabled"}));
        assert_eq!(params["max_tokens"], 4096);
        assert_eq!(reasoning("command-a-03-2025", HistoryThinking::High), None);
    }

    async fn cohere_request_body(content: Vec<UserContent>) -> Value {
        use crate::llm::test_support::{json_response, serve_once};

        let (endpoint, server) = serve_once(json_response(json!({
            "id": "chat-test",
            "finish_reason": "COMPLETE",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}]},
            "usage": {"tokens": {"input_tokens": 12, "output_tokens": 1}},
        })))
        .await;
        let model = CohereModel::from_provider(
            &ModelProvider {
                provider_name: "cohere".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([
                    ("api_key".to_string(), json!("key")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();
        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("command-a-vision-07-2025")
            .completion(user_media::tests::request(content))
            .await
            .unwrap();
        serde_json::from_str(&server.await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn images_reach_cohere_and_other_media_become_text() {
        use rig::message::{DocumentMediaType, DocumentSourceKind, ImageMediaType};
        use user_media::tests::{PRESIGNED_PNG, audio, document, image};

        let body = cohere_request_body(vec![
            UserContent::text("Describe both."),
            image(
                DocumentSourceKind::Url(PRESIGNED_PNG.to_string()),
                Some(ImageMediaType::PNG),
            ),
            image(
                DocumentSourceKind::Url("data:image/webp;base64,UklGRiQAAABXRUJQ".to_string()),
                None,
            ),
            document(
                DocumentSourceKind::String("a,b\n1,2".to_string()),
                Some(DocumentMediaType::CSV),
            ),
            audio(DocumentSourceKind::Base64("UklGRg==".to_string())),
        ])
        .await;

        assert_eq!(
            body["messages"][1],
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Describe both."},
                    {"type": "image_url", "image_url": {"url": PRESIGNED_PNG}},
                    {"type": "image_url", "image_url": {"url": "data:image/webp;base64,UklGRiQAAABXRUJQ"}},
                    {"type": "text", "text": "a,b\n1,2"},
                    {"type": "text", "text": "[attachment omitted: unsupported file type audio/wav]"},
                ],
            })
        );
    }

    #[tokio::test]
    async fn a_text_document_no_longer_fails_the_call() {
        use rig::message::{DocumentMediaType, DocumentSourceKind};

        let body = cohere_request_body(vec![
            UserContent::text("Summarise."),
            user_media::tests::document(
                DocumentSourceKind::String("Quarterly notes".to_string()),
                Some(DocumentMediaType::TXT),
            ),
        ])
        .await;

        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": [{"type": "text", "text": "Summarise."}]})
        );
        assert_eq!(
            body["messages"][2],
            json!({"role": "user", "content": [{"type": "text", "text": "Quarterly notes"}]})
        );
    }
}
