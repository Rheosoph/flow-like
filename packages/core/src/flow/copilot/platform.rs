//! Platform assistant runner — makes profile ("Bits") models tool-capable for the global FlowPilot
//! assistant, reusing the exact machinery the board copilot uses for Bits: resolve the profile model
//! into a rig completion client, attach the shared platform tool specs to each completion request,
//! and run the manual tool-call loop, streaming the same tagged-frame protocol (`<tool_start>`,
//! `<tool_end>`, `<plan_step>`) that the shared frontend parser renders for every backend.
//!
//! The platform tools act on the host app (navigate, create app, delegate to the board copilot, ask
//! the user), which is desktop-only, so core defines only the tool *specs* (see `tool_spec`) + a
//! `PlatformToolBridge` trait; the concrete execution + approval lives in the desktop crate.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_model_provider::llm::CompletionClientDyn;
use flow_like_model_provider::provider::ModelProvider;
use flow_like_model_provider::response::{LLMUsageStats, Usage};
use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::{Completion, GetTokenUsage},
    message::{
        AssistantContent, DocumentSourceKind, Image, ImageDetail, ImageMediaType,
        ToolResult as RigToolResult, ToolResultContent, UserContent,
    },
    streaming::StreamedAssistantContent,
};
use serde_json::{Value, json};

use super::memory::AssistantMemory;
use super::stream::{plan_step_frame, tool_end_frame, tool_start_frame, usage_stat_frame};
use super::tool_spec::{MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, global_assistant_tool_specs};
use super::types::{ChatImage, ChatMessage, ChatRole, PlanStepStatus};
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters};
use crate::profile::Profile;
use crate::state::FlowLikeState;

/// Executes the global assistant's platform tools. Implemented by the desktop crate over the
/// FrontendToolBridge (with per-tool approval); returns the tool result as a string for the model.
#[async_trait]
pub trait PlatformToolBridge: Send + Sync {
    async fn call(&self, tool_name: &str, arguments: Value) -> String;
}

/// Global platform assistant. Holds just the state + profile needed to resolve a model; the tools and
/// prompt are supplied per call so the desktop keeps ownership of the self-awareness context.
pub struct PlatformCopilot {
    state: Arc<FlowLikeState>,
    profile: Option<Arc<Profile>>,
}

impl PlatformCopilot {
    pub fn new(state: Arc<FlowLikeState>, profile: Option<Arc<Profile>>) -> Self {
        Self { state, profile }
    }

