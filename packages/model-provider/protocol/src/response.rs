use super::response_chunk::{Delta, DeltaFunctionCall, ResponseChunk};
use crate::history::{Content, ContentType};
use anyhow::Result;
use rig::OneOrMany;
use rig::completion::{Message as RigMessage, Usage as RigUsage};
use rig::message::{
    AssistantContent as RigAssistantContent, Reasoning as RigReasoning, Text as RigText,
    ToolCall as RigToolCall, ToolFunction as RigToolFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json as json;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    pub id: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    pub function: ResponseFunction,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ResponseFunction {
    //#[serde(skip_serializing_if = "Option::is_none")]
    pub name: String,
    //#[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LogProbs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<TokenLogProbs>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Vec<TokenLogProbs>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct TokenLogProbs {
    pub token: String,
    pub logprob: f64,
    pub bytes: Option<Vec<u8>>,
    pub top_logprobs: Option<Vec<TopLogProbs>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct TopLogProbs {
    pub token: String,
    pub logprob: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Choice {
    pub index: i32,
    pub finish_reason: String,
    pub message: ResponseMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogProbs>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Audio {
    pub data: String,
    pub expires_at: Option<u64>,
    pub id: String,
    pub transcript: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ResponseMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Ordered structured content returned by Rig when the assistant response contains media.
    /// Text-only responses continue to use `content` alone for OpenAI compatibility.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<Audio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,

    //#[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Vec<FunctionCall>,
}

impl Default for ResponseMessage {
    fn default() -> Self {
        ResponseMessage {
            content: None,
            content_parts: vec![],
            refusal: None,
            annotations: None,
            audio: None,
            reasoning: None,
            tool_calls: vec![],
            role: "".to_string(),
        }
    }
}

impl ResponseMessage {
    /// Returns ordered structured content, adding the legacy `content` string when a mixed-media
    /// response supplied only media in `content_parts`.
    pub fn ordered_content_parts(&self) -> Vec<Content> {
        let mut parts = self.content_parts.clone();
        let content = self
            .content
            .as_deref()
            .filter(|content| !content.is_empty());

        if parts.is_empty() {
            return content
                .map(|text| {
                    vec![Content::Text {
                        content_type: ContentType::Text,
                        text: text.to_string(),
                    }]
                })
                .unwrap_or_default();
        }

        if !parts
            .iter()
            .any(|part| matches!(part, Content::Text { .. }))
            && let Some(text) = content
        {
            parts.insert(
                0,
                Content::Text {
                    content_type: ContentType::Text,
                    text: text.to_string(),
                },
            );
        }

        parts
    }

    pub fn apply_delta(&mut self, delta: Delta) {
        let has_text_delta = delta.content.is_some();
        let structured_parts_contain_text = delta.content_parts.as_ref().is_some_and(|parts| {
            parts
                .iter()
                .any(|part| matches!(part, Content::Text { .. }))
        });
        if let Some(content) = delta.content {
            if !self.content_parts.is_empty() && !structured_parts_contain_text {
                match self.content_parts.last_mut() {
                    Some(Content::Text { text, .. }) => text.push_str(&content),
                    _ => self.content_parts.push(Content::Text {
                        content_type: ContentType::Text,
                        text: content.clone(),
                    }),
                }
            }
            self.content = Some(self.content.as_deref().unwrap_or("").to_string() + &content);
        }

        if let Some(content_parts) = delta.content_parts {
            if self.content_parts.is_empty()
                && self.content.as_ref().is_some_and(|text| !text.is_empty())
                && !structured_parts_contain_text
            {
                self.content_parts.push(Content::Text {
                    content_type: ContentType::Text,
                    text: self.content.clone().unwrap_or_default(),
                });
            }
            if !has_text_delta {
                for part in &content_parts {
                    if let Content::Text { text, .. } = part {
                        self.content =
                            Some(self.content.as_deref().unwrap_or("").to_string() + text.as_str());
                    }
                }
            }
            self.content_parts.extend(content_parts);
        }

        if let Some(refusal) = delta.refusal {
            self.refusal = Some(self.refusal.as_deref().unwrap_or("").to_string() + &refusal);
        }

        if let Some(reasoning) = delta.reasoning {
            self.reasoning = Some(self.reasoning.as_deref().unwrap_or("").to_string() + &reasoning);
        }

        if let Some(role) = delta.role
            && role != self.role
        {
            self.role = self.role.to_string() + &role;
        }

        if let Some(tool_calls) = delta.tool_calls {
            for dcall in tool_calls.into_iter() {
                self.apply_delta_tool_call(dcall);
            }
        }
    }

    fn apply_delta_tool_call(&mut self, dcall: DeltaFunctionCall) {
        // Determine index (default to next position if missing)
        let idx = dcall.index;

        // Try to find existing entry by index when provided
        if let Some(i) = idx
            && let Some(existing) = self.tool_calls.iter_mut().find(|c| c.index == Some(i))
        {
            if let Some(id) = dcall.id {
                existing.id = id;
            }
            if let Some(t) = dcall.tool_type {
                existing.tool_type =
                    Some(existing.tool_type.as_deref().unwrap_or("").to_string() + &t);
            }
            if let Some(name) = dcall.function.name {
                existing.function.name += &name;
            }
            if let Some(args) = dcall.function.arguments {
                existing.function.arguments += &args;
            }
            return;
        }

        // Create new entry, using empty strings for missing fields
        let index = idx;
        let id = dcall.id.unwrap_or_default();
        let tool_type = dcall.tool_type;
        let name = dcall.function.name.unwrap_or_default();
        let arguments = dcall.function.arguments.unwrap_or_default();
        self.tool_calls.push(FunctionCall {
            index,
            id,
            tool_type,
            function: ResponseFunction { name, arguments },
        });
    }
}

impl TryFrom<ResponseMessage> for RigMessage {
    type Error = anyhow::Error;

    fn try_from(msg: ResponseMessage) -> Result<Self> {
        let mut rig_contents = Vec::new();

        if msg.content_parts.is_empty() {
            if let Some(content) = msg.content
                && !content.is_empty()
            {
                rig_contents.push(RigAssistantContent::Text(RigText {
                    text: content,
                    additional_params: None,
                }));
            }
        } else {
            let parts_contain_text = msg
                .content_parts
                .iter()
                .any(|part| matches!(part, Content::Text { .. }));
            if !parts_contain_text
                && let Some(content) = msg.content
                && !content.is_empty()
            {
                rig_contents.push(RigAssistantContent::Text(RigText {
                    text: content,
                    additional_params: None,
                }));
            }
            for part in msg.content_parts {
                rig_contents.push(part.try_into_rig_assistant()?);
            }
        }

        if let Some(reasoning) = msg.reasoning
            && !reasoning.is_empty()
        {
            rig_contents.push(RigAssistantContent::Reasoning(RigReasoning::new(
                &reasoning,
            )));
        }

        for tool_call in msg.tool_calls {
            rig_contents.push(RigAssistantContent::ToolCall(RigToolCall {
                id: tool_call.id,
                call_id: None,
                function: RigToolFunction {
                    name: tool_call.function.name,
                    arguments: json::from_str(&tool_call.function.arguments)
                        .unwrap_or(json::json!({})),
                },
                signature: None,
                additional_params: None,
            }));
        }

        let content = if rig_contents.is_empty() {
            OneOrMany::one(RigAssistantContent::Text(RigText {
                text: String::new(),
                additional_params: None,
            }))
        } else if rig_contents.len() == 1 {
            OneOrMany::one(rig_contents.into_iter().next().unwrap())
        } else {
            OneOrMany::many(rig_contents).map_err(|e| anyhow::Error::msg(e.to_string()))?
        };

        Ok(RigMessage::Assistant { id: None, content })
    }
}

impl TryFrom<RigMessage> for ResponseMessage {
    type Error = anyhow::Error;

    fn try_from(msg: RigMessage) -> Result<Self> {
        match msg {
            RigMessage::Assistant { id: _, content } => {
                let mut text_content = String::new();
                let mut content_parts = Vec::new();
                let mut tool_calls = Vec::new();
                let mut reasoning_content = String::new();
                let mut has_media = false;

                for item in content.iter() {
                    match item {
                        RigAssistantContent::Text(text) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(&text.text);
                            content_parts.push(Content::Text {
                                content_type: ContentType::Text,
                                text: text.text.clone(),
                            });
                        }
                        RigAssistantContent::ToolCall(tool_call) => {
                            tool_calls.push(FunctionCall {
                                index: None,
                                id: tool_call.id.clone(),
                                tool_type: Some("function".to_string()),
                                function: ResponseFunction {
                                    name: tool_call.function.name.clone(),
                                    arguments: tool_call.function.arguments.to_string(),
                                },
                            });
                        }
                        RigAssistantContent::Reasoning(reasoning) => {
                            let display = reasoning.display_text();
                            if !display.is_empty() {
                                if !reasoning_content.is_empty() {
                                    reasoning_content.push('\n');
                                }
                                reasoning_content.push_str(&display);
                            }
                        }
                        RigAssistantContent::Image(image) => {
                            has_media = true;
                            content_parts.push(Content::from_rig_image(image.clone()));
                        }
                    }
                }

                Ok(ResponseMessage {
                    role: "assistant".to_string(),
                    content: if text_content.is_empty() {
                        None
                    } else {
                        Some(text_content)
                    },
                    content_parts: if has_media { content_parts } else { Vec::new() },
                    refusal: None,
                    annotations: None,
                    audio: None,
                    reasoning: if reasoning_content.is_empty() {
                        None
                    } else {
                        Some(reasoning_content)
                    },
                    tool_calls,
                })
            }
            _ => Err(anyhow::Error::msg(
                "Can only convert Assistant messages to ResponseMessage",
            )),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct Usage {
    pub completion_tokens: u32,
    pub prompt_tokens: u32,
    pub total_tokens: u32,
    pub cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokenDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokenDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_inference_cost: Option<CostDetails>,
}

impl Usage {
    /// Converts from Rig's Usage type
    pub fn from_rig(usage: RigUsage) -> Self {
        Self {
            prompt_tokens: Self::safe_downcast(usage.input_tokens),
            completion_tokens: Self::safe_downcast(usage.output_tokens),
            total_tokens: Self::safe_downcast(usage.total_tokens),
            cost: None,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            upstream_inference_cost: None,
        }
    }

    fn safe_downcast(value: u64) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct CostDetails {
    upstream_inference_cost: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct PromptTokenDetails {
    cached_tokens: Option<u32>,
    audio_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct CompletionTokenDetails {
    accepted_prediction_tokens: Option<u32>,
    audio_tokens: Option<u32>,
    reasoning_tokens: Option<u32>,
    rejected_prediction_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Annotation {
    r#type: String,
    url_citation: Option<UrlCitation>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct UrlCitation {
    end_index: u32,
    start_index: u32,
    title: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub usage: Usage,
}

impl Response {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a Response from a simple text content
    pub fn from_text(text: impl Into<String>, model: impl Into<String>) -> Self {
        Response {
            id: None,
            choices: vec![Choice {
                index: 0,
                finish_reason: "stop".to_string(),
                message: ResponseMessage {
                    role: "assistant".to_string(),
                    content: Some(text.into()),
                    content_parts: vec![],
                    refusal: None,
                    annotations: None,
                    audio: None,
                    reasoning: None,
                    tool_calls: vec![],
                },
                logprobs: None,
            }],
            created: None,
            model: Some(model.into()),
            service_tier: None,
            system_fingerprint: None,
            object: None,
            usage: Usage::default(),
        }
    }

    pub fn last_message(&self) -> Option<&ResponseMessage> {
        self.choices.last().map(|c| &c.message)
    }

    /// Gets the text content from the first choice
    pub fn content(&self) -> Option<String> {
        self.choices.first().and_then(|c| c.message.content.clone())
    }

    /// Converts to rig message (from the first choice)
    pub fn to_rig_message(&self) -> Result<RigMessage> {
        self.last_message()
            .ok_or_else(|| anyhow::Error::msg("No message in response"))?
            .clone()
            .try_into()
    }

    /// Creates Response from rig assistant message
    pub fn from_rig_message(msg: RigMessage) -> Result<Self> {
        let response_msg: ResponseMessage = msg.try_into()?;

        Ok(Response {
            id: None,
            choices: vec![Choice {
                index: 0,
                finish_reason: "stop".to_string(),
                message: response_msg,
                logprobs: None,
            }],
            created: None,
            model: None,
            service_tier: None,
            system_fingerprint: None,
            object: None,
            usage: Usage::default(),
        })
    }

    pub fn push_chunk(&mut self, chunk: ResponseChunk) {
        // Update optional fields if present in the chunk
        if let Some(created) = chunk.created {
            self.created = Some(created);
        }

        if let Some(model) = chunk.model {
            self.model = Some(model);
        }

        if let Some(service_tier) = chunk.service_tier {
            self.service_tier = Some(service_tier);
        }

        if let Some(system_fingerprint) = chunk.system_fingerprint {
            self.system_fingerprint = Some(system_fingerprint);
        }

        if let Some(usage) = chunk.usage {
            self.usage.completion_tokens += usage.completion_tokens;
            self.usage.prompt_tokens += usage.prompt_tokens;
            self.usage.total_tokens += usage.total_tokens;
        }

        for choice in chunk.choices {
            // Check if a choice with the same index already exists
            if let Some(existing_choice) = self.choices.iter_mut().find(|c| c.index == choice.index)
            {
                // Update existing choice fields if present
                if let Some(delta) = choice.delta {
                    existing_choice.message.apply_delta(delta);
                }
                if let Some(logprobs) = choice.logprobs {
                    existing_choice.logprobs = Some(logprobs);
                }
                if let Some(finish_reason) = choice.finish_reason {
                    existing_choice.finish_reason = finish_reason;
                }

                continue; // Continue to next choice, don't return
            }

            // Create a new choice if it doesn't exist
            let mut message = ResponseMessage::default();
            if let Some(delta) = choice.delta {
                message.apply_delta(delta);
            }

            self.choices.push(Choice {
                finish_reason: choice.finish_reason.unwrap_or_default(),
                index: choice.index,
                logprobs: choice.logprobs,
                message,
            });
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct ModelCallEntry {
    pub model: String,
    pub usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct LLMUsageStats {
    pub usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<ModelCallEntry>,
}

impl LLMUsageStats {
    pub fn from_response(response: &Response) -> Self {
        let call = ModelCallEntry {
            model: response.model.clone().unwrap_or_default(),
            usage: response.usage.clone(),
            duration_ms: None,
        };
        Self {
            usage: response.usage.clone(),
            model: response.model.clone(),
            duration_ms: None,
            iterations: None,
            calls: vec![call],
        }
    }

    pub fn set_duration_ms(&mut self, duration_ms: u64) {
        self.duration_ms = Some(duration_ms);
        if let Some(last) = self.calls.last_mut()
            && last.duration_ms.is_none()
        {
            last.duration_ms = Some(duration_ms);
        }
    }

    pub fn set_iterations(&mut self, iterations: u32) {
        self.iterations = Some(iterations);
    }

    pub fn accumulate(&mut self, other: &Usage, model: Option<&str>) {
        self.usage.prompt_tokens = self.usage.prompt_tokens.saturating_add(other.prompt_tokens);
        self.usage.completion_tokens = self
            .usage
            .completion_tokens
            .saturating_add(other.completion_tokens);
        self.usage.total_tokens = self.usage.total_tokens.saturating_add(other.total_tokens);
        if let Some(other_cost) = other.cost {
            self.usage.cost = Some(self.usage.cost.unwrap_or(0.0) + other_cost);
        }
        self.calls.push(ModelCallEntry {
            model: model.unwrap_or_default().to_string(),
            usage: other.clone(),
            duration_ms: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{DocumentSourceKind, Image, ImageDetail, ImageMediaType, Reasoning};
    use serde_json as json;

    #[test]
    fn deserialize_annotations_with_content() {
        let json_str = r#"{
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "Here's the latest news I found: ...",
                    "annotations": [
                        {
                            "type": "url_citation",
                            "url_citation": {
                                "url": "https://www.example.com/web-search-result",
                                "title": "Title of the web search result",
                                "content": "Content of the web search result",
                                "start_index": 100,
                                "end_index": 200
                            }
                        }
                    ],
                    "tool_calls": []
                }
            }],
            "usage": {"completion_tokens":0, "prompt_tokens":0, "total_tokens":0}
        }"#;

        let resp: Response = json::from_str(json_str).expect("valid response json");
        let anns = resp
            .choices
            .first()
            .and_then(|c| c.message.annotations.as_ref())
            .expect("annotations present");
        assert_eq!(anns.len(), 1);

        // Ensure it deserializes rather than panics; structure fields are private by design.
        // We just check presence by re-serializing.
        let out = json::to_string(&resp).unwrap();
        assert!(out.contains("url_citation"));
        assert!(out.contains("content"));
    }

    #[test]
    fn rig_assistant_media_and_reasoning_are_preserved() {
        let content = OneOrMany::many(vec![
            RigAssistantContent::Reasoning(Reasoning::new("working")),
            RigAssistantContent::Text(RigText::new("caption")),
            RigAssistantContent::Image(Image {
                data: DocumentSourceKind::Base64("aGVsbG8=".to_string()),
                media_type: Some(ImageMediaType::PNG),
                detail: Some(ImageDetail::High),
                additional_params: None,
            }),
        ])
        .expect("multiple assistant parts");

        let response = Response::from_rig_message(RigMessage::Assistant {
            id: Some("message-1".to_string()),
            content,
        })
        .expect("valid assistant response");
        let message = response.last_message().expect("response message");

        assert_eq!(message.content.as_deref(), Some("caption"));
        assert_eq!(message.reasoning.as_deref(), Some("working"));
        assert_eq!(message.content_parts.len(), 2);
        assert!(matches!(
            &message.content_parts[1],
            Content::Image { image_url, .. }
                if image_url.url == "data:image/png;base64,aGVsbG8="
                    && image_url.detail.as_deref() == Some("high")
        ));

        let round_trip = response.to_rig_message().expect("Rig round trip");
        let RigMessage::Assistant { content, .. } = round_trip else {
            panic!("expected assistant message")
        };
        assert!(content.iter().any(|part| matches!(
            part,
            RigAssistantContent::Image(Image {
                data: DocumentSourceKind::Base64(data),
                media_type: Some(ImageMediaType::PNG),
                ..
            }) if data == "aGVsbG8="
        )));
        assert!(
            content
                .iter()
                .any(|part| matches!(part, RigAssistantContent::Reasoning(_)))
        );
    }

    #[test]
    fn media_delta_keeps_preceding_text_as_an_ordered_part() {
        let mut message = ResponseMessage::default();
        message.apply_delta(Delta {
            role: Some("assistant".to_string()),
            content: Some("caption".to_string()),
            content_parts: None,
            tool_calls: None,
            refusal: None,
            reasoning: None,
        });
        message.apply_delta(Delta {
            role: None,
            content: None,
            content_parts: Some(vec![Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: crate::history::ImageUrl {
                    url: "https://example.com/generated.png".to_string(),
                    detail: None,
                    media_type: Some("image/png".to_string()),
                    additional_params: None,
                },
            }]),
            tool_calls: None,
            refusal: None,
            reasoning: None,
        });

        assert_eq!(message.content.as_deref(), Some("caption"));
        assert_eq!(message.content_parts.len(), 2);
        assert!(matches!(
            &message.content_parts[0],
            Content::Text { text, .. } if text == "caption"
        ));
    }

    #[test]
    fn text_delta_after_media_remains_an_ordered_part() {
        let mut message = ResponseMessage::default();
        message.apply_delta(Delta {
            role: Some("assistant".to_string()),
            content: None,
            content_parts: Some(vec![Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: crate::history::ImageUrl {
                    url: "https://example.com/generated.png".to_string(),
                    detail: None,
                    media_type: Some("image/png".to_string()),
                    additional_params: None,
                },
            }]),
            tool_calls: None,
            refusal: None,
            reasoning: None,
        });
        message.apply_delta(Delta {
            role: None,
            content: Some("caption".to_string()),
            content_parts: None,
            tool_calls: None,
            refusal: None,
            reasoning: None,
        });

        assert_eq!(message.content.as_deref(), Some("caption"));
        assert!(matches!(
            &message.content_parts[1],
            Content::Text { text, .. } if text == "caption"
        ));
    }

    #[test]
    fn legacy_text_is_prepended_to_media_only_parts() {
        let message = ResponseMessage {
            role: "assistant".to_string(),
            content: Some("caption".to_string()),
            content_parts: vec![Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: crate::history::ImageUrl {
                    url: "https://example.com/generated.png".to_string(),
                    detail: None,
                    media_type: Some("image/png".to_string()),
                    additional_params: None,
                },
            }],
            ..ResponseMessage::default()
        };

        let parts = message.ordered_content_parts();
        assert_eq!(parts.len(), 2);
        assert!(matches!(
            &parts[0],
            Content::Text { text, .. } if text == "caption"
        ));
    }
}
