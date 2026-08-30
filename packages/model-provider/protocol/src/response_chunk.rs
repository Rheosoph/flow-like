use super::response::{LogProbs, Usage};
use crate::history::Content;
use rig::completion::Usage as RigUsage;
use rig::message::ToolCall as RigMessageToolCall;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json as json;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct ResponseChunk {
    pub id: String,
    pub choices: Vec<ResponseChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_prefill_progress: Option<f32>,
}

impl ResponseChunk {
    pub fn get_streamed_token(&self) -> Option<String> {
        let choice = self.choices.first()?;
        let delta = choice.delta.as_ref()?;
        delta.content.clone()
    }

    /// Gets the content delta from the first choice
    pub fn content_delta(&self) -> Option<String> {
        self.get_streamed_token()
    }

    /// Gets the role from the first choice delta
    pub fn role(&self) -> Option<String> {
        self.choices
            .first()
            .and_then(|c| c.delta.as_ref())
            .and_then(|d| d.role.clone())
    }

    /// Checks if this chunk contains a tool call delta
    pub fn has_tool_call(&self) -> bool {
        self.choices
            .first()
            .and_then(|c| c.delta.as_ref())
            .and_then(|d| d.tool_calls.as_ref())
            .map(|tc| !tc.is_empty())
            .unwrap_or(false)
    }

    /// Gets tool calls from the first choice delta
    pub fn tool_calls(&self) -> Option<Vec<DeltaFunctionCall>> {
        self.choices
            .first()
            .and_then(|c| c.delta.as_ref())
            .and_then(|d| d.tool_calls.clone())
    }

    /// Checks if the response is finished
    pub fn is_finished(&self) -> bool {
        self.choices
            .first()
            .and_then(|c| c.finish_reason.as_ref())
            .is_some()
    }

    /// Gets the finish reason
    pub fn finish_reason(&self) -> Option<String> {
        self.choices.first().and_then(|c| c.finish_reason.clone())
    }

    /// Creates a chunk with text content
    pub fn from_text(text: &str, model_name: &str) -> Self {
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: Some(Delta {
                    role: Some("assistant".to_string()),
                    content: Some(text.to_string()),
                    content_parts: None,
                    tool_calls: None,
                    refusal: None,
                    reasoning: None,
                }),
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        }
    }

    /// Creates a chunk from a tool call
    pub fn from_tool_call(tool_call: &RigMessageToolCall, model_name: &str) -> Self {
        let arguments = json::to_string(&tool_call.function.arguments).unwrap_or_default();
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: Some(Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    content_parts: None,
                    tool_calls: Some(vec![DeltaFunctionCall {
                        index: None,
                        id: Some(tool_call.id.clone()),
                        tool_type: Some("function".to_string()),
                        function: DeltaResponseFunction {
                            name: Some(tool_call.function.name.clone()),
                            arguments: Some(arguments),
                        },
                    }]),
                    refusal: None,
                    reasoning: None,
                }),
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        }
    }

    /// Creates a chunk from a tool call delta
    pub fn from_tool_call_delta(id: &str, delta: &str, model_name: &str) -> Self {
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: Some(Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    content_parts: None,
                    tool_calls: Some(vec![DeltaFunctionCall {
                        index: None,
                        id: Some(id.to_string()),
                        tool_type: Some("function".to_string()),
                        function: DeltaResponseFunction {
                            name: None,
                            arguments: Some(delta.to_string()),
                        },
                    }]),
                    refusal: None,
                    reasoning: None,
                }),
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        }
    }

    /// Creates a chunk with reasoning content
    pub fn from_reasoning(reasoning: &str, model_name: &str) -> Self {
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: Some(Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    content_parts: None,
                    tool_calls: None,
                    refusal: None,
                    reasoning: Some(reasoning.to_string()),
                }),
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        }
    }

    /// Creates a synthetic non-stream chunk for structured media returned by Rig.
    pub fn from_content_part(content: Content, model_name: &str) -> Self {
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: Some(Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    content_parts: Some(vec![content]),
                    tool_calls: None,
                    refusal: None,
                    reasoning: None,
                }),
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        }
    }

    /// Creates a finish chunk with optional usage
    pub fn finish(model_name: &str, usage: Option<&RigUsage>) -> Self {
        Self {
            model: Some(model_name.to_string()),
            choices: vec![ResponseChunkChoice {
                index: 0,
                delta: None,
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
            usage: usage.map(|usage| Usage::from_rig(*usage)),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ResponseChunkChoice {
    pub index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<Delta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<LogProbs>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_parts: Option<Vec<Content>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<DeltaFunctionCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct DeltaFunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    pub function: DeltaResponseFunction,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct DeltaResponseFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}
