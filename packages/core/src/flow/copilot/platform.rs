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
    streaming::{StreamedAssistantContent, ToolCallDeltaContent},
};
use serde_json::{Value, json};

use super::memory::AssistantMemory;
use super::stream::{
    FlowScriptToolCallPreviewTracker, detailed_tool_end_frame, detailed_tool_start_frame,
    plan_step_frame, safe_tool_result_preview, tool_result_stream_status, tool_result_summary,
    tool_result_terminal_status, usage_stat_frame,
};
use super::tool_spec::{
    MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, find_global_tool_spec, global_assistant_tool_specs,
    resolve_tool_approval, spec_arg_str,
};
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

/// Whether a platform tool must preserve the model's declared call order within its round.
///
/// Read-only calls can run concurrently, but approval-requiring actions must not race each other:
/// for example, `flowpilot_board` has to finish persisting its entry node before a later
/// `upsert_event` can validate and register that node. Unknown tools are kept ordered as the safe
/// default because their side-effect policy is not available here.
fn platform_tool_requires_ordered_execution(name: &str, arguments: &Value) -> bool {
    let Some(spec) = find_global_tool_spec(name) else {
        return true;
    };
    resolve_tool_approval(&spec, arguments).kind != "none"
}

fn platform_tool_round_requires_ordered_execution<'a>(
    calls: impl IntoIterator<Item = (&'a str, &'a Value)>,
) -> bool {
    calls
        .into_iter()
        .any(|(name, arguments)| platform_tool_requires_ordered_execution(name, arguments))
}

fn is_editing_flowpilot_board_call(name: &str, arguments: &Value) -> bool {
    name == "flowpilot_board" && spec_arg_str(arguments, "mode", "mode") != "explain"
}

fn is_workflow_event_upsert_call(name: &str, arguments: &Value) -> bool {
    name == "upsert_event"
        && !spec_arg_str(arguments, "board_id", "boardId")
            .trim()
            .is_empty()
        && !spec_arg_str(arguments, "node_id", "nodeId")
            .trim()
            .is_empty()
}

