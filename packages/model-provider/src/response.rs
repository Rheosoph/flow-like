use super::response_chunk::{Delta, DeltaFunctionCall, ResponseChunk};
use flow_like_types::{
    JsonSchema, Result,
    json::{self, Deserialize, Serialize},
};
use rig::OneOrMany;
use rig::completion::{Message as RigMessage, Usage as RigUsage};
use rig::message::{
    AssistantContent as RigAssistantContent, Text as RigText, ToolCall as RigToolCall,
    ToolFunction as RigToolFunction,
};

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
    pub fn apply_delta(&mut self, delta: Delta) {
        if let Some(content) = delta.content {
            self.content = Some(self.content.as_deref().unwrap_or("").to_string() + &content);
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
    type Error = flow_like_types::Error;

    fn try_from(msg: ResponseMessage) -> Result<Self> {
        let mut rig_contents = Vec::new();

        if let Some(content) = msg.content
            && !content.is_empty()
        {
            rig_contents.push(RigAssistantContent::Text(RigText { text: content }));
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
            }))
        } else if rig_contents.len() == 1 {
            OneOrMany::one(rig_contents.into_iter().next().unwrap())
        } else {
            OneOrMany::many(rig_contents).map_err(|e| flow_like_types::Error::msg(e.to_string()))?
        };

        Ok(RigMessage::Assistant { id: None, content })
    }
}

impl TryFrom<RigMessage> for ResponseMessage {
    type Error = flow_like_types::Error;

    fn try_from(msg: RigMessage) -> Result<Self> {
        match msg {
            RigMessage::Assistant { id: _, content } => {
                let mut text_content = String::new();
                let mut tool_calls = Vec::new();

                for item in content.iter() {
                    match item {
                        RigAssistantContent::Text(text) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(&text.text);
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
                        RigAssistantContent::Reasoning(_) | RigAssistantContent::Image(_) => {}
                    }
                }

                Ok(ResponseMessage {
                    role: "assistant".to_string(),
                    content: if text_content.is_empty() {
                        None
                    } else {
                        Some(text_content)
                    },
                    refusal: None,
                    annotations: None,
                    audio: None,
                    reasoning: None,
                    tool_calls,
                })
            }
            _ => Err(flow_like_types::Error::msg(
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
            .ok_or_else(|| flow_like_types::Error::msg("No message in response"))?
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
        if let Some(last) = self.calls.last_mut() {
            if last.duration_ms.is_none() {
                last.duration_ms = Some(duration_ms);
            }
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
    use flow_like_types::json;

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
}
