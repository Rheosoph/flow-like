//! WASI-safe agent helpers for FlowLike WASM components.
//!
//! This module intentionally avoids depending on `rig-core` on `wasm32`.
//! The browser-WASM pieces pulled by Rig's HTTP stack do not link cleanly
//! into `wasm32-wasip2` components, while FlowLike components already call
//! the host model API through WIT.

use crate::interop::{Bit, ChatContent, ChatMessage, ToolCallData};
use crate::Context;

/// A host-backed completion model descriptor used by [`WasiAgent`].
#[derive(Clone)]
pub struct FlowLikeCompletionModel {
    pub(crate) bit: Bit,
}

impl FlowLikeCompletionModel {
    /// Wrap a FlowLike model `Bit`.
    ///
    /// The `Context` parameter keeps the constructor compatible with the
    /// native Rig-backed provider. WASI host calls are imported functions, so
    /// the value does not need to be stored.
    pub fn new(bit: Bit, _ctx: &Context) -> Self {
        Self { bit }
    }
}

/// Tool metadata understood by the FlowLike host LLM bridge.
///
/// This is also exported as `flow_like_wasm_sdk::rig::completion::ToolDefinition`
/// on `wasm32` so existing WASI agent code can keep using the Rig-shaped path
/// without pulling `rig-core` into the component build.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
}

struct WasiToolEntry {
    definition: ToolDefinition,
    call: Box<dyn Fn(serde_json::Value) -> Result<String, String> + Send + Sync>,
}

/// A lightweight synchronous agent loop for WASI components.
///
/// It sends messages to the FlowLike host model API, runs requested tools in
/// the component, appends tool results, and repeats until the model returns a
/// final text response.
pub struct WasiAgent {
    model: FlowLikeCompletionModel,
    preamble: Option<String>,
    tools: Vec<WasiToolEntry>,
    max_steps: usize,
}

impl WasiAgent {
    pub fn new(model: FlowLikeCompletionModel) -> Self {
        Self {
            model,
            preamble: None,
            tools: Vec::new(),
            max_steps: 10,
        }
    }

    pub fn preamble(mut self, preamble: impl Into<String>) -> Self {
        self.preamble = Some(preamble.into());
        self
    }

    pub fn max_steps(mut self, n: usize) -> Self {
        self.max_steps = n;
        self
    }

    /// Register a synchronous tool callback.
    pub fn tool(
        mut self,
        definition: crate::rig::completion::ToolDefinition,
        call: impl Fn(serde_json::Value) -> Result<String, String> + Send + Sync + 'static,
    ) -> Self {
        self.tools.push(WasiToolEntry {
            definition,
            call: Box::new(call),
        });
        self
    }

    /// Run the agent loop until the host model returns a final response.
    pub fn prompt(&self, user_message: &str) -> Result<String, String> {
        let mut messages: Vec<ChatMessage> = Vec::new();
        if let Some(ref p) = self.preamble {
            messages.push(ChatMessage::system(p.clone()));
        }
        messages.push(ChatMessage::user(user_message.to_string()));

        for step in 0..self.max_steps {
            crate::host::debug(&format!("WasiAgent: step {step}"));

            let json = self.serialize_request(&messages)?;
            let bit_json = serde_json::to_string(&self.model.bit)
                .map_err(|e| format!("Bit serialize: {e}"))?;

            let resp_str = crate::host::llm_prompt(&bit_json, &json, false)
                .ok_or("Host LLM prompt returned None (MODELS capability may not be granted)")?;

            if let Ok(err_obj) = serde_json::from_str::<serde_json::Value>(&resp_str) {
                if let Some(err_msg) = err_obj.get("error").and_then(|v| v.as_str()) {
                    return Err(err_msg.to_string());
                }
            }

            let resp: HostResp = serde_json::from_str(&resp_str)
                .map_err(|e| format!("Failed to parse host response: {e}"))?;

            let tool_calls = resp.tool_calls.unwrap_or_default();
            if tool_calls.is_empty() {
                return Ok(resp.content.unwrap_or_default());
            }

            let tc_data: Vec<ToolCallData> = tool_calls
                .iter()
                .map(|tc| ToolCallData {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect();

            messages.push(ChatMessage {
                role: "assistant".into(),
                content: ChatContent::Text {
                    content: resp.content.clone().unwrap_or_default(),
                },
                tool_calls: Some(tc_data),
                tool_call_id: None,
            });

            for tc in &tool_calls {
                let result =
                    if let Some(entry) = self.tools.iter().find(|t| t.definition.name == tc.name) {
                        match (entry.call)(tc.arguments.clone()) {
                            Ok(r) => r,
                            Err(e) => format!("Tool error: {e}"),
                        }
                    } else {
                        format!("Unknown tool: {}", tc.name)
                    };

                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: ChatContent::Text { content: result },
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                });
            }
        }

        Err(format!("Agent exceeded max steps ({})", self.max_steps))
    }

    fn serialize_request(&self, messages: &[ChatMessage]) -> Result<String, String> {
        if self.tools.is_empty() {
            return serde_json::to_string(messages)
                .map_err(|e| format!("Failed to serialize messages: {e}"));
        }

        let tool_defs: Vec<serde_json::Value> = self
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.definition.name,
                    "description": t.definition.description,
                    "parameters": t.definition.parameters,
                })
            })
            .collect();

        serde_json::to_string(&serde_json::json!({
            "messages": messages,
            "tools": tool_defs,
        }))
        .map_err(|e| format!("Failed to serialize messages: {e}"))
    }
}

#[derive(serde::Deserialize)]
struct HostResp {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<HostToolCall>>,
}

#[derive(serde::Deserialize, Clone)]
struct HostToolCall {
    id: String,
    name: String,
    arguments: serde_json::Value,
}
