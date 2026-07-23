//! Platform assistant runner — makes profile ("Bits") models tool-capable for the global FlowPilot
//! assistant, reusing the exact machinery the board copilot uses for Bits: resolve the profile model
//! into a rig completion client, attach the shared platform tool specs to each completion request,
//! and run the manual tool-call loop, streaming the same tagged-frame protocol (`<tool_start>`,
//! `<tool_end>`, `<plan_step>`) that the shared frontend parser renders for every backend.
//!
//! Most platform tools act on the host app (navigate, create app, delegate to the board copilot,
//! ask the user), so core defines their specs plus a `PlatformToolBridge`; host-neutral memory and
//! safe public-web reads execute in core.

use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
    time::Duration,
};

use async_trait::async_trait;
use flow_like_model_provider::llm::CompletionClientDyn;
use flow_like_model_provider::provider::ModelProvider;
use flow_like_model_provider::response::{LLMUsageStats, Usage};
use flow_like_types::{Result, tokio};
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
use url::Url;

use super::memory::AssistantMemory;
use super::public_web::{
    OpenUrlSessionBudget, WebResearchSession, normalize_public_discovery_url,
    run_archive_lookup_for_session, run_open_url_for_session, source_id_for_url,
};
use super::stream::{
    FlowScriptToolCallPreviewTracker, detailed_tool_end_frame, detailed_tool_start_frame,
    plan_step_frame, safe_tool_result_preview, tool_result_stream_status, tool_result_summary,
    tool_result_terminal_status, usage_stat_frame,
};
use super::tool_spec::{
    ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL,
    OPEN_URL_TOOL, find_global_tool_spec, global_assistant_tool_specs, resolve_tool_effect,
    spec_arg_str,
};
use super::types::{ChatImage, ChatMessage, ChatRole, PlanStepStatus};
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters};
use crate::profile::Profile;
use crate::state::FlowLikeState;

/// Private frontend-tool result field used to carry temporary vision URLs alongside normal JSON.
/// Host adapters remove it before exposing the textual result to the model or diagnostics.
pub const PLATFORM_TOOL_IMAGE_URLS_FIELD: &str = "_flowpilot_image_urls";

const MAX_PLATFORM_TOOL_ROUNDS: usize = 8;
const MAX_SEARCH_CALLS_PER_ROUND: usize = 5;
const MAX_SEARCH_CALLS_PER_SESSION: usize = 12;
const MAX_ARCHIVE_CALLS_PER_ROUND: usize = 2;
const MAX_ARCHIVE_CALLS_PER_SESSION: usize = 4;
const MAX_SEARCH_QUERY_CHARS: usize = 512;
const MAX_SEARCH_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SEARCH_TITLE_CHARS: usize = 300;
const MAX_SEARCH_SNIPPET_CHARS: usize = 1_200;
const MAX_CONCURRENT_SEARCH_CALLS: usize = 8;
static SEARCH_CONCURRENCY: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_SEARCH_CALLS));

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PlatformToolImageUrl {
    pub url: String,
    pub media_type: String,
}

/// Remove and validate temporary image references from a frontend tool result.
pub fn take_platform_tool_image_urls(value: &mut Value) -> Vec<PlatformToolImageUrl> {
    let Some(object) = value.as_object_mut() else {
        return Vec::new();
    };
    object
        .remove(PLATFORM_TOOL_IMAGE_URLS_FIELD)
        .and_then(|images| serde_json::from_value::<Vec<PlatformToolImageUrl>>(images).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|image| {
            image.media_type.starts_with("image/")
                && (image.url.starts_with("https://") || image.url.starts_with("http://"))
        })
        .take(12)
        .collect()
}