    async fn resolve_model<'a>(
        &self,
        model_id: Option<String>,
        token: Option<String>,
    ) -> Result<(String, Box<dyn CompletionClientDyn + Send + Sync + 'a>)> {
        let bit = if let Some(profile) = &self.profile {
            if let Some(id) = model_id {
                profile
                    .find_bit(&id, self.state.http_client.clone())
                    .await?
            } else {
                let preference = BitModelPreference {
                    reasoning_weight: Some(1.0),
                    ..Default::default()
                };
                profile
                    .get_best_model(&preference, false, true, self.state.http_client.clone())
                    .await?
            }
        } else {
            Bit {
                id: "gpt-4o".to_string(),
                bit_type: BitTypes::Llm,
                parameters: serde_json::to_value(LLMParameters {
                    context_length: 128000,
                    provider: ModelProvider {
                        provider_name: "openai".to_string(),
                        model_id: None,
                        version: None,
                        params: None,
                    },
                    model_classification: Default::default(),
                })
                .unwrap(),
                ..Default::default()
            }
        };

        let model_factory = self.state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(&bit, self.state.clone(), token, None)
            .await?;
        let default_model = model.default_model().await.unwrap_or("gpt-4o".to_string());
        let provider = model.provider().await?;
        Ok((default_model, provider.into_client()))
    }

    /// Run the platform assistant loop for a profile model, dispatching platform tools through the
    /// supplied bridge and streaming tagged frames via `on_token`. Returns the final assistant text.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat<F>(
        &self,
        mut system_prompt: String,
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<ChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        bridge: Arc<dyn PlatformToolBridge>,
        memory: Option<Arc<AssistantMemory>>,
        on_token: Option<F>,
    ) -> Result<String>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let (model_name, completion_client) = self.resolve_model(model_id, token).await?;

        // Memory: recall relevant context into the prompt + advertise the memory tools.
        if let Some(memory) = &memory {
            system_prompt.push_str(&memory.prompt_sections(&user_prompt).await);
        }

        let tool_definitions: Vec<_> = global_assistant_tool_specs(memory.is_some())
            .iter()
            .map(|spec| spec.to_tool_definition())
            .collect();

        let agent = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .build();

        let parse_media_type = |s: &str| -> Option<ImageMediaType> {
            match s.to_lowercase().as_str() {
                "image/jpeg" | "jpeg" | "jpg" => Some(ImageMediaType::JPEG),
                "image/png" | "png" => Some(ImageMediaType::PNG),
                "image/gif" | "gif" => Some(ImageMediaType::GIF),
                "image/webp" | "webp" => Some(ImageMediaType::WEBP),
                _ => None,
            }
        };

        let mut prompt_contents = vec![UserContent::Text(rig::message::Text {
            text: user_prompt.clone(),
            additional_params: None,
        })];
        if let Some(images) = &current_images {
            for img in images {
                prompt_contents.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(img.data.clone()),
                    media_type: parse_media_type(&img.media_type),
                    detail: Some(ImageDetail::Auto),
                    additional_params: None,
                }));
            }
        }
        let prompt_message = rig::message::Message::User {
            content: OneOrMany::many(prompt_contents).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(rig::message::Text {
                    text: user_prompt.clone(),
                    additional_params: None,
                }))
            }),
        };

        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| match msg.role {
                ChatRole::User => {
                    let mut contents: Vec<UserContent> =
                        vec![UserContent::Text(rig::message::Text {
                            text: msg.content.clone(),
                            additional_params: None,
                        })];
                    if let Some(images) = &msg.images {
                        for img in images {
                            contents.push(UserContent::Image(Image {
                                data: DocumentSourceKind::Base64(img.data.clone()),
                                media_type: parse_media_type(&img.media_type),
                                detail: Some(ImageDetail::Auto),
                                additional_params: None,
                            }));
                        }
                    }
                    OneOrMany::many(contents)
                        .ok()
                        .map(|content| rig::message::Message::User { content })
                }
                ChatRole::Assistant => Some(rig::message::Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: msg.content.clone(),
                        additional_params: None,
                    })),
                }),
            })
            .collect();

        let max_iterations = 8u64;
        let mut plan_step_counter = 0u32;
        let mut full_response = String::new();
        // Accumulated token usage of the whole assistant session (one call entry per iteration),
        // streamed to the frontend as a `<usage_stat>` frame at the end so the chat shows the
        // agent's own model usage alongside any stats reported by apps it called.
        let mut session_stats = LLMUsageStats::default();
        let mut current_prompt = prompt_message;

        for _iteration in 0..max_iterations {
            let request = agent
                .completion(current_prompt.clone(), current_history.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Completion error: {}", e))?
                .tools(tool_definitions.clone());
            let mut stream = request
                .stream()
                .await
                .map_err(|e| flow_like_types::anyhow!("Stream error: {}", e))?;

            let mut response_contents: Vec<AssistantContent> = Vec::new();
            let mut iteration_text = String::new();
            let mut current_reasoning = String::new();
            let mut reasoning_step_id: Option<String> = None;

            while let Some(item) = stream.next().await {
                let content =
                    item.map_err(|e| flow_like_types::anyhow!("Stream chunk error: {}", e))?;
                match content {
                    StreamedAssistantContent::Text(text) => {
                        iteration_text.push_str(&text.text);
                        if let Some(ref callback) = on_token {
                            callback(text.text.clone());
                        }
                        response_contents.push(AssistantContent::Text(text));
                    }
                    StreamedAssistantContent::ToolCall { tool_call, .. } => {
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta { .. } => {}
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let reasoning_text = reasoning
                            .content
                            .iter()
                            .filter_map(|c| match c {
                                rig::message::ReasoningContent::Text { text, .. } => {
                                    Some(text.as_str())
                                }
                                rig::message::ReasoningContent::Summary(s) => Some(s.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        current_reasoning.push_str(&reasoning_text);
                        current_reasoning.push('\n');
                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        current_reasoning.push_str(&reasoning);
                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                    StreamedAssistantContent::Final(res) => {
                        // Providers attach token usage to the final response of each turn; fold it
                        // into the session total (one call entry per model round-trip).
                        if let Some(usage) = res.token_usage() {
                            session_stats.accumulate(&Usage::from_rig(usage), Some(&model_name));
                        }
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            callback(plan_step_frame(
                                step_id.clone(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::Completed,
                                "think",
                            ));
                        }
                        reasoning_step_id = None;
                        current_reasoning.clear();
                    }
                }
            }

            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                callback(plan_step_frame(
                    step_id.clone(),
                    current_reasoning.trim().to_string(),
                    PlanStepStatus::Completed,
                    "think",
                ));
            }

            let tool_calls: Vec<_> = response_contents
                .iter()
                .filter_map(|content| match content {
                    AssistantContent::ToolCall(tool_call) => Some(tool_call.clone()),
                    _ => None,
                })
                .collect();

            if tool_calls.is_empty() {
                full_response.push_str(&iteration_text);
                break;
            }

            current_history.push(current_prompt.clone());

            let mut frame_ids: Vec<String> = Vec::new();
            for tool_call in &tool_calls {
                plan_step_counter += 1;
                let frame_id = if tool_call.id.is_empty() {
                    format!("step_{}", plan_step_counter)
                } else {
                    tool_call.id.clone()
                };
                if let Some(ref callback) = on_token {
                    callback(tool_start_frame(&frame_id, &tool_call.function.name, None));
                }
                frame_ids.push(frame_id);
            }

            let tool_futures: Vec<_> = tool_calls
                .iter()
                .map(|tool_call| {
                    let name = tool_call.function.name.clone();
                    let arguments = tool_call.function.arguments.clone();
                    let id = tool_call.id.clone();
                    let bridge = bridge.clone();
                    let memory = memory.clone();
                    async move {
                        let output =
                            execute_platform_tool(&name, arguments, &bridge, memory.as_ref()).await;
                        (id, name, output)
                    }
                })
                .collect();
            let tool_results: Vec<(String, String, String)> =
                futures::future::join_all(tool_futures).await;

            for (i, (_id, name, output)) in tool_results.iter().enumerate() {
                if let (Some(callback), Some(frame_id)) = (&on_token, frame_ids.get(i)) {
                    callback(tool_end_frame(frame_id, name, tool_output_status(output)));
                }
            }

            let assistant_msg = rig::message::Message::Assistant {
                id: None,
                content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                    OneOrMany::one(AssistantContent::Text(rig::message::Text {
                        text: String::new(),
                        additional_params: None,
                    }))
                }),
            };
            current_history.push(assistant_msg);

            let tool_result_contents: Vec<UserContent> = tool_results
                .iter()
                .map(|(tool_id, _name, tool_output)| {
                    UserContent::ToolResult(RigToolResult {
                        id: tool_id.clone(),
                        call_id: None,
                        content: OneOrMany::one(ToolResultContent::text(tool_output.clone())),
                    })
                })
                .collect();
            let combined = if tool_result_contents.len() == 1 {
                OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
            } else {
                OneOrMany::many(tool_result_contents)
                    .unwrap_or_else(|_| OneOrMany::one(UserContent::text("")))
            };
            current_prompt = rig::message::Message::User { content: combined };
        }

        // Publish the session's own token usage (skipped when no provider reported any).
        if session_stats.usage.total_tokens > 0 {
            session_stats.set_iterations(session_stats.calls.len() as u32);
            if let (Some(callback), Ok(stats)) = (&on_token, serde_json::to_value(&session_stats)) {
                callback(usage_stat_frame("Assistant", &stats));
            }
        }

        Ok(full_response)
    }
}

