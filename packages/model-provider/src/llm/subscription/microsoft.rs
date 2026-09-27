//! Microsoft 365 Copilot's Graph chat surface, adapted to Rig text completions.

use super::{REQUEST_TIMEOUT, http_client, param, validate_token};
use crate::{
    history::History,
    llm::{
        CompletionClientDyn, CompletionModelDyn, CompletionModelHandle, ModelConstructor,
        ModelLogic,
    },
    provider::ModelProvider,
};
use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use rig::{
    OneOrMany,
    agent::AgentBuilder,
    completion::{
        CompletionError, CompletionModel, CompletionRequest, CompletionResponse, GetTokenUsage,
        Usage,
    },
    message::{AssistantContent, Message, Text, UserContent},
    streaming::{RawStreamingChoice, StreamingCompletionResponse},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

const CONVERSATIONS_URL: &str = "https://graph.microsoft.com/beta/copilot/conversations";
const MODEL_ID: &str = "microsoft-365-copilot";
const MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct MicrosoftClient {
    http: reqwest::Client,
    access_token: String,
    timezone: String,
}

pub(super) struct MicrosoftModel {
    client: MicrosoftClient,
}

impl MicrosoftModel {
    pub(super) fn new(provider: &ModelProvider) -> Result<Self> {
        let access_token = param(provider, "access_token").ok_or_else(|| {
            anyhow!("Microsoft 365 Copilot requires a delegated Microsoft Graph access token")
        })?;
        validate_token(access_token, "Microsoft 365 Copilot")?;
        let model = super::model_id(provider, MODEL_ID);
        if model != MODEL_ID {
            bail!("Microsoft 365 Copilot does not support model selection; use '{MODEL_ID}'");
        }
        Ok(Self {
            client: MicrosoftClient {
                http: http_client(REQUEST_TIMEOUT)?,
                access_token: access_token.to_owned(),
                timezone: param(provider, "timezone").unwrap_or("Etc/UTC").to_owned(),
            },
        })
    }
}

#[async_trait]
impl ModelLogic for MicrosoftModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }
    async fn default_model(&self) -> Option<String> {
        Some(MODEL_ID.to_owned())
    }
    fn request_params(&self, history: &History, _streaming: bool) -> Result<Option<Value>> {
        if history.thinking.is_some() {
            bail!("Microsoft 365 Copilot does not support a thinking setting");
        }
        let mut params = history
            .build_additional_params()?
            .unwrap_or_else(|| json!({}));
        if let Some(object) = params.as_object_mut() {
            object.remove("stream");
            if !object.is_empty() {
                bail!(
                    "Microsoft 365 Copilot does not support model sampling or structured-output history settings"
                );
            }
        }
        Ok(None)
    }
}

impl CompletionClientDyn for MicrosoftClient {
    fn completion_model<'a>(&self, _model: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(MicrosoftCompletion {
            client: self.clone(),
        })
    }
    fn agent<'a>(&self, _model: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        AgentBuilder::new(CompletionModelHandle::new(Arc::new(MicrosoftCompletion {
            client: self.clone(),
        })))
    }
}