fn split_platform_tool_output(output: String) -> (String, Vec<PlatformToolImageUrl>) {
    let Ok(mut value) = serde_json::from_str::<Value>(&output) else {
        return (output, Vec::new());
    };
    let had_image_field = value
        .as_object()
        .is_some_and(|object| object.contains_key(PLATFORM_TOOL_IMAGE_URLS_FIELD));
    let images = take_platform_tool_image_urls(&mut value);
    if !had_image_field {
        return (output, images);
    }
    let clean = serde_json::to_string(&value).unwrap_or(output);
    (clean, images)
}

fn parse_image_media_type(value: &str) -> Option<ImageMediaType> {
    match value.to_lowercase().as_str() {
        "image/jpeg" | "jpeg" | "jpg" => Some(ImageMediaType::JPEG),
        "image/png" | "png" => Some(ImageMediaType::PNG),
        "image/gif" | "gif" => Some(ImageMediaType::GIF),
        "image/webp" | "webp" => Some(ImageMediaType::WEBP),
        "image/heic" | "heic" => Some(ImageMediaType::HEIC),
        "image/heif" | "heif" => Some(ImageMediaType::HEIF),
        "image/svg+xml" | "svg" | "svg+xml" => Some(ImageMediaType::SVG),
        _ => None,
    }
}

/// Executes the global assistant's platform tools. Implemented by the desktop crate over the
/// FrontendToolBridge (with per-tool approval); returns the tool result as a string for the model.
#[async_trait]
pub trait PlatformToolBridge: Send + Sync {
    async fn call(&self, tool_name: &str, arguments: Value) -> String;
}