/// Dispatch a tool call: memory tools run locally (embed + LanceDB), everything else goes to the
/// host bridge (frontend). Keeps the assistant's persistent memory server-side and out of the bridge.
async fn execute_platform_tool(
    name: &str,
    arguments: Value,
    bridge: &Arc<dyn PlatformToolBridge>,
    memory: Option<&Arc<AssistantMemory>>,
) -> String {
    match name {
        MEMORY_STORE_TOOL | MEMORY_SEARCH_TOOL => {
            run_memory_tool(name, &arguments, memory.map(Arc::as_ref)).await
        }
        _ => bridge.call(name, arguments).await,
    }
}

/// Execute one of the `_memory_*` tools. Shared by every backend adapter so memory behaves
/// identically regardless of the selected model backend.
pub async fn run_memory_tool(
    name: &str,
    arguments: &Value,
    memory: Option<&AssistantMemory>,
) -> String {
    let Some(memory) = memory else {
        return json!({ "status": "error", "error": "Memory is not enabled." }).to_string();
    };
    match name {
        MEMORY_STORE_TOOL => {
            let content = arguments
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("");
            let role = arguments
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("observation");
            match memory.store(role, content).await {
                Ok(count) => json!({ "status": "ok", "observation_count": count }).to_string(),
                Err(error) => json!({ "status": "error", "error": error.to_string() }).to_string(),
            }
        }
        MEMORY_SEARCH_TOOL => {
            let query = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            match memory.search(query, 10).await {
                Ok(results) => json!({ "status": "ok", "results": results }).to_string(),
                Err(error) => json!({ "status": "error", "error": error.to_string() }).to_string(),
            }
        }
        _ => json!({ "status": "error", "error": format!("Unknown memory tool '{name}'.") })
            .to_string(),
    }
}

/// Map a tool's JSON output to the stream status the frontend renders ("error" → failed step).
fn tool_output_status(output: &str) -> &'static str {
    let is_error = serde_json::from_str::<Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status == "error")
        })
        .unwrap_or(false);
    if is_error { "error" } else { "done" }
}