/// A workflow Event cannot be registered from the same model round that creates its board entry.
/// The upsert arguments were authored before the model saw `flowpilot_board.event_nodes`, so even
/// sequential execution cannot make those arguments reliable. Return a retryable tool result and
/// let the next assistant round use the exact persisted ids from the board result.
fn same_round_workflow_event_guard_result(
    name: &str,
    arguments: &Value,
    round_has_editing_board_call: bool,
) -> Option<String> {
    (round_has_editing_board_call && is_workflow_event_upsert_call(name, arguments)).then(|| {
        json!({
            "status": "error",
            "code": "workflow_event_dependency_pending",
            "retryable": true,
            "next_action": "wait_for_flowpilot_board_event_nodes_then_retry",
            "message": "A workflow upsert_event cannot run in the same assistant round as flowpilot_board. Wait for flowpilot_board to succeed, read the exact board_id and node id from its event_nodes result, then call upsert_event in the next assistant round. The Event was not registered."
        })
        .to_string()
    })
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
                "image/heic" | "heic" => Some(ImageMediaType::HEIC),
                "image/heif" | "heif" => Some(ImageMediaType::HEIF),
                "image/svg+xml" | "svg" | "svg+xml" => Some(ImageMediaType::SVG),
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
            let mut flowscript_preview = FlowScriptToolCallPreviewTracker::default();

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
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    } => {
                        if let Some(ref callback) = on_token
                            && let Some(frame) = flowscript_preview.complete(
                                &internal_call_id,
                                &tool_call.function.name,
                                &tool_call.function.arguments,
                            )
                        {
                            callback(frame);
                        }
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta {
                        internal_call_id,
                        content,
                        ..
                    } => {
                        let frame = match content {
                            ToolCallDeltaContent::Name(name) => {
                                flowscript_preview.observe_name(&internal_call_id, &name);
                                None
                            }
                            ToolCallDeltaContent::Delta(delta) => flowscript_preview
                                .observe_arguments_delta(&internal_call_id, &delta),
                        };
                        if let (Some(callback), Some(frame)) = (&on_token, frame) {
                            callback(frame);
                        }
                    }
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
                    callback(detailed_tool_start_frame(
                        &frame_id,
                        &tool_call.function.name,
                        None,
                        Some(&tool_call.function.arguments),
                    ));
                }
                frame_ids.push(frame_id);
            }

            let round_has_editing_board_call = tool_calls.iter().any(|tool_call| {
                is_editing_flowpilot_board_call(
                    &tool_call.function.name,
                    &tool_call.function.arguments,
                )
            });
            let ordered_round = round_has_editing_board_call
                || platform_tool_round_requires_ordered_execution(tool_calls.iter().map(
                    |tool_call| {
                        (
                            tool_call.function.name.as_str(),
                            &tool_call.function.arguments,
                        )
                    },
                ));
            let tool_results: Vec<(String, String, String)> = if ordered_round {
                let mut results = Vec::with_capacity(tool_calls.len());
                for tool_call in &tool_calls {
                    let name = tool_call.function.name.clone();
                    if let Some(output) = same_round_workflow_event_guard_result(
                        &name,
                        &tool_call.function.arguments,
                        round_has_editing_board_call,
                    ) {
                        results.push((tool_call.id.clone(), name, output));
                        continue;
                    }
                    let output = execute_platform_tool(
                        &name,
                        tool_call.function.arguments.clone(),
                        &bridge,
                        memory.as_ref(),
                    )
                    .await;
                    results.push((tool_call.id.clone(), name, output));
                }
                results
            } else {
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
                                execute_platform_tool(&name, arguments, &bridge, memory.as_ref())
                                    .await;
                            (id, name, output)
                        }
                    })
                    .collect();
                futures::future::join_all(tool_futures).await
            };

            for (i, (_id, name, output)) in tool_results.iter().enumerate() {
                if let (Some(callback), Some(frame_id)) = (&on_token, frame_ids.get(i)) {
                    let terminal_status = tool_result_terminal_status(output);
                    let result_summary = tool_result_summary(output);
                    let result_preview =
                        safe_tool_result_preview(output, super::stream::TOOL_RESULT_PREVIEW_CHARS);
                    callback(detailed_tool_end_frame(
                        frame_id,
                        name,
                        tool_result_stream_status(output),
                        terminal_status.as_deref(),
                        Some(&result_summary),
                        Some(&result_preview),
                    ));
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

/// Run the shared `internet_search` tool (SearXNG) server-side. Async counterpart of the desktop's
/// blocking version, so both hosts share one implementation: the server FlowPilot bridge awaits it
/// directly and the desktop delegates to it via `block_on`.
pub async fn run_internet_search(args: &Value) -> Value {
    let query = spec_arg_str(args, "query", "query").trim().to_string();
    if query.is_empty() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "error": "internet_search requires a non-empty query."
        });
    }

    let language = {
        let language = spec_arg_str(args, "language", "language").trim();
        if language.is_empty() {
            "en-US".to_string()
        } else {
            language.to_string()
        }
    };
    let page = args
        .get("page")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 100);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;

    let client = match flow_like_types::reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("FlowPilot/1.0")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "error": format!("Failed to create search client: {error}")
            });
        }
    };

    let page_str = page.to_string();
    let response = match client
        .get("https://search.flow-like.com/search")
        .query(&[
            ("q", query.as_str()),
            ("format", "json"),
            ("pageno", page_str.as_str()),
            ("language", language.as_str()),
        ])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search request failed: {error}")
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "query": query,
            "http_status": status.as_u16(),
            "error": format!("Search request failed with HTTP {status}")
        });
    }

    let payload = match response.json::<Value>().await {
        Ok(payload) => payload,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search response was not valid JSON: {error}")
            });
        }
    };

    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(limit)
                .map(compact_search_result)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "status": "ok",
        "query": query,
        "page": page,
        "results": results
    })
}