/// Whether a platform tool must preserve the model's declared call order within its round.
///
/// Read-only calls can run concurrently, but side-effecting actions must not race each other:
/// for example, `flowpilot_board` has to finish persisting its entry node before a later
/// `upsert_event` can validate and register that node. Unknown tools are kept ordered as the safe
/// default because their side-effect policy is not available here. Ordering is based on effect,
/// not approval timing: preparing a deferred-approval board edit remains an execute operation.
fn platform_tool_requires_ordered_execution(name: &str, arguments: &Value) -> bool {
    let Some(spec) = find_global_tool_spec(name) else {
        return true;
    };
    resolve_tool_effect(&spec, arguments).requires_ordered_execution()
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

        let mut prompt_contents = vec![UserContent::Text(rig::message::Text {
            text: user_prompt.clone(),
            additional_params: None,
        })];
        if let Some(images) = &current_images {
            for img in images {
                prompt_contents.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(img.data.clone()),
                    media_type: parse_image_media_type(&img.media_type),
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
                                media_type: parse_image_media_type(&img.media_type),
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

        let mut plan_step_counter = 0u32;
        let mut full_response = String::new();
        // Accumulated token usage of the whole assistant session (one call entry per iteration),
        // streamed to the frontend as a `<usage_stat>` frame at the end so the chat shows the
        // agent's own model usage alongside any stats reported by apps it called.
        let mut session_stats = LLMUsageStats::default();
        let mut current_prompt = prompt_message;
        let mut open_url_budget = OpenUrlSessionBudget::default();
        let web_research_session = Arc::new(WebResearchSession::new(&user_prompt));
        let mut session_search_calls = 0usize;
        let mut session_archive_calls = 0usize;

        // Tool rounds and answer generation have separate budgets. The last iteration deliberately
        // advertises no tools, guaranteeing that a search/open chain cannot consume the final turn
        // and leave the user without a synthesis.
        for iteration in 0..=MAX_PLATFORM_TOOL_ROUNDS {
            let tools_enabled = iteration < MAX_PLATFORM_TOOL_ROUNDS;
            let tools_for_round = if tools_enabled {
                tool_definitions.clone()
            } else {
                Vec::new()
            };
            let request = agent
                .completion(current_prompt.clone(), current_history.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Completion error: {}", e))?
                .tools(tools_for_round);
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

            // A provider should not emit calls for tools that were not advertised. If it does on
            // the reserved synthesis turn, stop safely instead of executing an unbudgeted action.
            if !tools_enabled {
                full_response.push_str(&iteration_text);
                if full_response.trim().is_empty() {
                    full_response.push_str(
                        "The research tools completed, but the model did not produce a final synthesis within the tool budget.",
                    );
                }
                break;
            }

            open_url_budget.begin_round(
                tool_calls
                    .iter()
                    .filter(|tool_call| tool_call.function.name == OPEN_URL_TOOL)
                    .count(),
            );
            let mut round_search_calls = 0usize;
            let mut round_archive_calls = 0usize;
            let mut prepared_arguments = Vec::with_capacity(tool_calls.len());
            for tool_call in &tool_calls {
                let tool_name = tool_call.function.name.as_str();
                if is_public_web_tool(tool_name)
                    && let Some(error) = web_research_session.public_web_phase_error(tool_name)
                {
                    prepared_arguments.push(Err(error.to_string()));
                    continue;
                }
                let prepared = match tool_name {
                    INTERNET_SEARCH_TOOL => {
                        if round_search_calls >= MAX_SEARCH_CALLS_PER_ROUND {
                            Err(web_call_budget_error(
                                tool_name,
                                "search_round_call_budget_exceeded",
                                "This research round reached its search-query budget. Inspect the current results and refine only unresolved gaps in the next round.",
                                true,
                            ))
                        } else if session_search_calls >= MAX_SEARCH_CALLS_PER_SESSION {
                            Err(web_call_budget_error(
                                tool_name,
                                "search_session_call_budget_exceeded",
                                "This assistant run reached its search-query budget. Synthesize the strongest verified evidence already collected and disclose remaining gaps.",
                                false,
                            ))
                        } else {
                            round_search_calls += 1;
                            session_search_calls += 1;
                            Ok(tool_call.function.arguments.clone())
                        }
                    }
                    ARCHIVE_LOOKUP_TOOL => {
                        if round_archive_calls >= MAX_ARCHIVE_CALLS_PER_ROUND {
                            Err(web_call_budget_error(
                                tool_name,
                                "archive_round_call_budget_exceeded",
                                "This research round reached its archive-lookup budget. Inspect the returned captures before trying another historical lead.",
                                true,
                            ))
                        } else if session_archive_calls >= MAX_ARCHIVE_CALLS_PER_SESSION {
                            Err(web_call_budget_error(
                                tool_name,
                                "archive_session_call_budget_exceeded",
                                "This assistant run reached its archive-lookup budget. Use the captures already found or disclose that the historical record remains incomplete.",
                                false,
                            ))
                        } else {
                            round_archive_calls += 1;
                            session_archive_calls += 1;
                            Ok(tool_call.function.arguments.clone())
                        }
                    }
                    _ => open_url_budget
                        .prepare_call(tool_name, tool_call.function.arguments.clone()),
                };
                prepared_arguments.push(prepared);
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
            let tool_results: Vec<(String, String, String, Vec<PlatformToolImageUrl>)> =
                if ordered_round {
                    let mut results = Vec::with_capacity(tool_calls.len());
                    for (tool_call, prepared) in tool_calls.iter().zip(prepared_arguments.iter()) {
                        let name = tool_call.function.name.clone();
                        if let Some(output) = same_round_workflow_event_guard_result(
                            &name,
                            &tool_call.function.arguments,
                            round_has_editing_board_call,
                        ) {
                            results.push((tool_call.id.clone(), name, output, Vec::new()));
                            continue;
                        }
                        let arguments = match prepared {
                            Ok(arguments) => arguments.clone(),
                            Err(output) => {
                                results.push((
                                    tool_call.id.clone(),
                                    name,
                                    output.clone(),
                                    Vec::new(),
                                ));
                                continue;
                            }
                        };
                        let output = execute_platform_tool(
                            &name,
                            arguments,
                            &bridge,
                            memory.as_ref(),
                            &web_research_session,
                        )
                        .await;
                        let (output, images) = split_platform_tool_output(output);
                        results.push((tool_call.id.clone(), name, output, images));
                    }
                    results
                } else {
                    let tool_futures: Vec<_> = tool_calls
                        .iter()
                        .zip(prepared_arguments.iter())
                        .map(|(tool_call, prepared)| {
                            let name = tool_call.function.name.clone();
                            let prepared = prepared.clone();
                            let id = tool_call.id.clone();
                            let bridge = bridge.clone();
                            let memory = memory.clone();
                            let web_research_session = web_research_session.clone();
                            async move {
                                let output = match prepared {
                                    Ok(arguments) => {
                                        execute_platform_tool(
                                            &name,
                                            arguments,
                                            &bridge,
                                            memory.as_ref(),
                                            &web_research_session,
                                        )
                                        .await
                                    }
                                    Err(output) => output,
                                };
                                let (output, images) = split_platform_tool_output(output);
                                (id, name, output, images)
                            }
                        })
                        .collect();
                    futures::future::join_all(tool_futures).await
                };

            // All calls in one model-authored batch may complete. Once that batch has introduced
            // app, memory, or interactive data, later model rounds lose public-network access so
            // private values cannot be transformed into a new query or outbound URL.
            let round_entered_private_context = tool_calls
                .iter()
                .any(|tool_call| !is_public_web_tool(&tool_call.function.name));
            if round_entered_private_context {
                web_research_session.close_public_web_phase();
            }

            for (i, (_id, name, output, _images)) in tool_results.iter().enumerate() {
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

            let mut tool_result_contents: Vec<UserContent> = Vec::new();
            let mut tool_images: Vec<&PlatformToolImageUrl> = Vec::new();
            for (tool_id, _name, tool_output, images) in &tool_results {
                tool_result_contents.push(UserContent::ToolResult(RigToolResult {
                    id: tool_id.clone(),
                    call_id: None,
                    content: OneOrMany::one(ToolResultContent::text(tool_output.clone())),
                }));
                tool_images.extend(images);
            }
            let combined = if tool_result_contents.len() == 1 {
                OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
            } else {
                OneOrMany::many(tool_result_contents)
                    .unwrap_or_else(|_| OneOrMany::one(UserContent::text("")))
            };
            let tool_result_message = rig::message::Message::User { content: combined };

            let synthesis_next = iteration + 1 == MAX_PLATFORM_TOOL_ROUNDS;
            let web_research_round = tool_calls.iter().any(|tool_call| {
                matches!(
                    tool_call.function.name.as_str(),
                    INTERNET_SEARCH_TOOL | OPEN_URL_TOOL | ARCHIVE_LOOKUP_TOOL
                )
            });
            if tool_images.is_empty()
                && !synthesis_next
                && !web_research_round
                && !round_entered_private_context
            {
                current_prompt = tool_result_message;
            } else {
                // OpenAI-style providers intentionally discard non-tool blocks from a message that
                // also contains tool results. Keep the required assistant → tool-result ordering,
                // then attach the visual result in an immediate user-context message in this same
                // agent iteration so every vision-capable provider actually receives it.
                current_history.push(tool_result_message);
                let mut image_contents = Vec::new();
                if synthesis_next {
                    let citation_allowlist =
                        citation_allowlist_text(&web_research_session.opened_urls());
                    image_contents.push(UserContent::text(format!(
                        "The tool budget is complete. Produce the final answer now from the verified evidence already collected. Do not call more tools. Put citations next to their claims, state material gaps or conflicts, and use no clickable web citation outside this exact successfully-opened URL allowlist:\n{citation_allowlist}"
                    )));
                } else if web_research_round {
                    let citation_allowlist =
                        citation_allowlist_text(&web_research_session.opened_urls());
                    image_contents.push(UserContent::text(format!(
                        "Host-verified web evidence state after this research round. Search snippets and archive candidates remain discovery-only; cite only successfully opened pages. Raw source IDs are internal and must not appear in the answer. Tools remain available for unresolved gaps. Exact currently citable URL allowlist:\n{citation_allowlist}"
                    )));
                }
                if round_entered_private_context {
                    image_contents.push(UserContent::text(
                        "Host privacy boundary: app, memory, or interactive user data has now entered the working context. Public-web tools are closed for the rest of this assistant run. Finish from public evidence already collected and do not derive a query or outbound URL from private data.",
                    ));
                }
                if !tool_images.is_empty() {
                    image_contents.push(UserContent::text(
                        "Rendered app page capture(s) from the preceding open_app_page result, ordered top-to-bottom. Inspect all images before answering about the page's content.",
                    ));
                }
                image_contents.extend(tool_images.into_iter().map(|image| {
                    UserContent::Image(Image {
                        data: DocumentSourceKind::Url(image.url.clone()),
                        media_type: parse_image_media_type(&image.media_type),
                        detail: Some(ImageDetail::High),
                        additional_params: None,
                    })
                }));
                current_prompt = rig::message::Message::User {
                    content: OneOrMany::many(image_contents)
                        .unwrap_or_else(|_| OneOrMany::one(UserContent::text(""))),
                };
            }
        }

        if full_response.trim().is_empty() {
            let fallback = "The research run ended without a usable final synthesis. No additional web action was taken; please retry or narrow the question.".to_string();
            if let Some(callback) = &on_token {
                callback(fallback.clone());
            }
            full_response = fallback;
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

fn citation_allowlist_text(opened_urls: &[String]) -> String {
    if opened_urls.is_empty() {
        return "(none — open a search/archive result before citing it)".to_string();
    }
    let mut opened_urls = opened_urls.to_vec();
    opened_urls.sort();
    opened_urls.dedup();
    opened_urls
        .iter()
        .map(|url| format!("- {url}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn web_call_budget_error(tool: &str, code: &str, message: &str, retryable: bool) -> String {
    json!({
        "status": "error",
        "tool": tool,
        "code": code,
        "retryable": retryable,
        "error": message,
    })
    .to_string()
}

fn is_public_web_tool(name: &str) -> bool {
    matches!(
        name,
        INTERNET_SEARCH_TOOL | OPEN_URL_TOOL | ARCHIVE_LOOKUP_TOOL
    )
}

/// Dispatch a tool call: memory and safe public-page reads run locally; everything else goes to the
/// host bridge (frontend). This keeps persistent memory and arbitrary-URL safety enforcement out of
/// the browser bridge.
async fn execute_platform_tool(
    name: &str,
    arguments: Value,
    bridge: &Arc<dyn PlatformToolBridge>,
    memory: Option<&Arc<AssistantMemory>>,
    web_research_session: &WebResearchSession,
) -> String {
    match name {
        MEMORY_STORE_TOOL | MEMORY_SEARCH_TOOL => {
            run_memory_tool(name, &arguments, memory.map(Arc::as_ref)).await
        }
        INTERNET_SEARCH_TOOL => {
            if let Some(error) = web_research_session.public_web_phase_error(name) {
                return error.to_string();
            }
            let mut result = run_internet_search(&arguments).await;
            web_research_session.register_and_decorate_tool_result(name, &mut result);
            serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
        }
        OPEN_URL_TOOL => {
            let mut result = run_open_url_for_session(&arguments, web_research_session).await;
            web_research_session.register_and_decorate_tool_result(name, &mut result);
            serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
        }
        ARCHIVE_LOOKUP_TOOL => {
            let mut result = run_archive_lookup_for_session(&arguments, web_research_session).await;
            web_research_session.register_and_decorate_tool_result(name, &mut result);
            serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
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
    if query.chars().count() > MAX_SEARCH_QUERY_CHARS {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "code": "query_too_long",
            "error": format!("Search queries are limited to {MAX_SEARCH_QUERY_CHARS} characters. Split the research question into focused subqueries."),
        });
    }
    if query.chars().any(is_unsafe_search_character) {
        return json!({
            "status": "error",
            "tool": INTERNET_SEARCH_TOOL,
            "code": "invalid_query",
            "error": "Search queries may not contain control or bidirectional-formatting characters.",
        });
    }
    if search_query_contains_likely_secret(&query) {
        return json!({
            "status": "error",
            "tool": INTERNET_SEARCH_TOOL,
            "code": "sensitive_query_not_allowed",
            "error": "The search query appears to contain a credential, access token, private key, or secret value and was not sent.",
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
    if language.len() > 32
        || !language
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "code": "invalid_language",
            "error": "language must be a short SearXNG language code such as en-US.",
        });
    }
    let time_range = spec_arg_str(args, "time_range", "timeRange")
        .trim()
        .to_ascii_lowercase();
    if !time_range.is_empty() && !matches!(time_range.as_str(), "day" | "week" | "month" | "year") {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "code": "invalid_time_range",
            "error": "time_range must be one of day, week, month, or year.",
        });
    }
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

    let _search_permit = match tokio::time::timeout(
        Duration::from_secs(5),
        SEARCH_CONCURRENCY.acquire(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            return json!({
                "status": "error",
                "tool": INTERNET_SEARCH_TOOL,
                "code": "concurrency_unavailable",
                "retryable": true,
                "error": "The search service concurrency guard is temporarily unavailable.",
            });
        }
        Err(_) => {
            return json!({
                "status": "error",
                "tool": INTERNET_SEARCH_TOOL,
                "code": "concurrency_busy",
                "retryable": true,
                "error": "The search service is busy; retry after inspecting the current evidence.",
            });
        }
    };

    let client = match flow_like_types::reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(flow_like_types::reqwest::redirect::Policy::none())
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
    let mut query_params = vec![
        ("q", query.clone()),
        ("format", "json".to_string()),
        ("pageno", page_str),
        ("language", language.clone()),
    ];
    if !time_range.is_empty() {
        query_params.push(("time_range", time_range.clone()));
    }
    let response = match client
        .get("https://search.flow-like.com/search")
        .query(&query_params)
        .header(flow_like_types::reqwest::header::ACCEPT, "application/json")
        .header(
            flow_like_types::reqwest::header::ACCEPT_ENCODING,
            "identity",
        )
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
    if response
        .headers()
        .get(flow_like_types::reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("identity"))
    {
        return json!({
            "status": "error",
            "tool": INTERNET_SEARCH_TOOL,
            "query": query,
            "code": "unsupported_content_encoding",
            "error": "The search service ignored the identity encoding request.",
        });
    }
    let media_type = response
        .headers()
        .get(flow_like_types::reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media_type != "application/json" && !media_type.ends_with("+json") {
        return json!({
            "status": "error",
            "tool": INTERNET_SEARCH_TOOL,
            "query": query,
            "code": "unsupported_content_type",
            "error": "The search service did not return JSON.",
        });
    }

    let body = match read_bounded_search_body(response).await {
        Ok(body) => body,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "code": "response_too_large",
                "error": error,
            });
        }
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "code": "invalid_response",
                "error": format!("Search response was not valid JSON: {error}")
            });
        }
    };

    let mut seen_urls = HashSet::new();
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(compact_search_result)
                .filter(|result| {
                    result
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(|url| seen_urls.insert(url.to_string()))
                })
                .take(limit)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let suggestions = compact_search_strings(payload.get("suggestions"), 5, 160);
    let corrections = compact_search_strings(payload.get("corrections"), 5, 160);

    json!({
        "status": "ok",
        "tool": "internet_search",
        "query": query,
        "language": language,
        "time_range": if time_range.is_empty() { Value::Null } else { Value::String(time_range) },
        "page": page,
        "searched_at": chrono::Utc::now().to_rfc3339(),
        "results": results,
        "suggestions": suggestions,
        "corrections": corrections,
        "citation_eligible": false,
        "untrusted_content": true,
    })
}

async fn read_bounded_search_body(
    mut response: flow_like_types::reqwest::Response,
) -> std::result::Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SEARCH_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "Search response exceeded the {MAX_SEARCH_RESPONSE_BYTES}-byte safety limit."
        ));
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_SEARCH_RESPONSE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Failed to read search response: {error}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_SEARCH_RESPONSE_BYTES {
            return Err(format!(
                "Search response exceeded the {MAX_SEARCH_RESPONSE_BYTES}-byte safety limit."
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn compact_search_result(result: &Value) -> Option<Value> {
    let object = result.as_object()?;
    let url = normalized_search_result_url(object.get("url")?.as_str()?)?;
    let source_id = source_id_for_url(&url);
    let domain = Url::parse(&url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));
    let title = sanitize_search_text(
        object
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled source"),
        MAX_SEARCH_TITLE_CHARS,
    );
    let content = object
        .get("content")
        .and_then(Value::as_str)
        .map(|value| sanitize_search_text(value, MAX_SEARCH_SNIPPET_CHARS));
    let published_date = object
        .get("publishedDate")
        .or_else(|| object.get("published_date"))
        .and_then(Value::as_str)
        .map(|value| sanitize_search_text(value, 64));
    let engine = object
        .get("engine")
        .and_then(Value::as_str)
        .map(|value| sanitize_search_text(value, 80));
    let category = object
        .get("category")
        .and_then(Value::as_str)
        .map(|value| sanitize_search_text(value, 80));
    let score = object.get("score").and_then(Value::as_f64);
    Some(json!({
        "source_id": source_id,
        "title": title,
        "url": url,
        "domain": domain,
        "content": content,
        "publishedDate": published_date,
        "engine": engine,
        "category": category,
        "score": score,
        "citation_eligible": false,
    }))
}

fn normalized_search_result_url(raw: &str) -> Option<String> {
    normalize_public_discovery_url(raw)
}

fn compact_search_strings(value: Option<&Value>, limit: usize, max_chars: usize) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| sanitize_search_text(value, max_chars))
        .filter(|value| !value.is_empty())
        .take(limit)
        .collect()
}