#[derive(Clone)]
struct MicrosoftCompletion {
    client: MicrosoftClient,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MicrosoftResponse {
    finish_reason: String,
    conversation_id: Option<String>,
}

impl GetTokenUsage for MicrosoftResponse {
    // Graph does not return model token usage.
    fn token_usage(&self) -> Option<Usage> {
        None
    }
}

fn unsupported(message: &str) -> CompletionError {
    CompletionError::RequestError(anyhow!("Microsoft 365 Copilot {message}").into())
}

fn request_body(
    request: &CompletionRequest,
    timezone: &str,
) -> std::result::Result<Value, CompletionError> {
    if !request.tools.is_empty()
        || matches!(
            request.tool_choice,
            Some(rig::message::ToolChoice::Required | rig::message::ToolChoice::Specific { .. })
        )
    {
        return Err(unsupported(
            "does not support caller-defined tools or tool choice; use a model with function calling for this node",
        ));
    }
    if request.output_schema.is_some() {
        return Err(unsupported(
            "does not support constrained structured output",
        ));
    }
    if request.temperature.is_some() || request.max_tokens.is_some() {
        return Err(unsupported(
            "does not expose temperature or a maximum token count",
        ));
    }
    if request
        .model
        .as_deref()
        .is_some_and(|model| model != MODEL_ID)
    {
        return Err(unsupported("does not expose model selection"));
    }
    if request
        .additional_params
        .as_ref()
        .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|o| !o.is_empty()))
    {
        return Err(unsupported("does not support additional model parameters"));
    }
    let messages: Vec<_> = request.chat_history.iter().collect();
    let mut context = Vec::new();
    if let Some(preamble) = request.preamble.as_deref().filter(|v| !v.is_empty()) {
        context.push(json!({"text": format!("Requested instructions:\n{preamble}")}));
    }
    let mut prompt = None;
    for (index, message) in messages.iter().enumerate() {
        let (role, text) = text_message(message)?;
        if index + 1 == messages.len() {
            if role != "User" {
                return Err(unsupported(
                    "requires the last message to be a user text message",
                ));
            }
            prompt = Some(text);
        } else {
            context.push(json!({"text": format!("{role}:\n{text}")}));
        }
    }
    for document in &request.documents {
        context.push(json!({"text": document.to_string()}));
    }
    let prompt = prompt
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| unsupported("requires a nonempty user message"))?;
    // Graph accepts grounding context rather than replayable role-tagged model messages.
    let mut body = json!({"message": {"text": prompt}, "locationHint": {"timeZone": timezone}});
    if !context.is_empty() {
        body["additionalContext"] = json!(context);
    }
    Ok(body)
}

fn text_message(message: &Message) -> std::result::Result<(&'static str, String), CompletionError> {
    match message {
        Message::System { content } => Ok(("Requested instructions", content.clone())),
        Message::User { content } => {
            let parts: std::result::Result<Vec<_>, _> = content.iter().map(|part| match part {
                UserContent::Text(text) => Ok(text.text.clone()),
                _ => Err(unsupported("accepts text here; image, audio, document attachments and tool results need a different provider")),
            }).collect();
            Ok(("User", parts?.join("\n")))
        }
        Message::Assistant { content, .. } => {
            let parts: std::result::Result<Vec<_>, _> = content
                .iter()
                .map(|part| match part {
                    AssistantContent::Text(text) => Ok(text.text.clone()),
                    _ => Err(unsupported("accepts text history only")),
                })
                .collect();
            Ok(("Assistant", parts?.join("\n")))
        }
    }
}

async fn response_json(
    response: reqwest::Response,
    operation: &str,
) -> std::result::Result<Value, CompletionError> {
    if !response.status().is_success() {
        return Err(CompletionError::ProviderError(format!(
            "Microsoft 365 Copilot {operation} returned HTTP {}; check delegated Graph consent and Copilot licensing",
            response.status()
        )));
    }
    response.json().await.map_err(|_| {
        CompletionError::ResponseError(format!(
            "Microsoft 365 Copilot {operation} returned invalid JSON"
        ))
    })
}

impl MicrosoftClient {
    async fn conversation_url(&self) -> std::result::Result<String, CompletionError> {
        let response = self
            .http
            .post(CONVERSATIONS_URL)
            .bearer_auth(&self.access_token)
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| {
                CompletionError::ProviderError(format!(
                    "Microsoft 365 Copilot conversation request failed: {e}"
                ))
            })?;
        let value = response_json(response, "create conversation").await?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                CompletionError::ResponseError(
                    "Microsoft 365 Copilot returned no conversation ID".into(),
                )
            })?;
        let mut url =
            reqwest::Url::parse(CONVERSATIONS_URL).expect("fixed Graph endpoint is valid");
        url.path_segments_mut()
            .expect("fixed Graph endpoint supports path segments")
            .push(id);
        Ok(url.into())
    }
}

fn response_message<'a>(value: &'a Value, prompt: &str) -> Option<&'a Value> {
    value
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|message| {
            message
                .get("@odata.type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.ends_with("copilotConversationResponseMessage"))
                && message
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty() && text != prompt)
        })
}

fn metadata(message: &Value, value: &Value) -> Value {
    json!({"microsoft_copilot": {
        "conversation_id": value.get("id"),
        "message_id": message.get("id"),
        "attributions": message.get("attributions"),
        "adaptive_cards": message.get("adaptiveCards"),
        "sensitivity_label": message.get("sensitivityLabel"),
    }})
}

impl CompletionModel for MicrosoftCompletion {
    type Response = MicrosoftResponse;
    type StreamingResponse = MicrosoftResponse;
    type Client = MicrosoftClient;