fn compact_search_result(result: &Value) -> Value {
    let object = result.as_object();
    json!({
        "title": object.and_then(|item| item.get("title")).cloned().unwrap_or(Value::Null),
        "url": object.and_then(|item| item.get("url")).cloned().unwrap_or(Value::Null),
        "content": object.and_then(|item| item.get("content")).cloned().unwrap_or(Value::Null),
        "publishedDate": object.and_then(|item| item.get("publishedDate")).cloned().unwrap_or(Value::Null),
        "engine": object.and_then(|item| item.get("engine")).cloned().unwrap_or(Value::Null),
        "category": object.and_then(|item| item.get("category")).cloned().unwrap_or(Value::Null),
        "score": object.and_then(|item| item.get("score")).cloned().unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_platform_rounds_can_run_in_parallel() {
        let list_args = json!({});
        let describe_args = json!({ "app_id": "app", "event_id": "event" });

        assert!(!platform_tool_round_requires_ordered_execution([
            ("list_apps", &list_args),
            ("describe_app_interface", &describe_args),
        ]));
    }

    #[test]
    fn approval_requiring_platform_rounds_preserve_model_order() {
        let list_args = json!({});
        let board_args = json!({
            "app_id": "app",
            "instruction": "Build the workflow",
        });
        let event_args = json!({
            "app_id": "app",
            "name": "Scheduled poll",
            "board_id": "board",
            "node_id": "entry",
        });

        assert!(platform_tool_round_requires_ordered_execution([
            ("list_apps", &list_args),
            ("flowpilot_board", &board_args),
            ("upsert_event", &event_args),
        ]));
    }

    #[test]
    fn read_only_board_explanations_do_not_force_ordered_execution() {
        let explain_args = json!({
            "app_id": "app",
            "board_id": "board",
            "instruction": "Explain this workflow",
            "mode": "explain",
        });

        assert!(!platform_tool_requires_ordered_execution(
            "flowpilot_board",
            &explain_args,
        ));
    }

    #[test]
    fn unknown_platform_tools_default_to_ordered_execution() {
        assert!(platform_tool_requires_ordered_execution(
            "future_side_effecting_tool",
            &json!({}),
        ));
    }

    #[test]
    fn runtime_execution_is_ordered_but_log_queries_are_read_only() {
        let execute_args = json!({
            "app_id": "app",
            "board_id": "board",
            "node_id": "node",
        });
        let log_args = json!({
            "app_id": "app",
            "board_id": "board",
            "run_id": "run",
        });

        assert!(platform_tool_requires_ordered_execution(
            "execute_node",
            &execute_args,
        ));
        assert!(!platform_tool_requires_ordered_execution(
            "query_execution_logs",
            &log_args,
        ));
    }

    #[test]
    fn workflow_event_upsert_is_deferred_when_board_is_edited_in_same_round() {
        let event_args = json!({
            "app_id": "app",
            "name": "Scheduled poll",
            "board_id": "board",
            "node_id": "guessed-entry",
        });
        let output = same_round_workflow_event_guard_result("upsert_event", &event_args, true)
            .expect("workflow upsert must be deferred");
        let payload: Value = serde_json::from_str(&output).expect("structured guard result");

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "workflow_event_dependency_pending");
        assert_eq!(payload["retryable"], true);
        assert_eq!(
            payload["next_action"],
            "wait_for_flowpilot_board_event_nodes_then_retry"
        );
    }

    #[test]
    fn page_only_event_upsert_is_allowed_in_board_edit_round() {
        let page_event_args = json!({
            "app_id": "app",
            "name": "Dashboard",
            "page_id": "page",
            "route": "/dashboard",
        });

        assert!(
            same_round_workflow_event_guard_result("upsert_event", &page_event_args, true,)
                .is_none()
        );
    }

    #[test]
    fn workflow_event_upsert_is_allowed_without_same_round_board_edit() {
        let event_args = json!({
            "app_id": "app",
            "name": "Scheduled poll",
            "board_id": "board",
            "node_id": "persisted-entry",
        });

        assert!(
            same_round_workflow_event_guard_result("upsert_event", &event_args, false,).is_none()
        );
    }

    #[test]
    fn explain_board_call_does_not_trigger_workflow_event_guard() {
        assert!(!is_editing_flowpilot_board_call(
            "flowpilot_board",
            &json!({ "mode": "explain" }),
        ));
        assert!(is_editing_flowpilot_board_call(
            "flowpilot_board",
            &json!({}),
        ));
    }

    #[test]
    fn terminal_bridge_failures_never_render_as_done() {
        for status in [
            "error",
            "failed",
            "timeout",
            "timed_out",
            "denied",
            "cancelled",
            "validation_errors",
        ] {
            let output = json!({ "status": status }).to_string();
            assert_eq!(
                tool_result_stream_status(&output),
                "error",
                "{status} must render as failed"
            );
            assert_eq!(
                tool_result_terminal_status(&output).as_deref(),
                Some(status)
            );
        }
        for status in ["ok", "done", "queued", "applied", "completed"] {
            assert_eq!(
                tool_result_stream_status(&json!({ "status": status }).to_string()),
                "done"
            );
        }
    }

    #[test]
    fn tool_summary_is_bounded_to_status_and_counts() {
        let output = json!({
            "status": "timeout",
            "error": "must not appear in summary",
            "commands": [{}, {}],
            "diagnostics": [{}],
        })
        .to_string();
        let summary = tool_result_summary(&output);

        assert_eq!(summary, "timeout · 2 command(s) · 1 diagnostic(s)");
        assert!(!summary.contains("must not appear"));
    }
}