fn sanitize_search_text(value: &str, max_chars: usize) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| {
            (*character == '\n' || *character == '\t' || !character.is_control())
                && !matches!(
                    *character,
                    '\u{200b}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
        })
        .collect();
    sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn is_unsafe_search_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200b}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

fn search_query_contains_likely_secret(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    if [
        "-----begin private key",
        "-----begin rsa private key",
        "access_token=",
        "api_key=",
        "apikey=",
        "authorization: bearer ",
        "password=",
        "secret=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return true;
    }

    query
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';')
        })
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(character, '(' | ')' | '[' | ']' | '{' | '}')
            })
        })
        .any(|token| {
            let lower = token.to_ascii_lowercase();
            (lower.starts_with("sk-") && token.len() >= 20)
                || (lower.starts_with("ghp_") && token.len() >= 20)
                || (lower.starts_with("github_pat_") && token.len() >= 30)
                || (lower.starts_with("xoxb-") && token.len() >= 20)
                || (token.starts_with("AKIA")
                    && token.len() == 20
                    && token.chars().all(|character| {
                        character.is_ascii_uppercase() || character.is_ascii_digit()
                    }))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citation_allowlist_is_sorted_deduplicated_and_requires_opened_urls() {
        assert_eq!(
            citation_allowlist_text(&[]),
            "(none — open a search/archive result before citing it)"
        );
        assert_eq!(
            citation_allowlist_text(&[
                "https://z.example/source".to_string(),
                "https://a.example/source".to_string(),
                "https://z.example/source".to_string(),
            ]),
            "- https://a.example/source\n- https://z.example/source"
        );
    }

    #[test]
    fn compact_search_results_include_stable_source_provenance() {
        let result = compact_search_result(&json!({
            "title": "Flow-Like",
            "url": "https://flow-like.com/docs",
            "content": "Documentation",
        }))
        .expect("public result should be retained");
        let expected_source_id = source_id_for_url("https://flow-like.com/docs");
        assert_eq!(
            result["source_id"].as_str(),
            Some(expected_source_id.as_str())
        );
        assert_eq!(result["url"], "https://flow-like.com/docs");
        assert_eq!(result["citation_eligible"], false);
    }

    #[test]
    fn search_results_reject_non_public_or_credentialed_urls() {
        for url in [
            "file:///etc/passwd",
            "http://localhost/admin",
            "http://127.0.0.1/private",
            "https://example.com:8443/private",
            "https://user:secret@example.com/private",
            "https://example.com/private?access_token=secret",
            "https://example.com/private?X-Amz-Signature=secret",
        ] {
            assert!(
                compact_search_result(&json!({ "title": "unsafe", "url": url })).is_none(),
                "unsafe search URL was retained: {url}"
            );
        }

        let oversized = format!("https://example.com/{}", "x".repeat(4_096));
        assert!(compact_search_result(&json!({ "title": "unsafe", "url": oversized })).is_none());
    }

    #[test]
    fn search_result_text_is_bounded_and_strips_directional_controls() {
        let title = format!("safe\u{202e}title\u{2066} {}", "x".repeat(500));
        let result = compact_search_result(&json!({
            "title": title,
            "url": "https://EXAMPLE.com/article#fragment",
            "content": "first\nsecond\u{0000}\u{202d}third",
        }))
        .expect("public result should be retained");

        let clean_title = result["title"].as_str().expect("sanitized title");
        let clean_content = result["content"].as_str().expect("sanitized snippet");
        assert!(clean_title.chars().count() <= MAX_SEARCH_TITLE_CHARS);
        assert!(!clean_title.contains('\u{202e}'));
        assert!(!clean_title.contains('\u{2066}'));
        assert!(!clean_content.contains('\u{0000}'));
        assert!(!clean_content.contains('\u{202d}'));
        assert_eq!(result["url"], "https://example.com/article");
    }

    #[test]
    fn search_query_controls_are_rejected_by_validation() {
        assert!(is_unsafe_search_character('\n'));
        assert!(is_unsafe_search_character('\u{202e}'));
        assert!(!is_unsafe_search_character('é'));
    }

    #[test]
    fn likely_secrets_are_blocked_from_search_queries() {
        assert!(search_query_contains_likely_secret(
            "debug token sk-1234567890abcdefghijklmnop"
        ));
        assert!(search_query_contains_likely_secret(
            "authorization: bearer abcdefghijklmnopqrstuvwxyz"
        ));
        assert!(!search_query_contains_likely_secret(
            "OpenAI API key security best practices"
        ));
    }

    #[test]
    fn page_capture_urls_are_removed_from_textual_tool_output() {
        let raw = json!({
            "status": "ok",
            "screenshot_count": 2,
            "_flowpilot_image_urls": [
                { "url": "https://tmp.example/first.png", "media_type": "image/png" },
                { "url": "https://tmp.example/second.png", "media_type": "image/png" },
            ],
        })
        .to_string();

        let (text, images) = split_platform_tool_output(raw);
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].url, "https://tmp.example/first.png");
        assert!(!text.contains(PLATFORM_TOOL_IMAGE_URLS_FIELD));
        assert!(!text.contains("tmp.example"));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["screenshot_count"],
            2
        );
    }

    #[test]
    fn malformed_page_capture_urls_are_still_removed_from_text() {
        let raw = json!({
            "status": "ok",
            "_flowpilot_image_urls": [{ "url": "data:image/png;base64,large", "media_type": "image/png" }],
        })
        .to_string();

        let (text, images) = split_platform_tool_output(raw);
        assert!(images.is_empty());
        assert!(!text.contains(PLATFORM_TOOL_IMAGE_URLS_FIELD));
        assert!(!text.contains("base64"));
    }

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
    fn side_effecting_platform_rounds_preserve_model_order_even_with_deferred_approval() {
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