    fn make(client: &Self::Client, _model: impl Into<String>) -> Self {
        Self {
            client: client.clone(),
        }
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionResponse<Self::Response>, CompletionError> {
        let body = request_body(&request, &self.client.timezone)?;
        let conversation_url = self.client.conversation_url().await?;
        let response = self
            .client
            .http
            .post(format!("{conversation_url}/chat"))
            .bearer_auth(&self.client.access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                CompletionError::ProviderError(format!(
                    "Microsoft 365 Copilot chat request failed: {e}"
                ))
            })?;
        let value = response_json(response, "chat").await?;
        let message =
            response_message(&value, body["message"]["text"].as_str().unwrap_or_default())
                .ok_or_else(|| {
                    CompletionError::ResponseError(
                        "Microsoft 365 Copilot returned no assistant text".into(),
                    )
                })?;
        Ok(CompletionResponse {
            choice: OneOrMany::one(AssistantContent::Text(Text {
                text: message["text"].as_str().unwrap_or_default().to_owned(),
                additional_params: Some(metadata(message, &value)),
            })),
            usage: Usage::default(),
            raw_response: MicrosoftResponse {
                finish_reason: "stop".into(),
                conversation_id: value.get("id").and_then(Value::as_str).map(str::to_owned),
            },
            message_id: message.get("id").and_then(Value::as_str).map(str::to_owned),
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>
    {
        let body = request_body(&request, &self.client.timezone)?;
        let prompt = body["message"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let conversation_url = self.client.conversation_url().await?;
        let mut events = self
            .client
            .http
            .post(format!("{conversation_url}/chatOverStream"))
            .bearer_auth(&self.client.access_token)
            .json(&body)
            .eventsource()
            .map_err(|_| unsupported("could not create the streaming request"))?;
        // Reconnecting a POST would create a second model turn and could repeat side effects.
        events.set_retry_policy(Box::new(reqwest_eventsource::retry::Never));
        let stream = async_stream::try_stream! {
            let mut snapshots = HashMap::<String, String>::new();
            let mut conversation_id = None;
            let mut saw_text = false;
            let mut last_metadata = None;
            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Open) => {}
                    Ok(Event::Message(event)) => {
                        if event.data == "[DONE]" { break; }
                        if event.data.len() > MAX_EVENT_BYTES { Err(CompletionError::ResponseError("Microsoft 365 Copilot event exceeds the size limit".into()))?; }
                        let value: Value = serde_json::from_str(&event.data)?;
                        if value.get("error").is_some() { Err(CompletionError::ProviderError("Microsoft 365 Copilot reported a stream error".into()))?; }
                        conversation_id = value.get("id").and_then(Value::as_str).map(str::to_owned).or(conversation_id);
                        if let Some(message) = response_message(&value, &prompt) {
                            let id = message.get("id").and_then(Value::as_str).unwrap_or("reply").to_owned();
                            let text = message["text"].as_str().unwrap_or_default();
                            let previous = snapshots.entry(id.clone()).or_default();
                            let delta = snapshot_delta(previous, text)?;
                            if !delta.is_empty() {
                                if !saw_text { yield RawStreamingChoice::MessageId(id); }
                                yield RawStreamingChoice::Message(delta.to_owned());
                                saw_text = true;
                            }
                            if text.len() >= previous.len() { *previous = text.to_owned(); }
                            last_metadata = Some(metadata(message, &value));
                        }
                    }
                    Err(reqwest_eventsource::Error::StreamEnded) => break,
                    Err(reqwest_eventsource::Error::InvalidStatusCode(status, _)) => {
                        Err(CompletionError::ProviderError(format!("Microsoft 365 Copilot stream returned HTTP {status}")))?;
                    }
                    Err(_) => { Err(CompletionError::ProviderError("Microsoft 365 Copilot stream failed or returned malformed SSE".into()))?; }
                }
            }
            events.close();
            if !saw_text { Err(CompletionError::ResponseError("Microsoft 365 Copilot stream returned no assistant text".into()))?; }
            if let Some(metadata) = last_metadata { yield RawStreamingChoice::TextAdditionalParams(metadata); }
            yield RawStreamingChoice::FinalResponse(MicrosoftResponse { finish_reason: "stop".into(), conversation_id });
        };
        Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
    }
}

fn snapshot_delta<'a>(
    previous: &str,
    current: &'a str,
) -> std::result::Result<&'a str, CompletionError> {
    if previous.starts_with(current) {
        return Ok("");
    }
    current.strip_prefix(previous).ok_or_else(|| CompletionError::ResponseError("Microsoft 365 Copilot revised previously streamed text; rerun without streaming to receive the final response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::CompletionRequestBuilder;

    fn completion_model() -> MicrosoftCompletion {
        MicrosoftCompletion {
            client: MicrosoftClient {
                http: reqwest::Client::new(),
                access_token: "test".into(),
                timezone: "Etc/UTC".into(),
            },
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequestBuilder::new(completion_model(), "What next?").build()
    }

    #[test]
    fn maps_history_to_grounding_without_relabeling_the_last_prompt() {
        let mut request = request();
        request.preamble = Some("Be concise".into());
        request.chat_history = OneOrMany::many([
            Message::user("Earlier question"),
            Message::assistant("Earlier answer"),
            Message::user("What next?"),
        ])
        .unwrap();
        let body = request_body(&request, "Europe/Berlin").unwrap();
        assert_eq!(body["message"]["text"], "What next?");
        assert_eq!(
            body["additionalContext"][0]["text"],
            "Requested instructions:\nBe concise"
        );
        assert_eq!(
            body["additionalContext"][2]["text"],
            "Assistant:\nEarlier answer"
        );
        assert_eq!(body["locationHint"]["timeZone"], "Europe/Berlin");
    }

    #[test]
    fn refuses_tools_before_creating_a_conversation() {
        let mut request = request();
        request.tool_choice = Some(rig::message::ToolChoice::Required);
        assert!(
            request_body(&request, "Etc/UTC")
                .unwrap_err()
                .to_string()
                .contains("caller-defined tools")
        );
    }

    #[test]
    fn tool_free_auto_and_none_choices_do_not_require_tool_support() {
        for choice in [
            None,
            Some(rig::message::ToolChoice::Auto),
            Some(rig::message::ToolChoice::None),
        ] {
            let mut request = request();
            request.tool_choice = choice;
            assert!(request_body(&request, "Etc/UTC").is_ok());
        }
    }

    #[tokio::test]
    async fn ordinary_rig_agent_and_flow_history_defaults_accept_plain_chat() {
        use rig::completion::Completion;
        let model = MicrosoftModel {
            client: completion_model().client,
        };
        let history = History::new(
            MODEL_ID.into(),
            vec![crate::history::HistoryMessage::from_string(
                crate::history::Role::User,
                "What next?",
            )],
        );
        let settings = model.agent_settings(&Some(history.clone())).unwrap();
        assert!(settings.params.is_none());
        assert!(settings.temperature.is_none());
        assert!(settings.max_tokens.is_none());
        assert!(model.request_params(&history, true).unwrap().is_none());

        let agent = AgentBuilder::new(completion_model())
            .preamble("Be concise")
            .build();
        let request = agent
            .completion("What next?", Vec::<Message>::new())
            .await
            .unwrap()
            .build();
        assert!(request.tools.is_empty());
        assert!(request.tool_choice.is_none());
        assert!(request_body(&request, "Etc/UTC").is_ok());
    }

    #[test]
    fn response_ignores_prompt_echo_and_preserves_attributions() {
        let value = json!({"id":"conversation", "messages":[
            {"@odata.type":"#microsoft.graph.copilotConversationResponseMessage", "text":"Question"},
            {"@odata.type":"#microsoft.graph.copilotConversationResponseMessage", "id":"answer", "text":"Answer", "attributions":[{"seeMoreWebUrl":"https://example.test"}]}
        ]});
        let message = response_message(&value, "Question").unwrap();
        assert_eq!(message["text"], "Answer");
        assert_eq!(
            metadata(message, &value)["microsoft_copilot"]["attributions"][0]["seeMoreWebUrl"],
            "https://example.test"
        );
    }

    #[test]
    fn snapshot_deltas_do_not_duplicate_replayed_text() {
        assert_eq!(snapshot_delta("Hi", "Hi there").unwrap(), " there");
        assert_eq!(snapshot_delta("Hi there", "Hi").unwrap(), "");
        assert!(snapshot_delta("Hi", "Hello").is_err());
        assert_eq!(snapshot_delta("Grü", "Grüße").unwrap(), "ße");
    }
}
