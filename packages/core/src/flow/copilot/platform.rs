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
    collections::{HashMap, HashSet},
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
    MAX_ARCHIVE_CALLS_PER_SESSION, MAX_SEARCH_CALLS_PER_SESSION, OpenUrlSessionBudget,
    WebResearchSession, normalize_public_discovery_url, run_archive_lookup_for_session,
    run_open_url_for_session, source_id_for_url,
};
use super::stream::{
    FlowScriptToolCallPreviewTracker, detailed_tool_end_frame, detailed_tool_start_frame,
    plan_step_frame, safe_tool_result_preview, tool_result_stream_status, tool_result_summary,
    tool_result_terminal_status, usage_stat_frame,
};
use super::tool_spec::{
    ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL,
    OPEN_URL_TOOL, PlatformToolSpec, RESEARCH_AGENT_TOOL, find_global_tool_spec,
    global_assistant_tool_specs, public_web_tool_specs, resolve_tool_effect, spec_arg_str,
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
const MAX_ARCHIVE_CALLS_PER_ROUND: usize = 2;
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

    /// Take any user instructions that arrived since the last round and should be folded into the
    /// conversation now. The host owns the queue (an in-process registry on the desktop, a table
    /// row on the server); the loop just drains it at the one point where appending a user turn is
    /// protocol-valid. Hosts without steering keep the default and never see a behaviour change.
    async fn drain_steering(&self) -> Vec<String> {
        Vec::new()
    }

    /// True once the run has been cancelled. Checked between rounds so a stopped turn stops
    /// spending tokens immediately instead of only when the outer `select!` fires — which, with a
    /// long tool round in flight, can be minutes later.
    ///
    /// Async because the hosts answer it differently: the desktop reads an in-process token, while
    /// the server has to consult shared storage — a browser's cancel POST can land on a different
    /// instance than the one running the turn.
    async fn is_cancelled(&self) -> bool {
        false
    }
}

/// Fold instructions the user sent mid-run into the turn about to be dispatched.
///
/// Appended to the pending user message rather than pushed as a separate turn: at a round boundary
/// that pending message is often a tool-result block, and providers requiring strict
/// user/assistant alternation reject a second consecutive user turn after it.
fn merge_steering_into_prompt(prompt: &mut rig::message::Message, steering: &[String]) {
    let instructions: Vec<&str> = steering
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .collect();
    if instructions.is_empty() {
        return;
    }
    let note = format!(
        "The user sent this while you were working. Treat it as part of the current request and adjust course now:\n{}",
        instructions.join("\n")
    );
    if let rig::message::Message::User { content } = prompt {
        content.push(UserContent::text(note));
    }
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

fn is_editing_flowpilot_board_call(name: &str, arguments: &Value) -> bool {
    name == "flowpilot_board" && spec_arg_str(arguments, "mode", "mode") != "explain"
}

fn target_values_may_match(
    left: &Value,
    right: &Value,
    snake_case: &str,
    camel_case: &str,
) -> bool {
    let left = spec_arg_str(left, snake_case, camel_case).trim();
    let right = spec_arg_str(right, snake_case, camel_case).trim();
    left.is_empty() || right.is_empty() || left == right
}

fn calls_may_share_app(left: &Value, right: &Value) -> bool {
    target_values_may_match(left, right, "app_id", "appId")
}

fn is_related_editing_board_call(
    dependent_arguments: &Value,
    candidate_name: &str,
    candidate_arguments: &Value,
) -> bool {
    is_editing_flowpilot_board_call(candidate_name, candidate_arguments)
        && calls_may_share_app(dependent_arguments, candidate_arguments)
        && target_values_may_match(
            dependent_arguments,
            candidate_arguments,
            "board_id",
            "boardId",
        )
}

fn is_creating_flowpilot_widget_call(name: &str, arguments: &Value) -> bool {
    if name != "flowpilot_widget" {
        return false;
    }
    match spec_arg_str(arguments, "mode", "mode").trim() {
        "edit" => false,
        "create" => true,
        _ => [
            ("app_id", "appId"),
            ("board_id", "boardId"),
            ("page_id", "pageId"),
            ("page_name", "pageName"),
            ("route", "route"),
        ]
        .iter()
        .any(|(snake, camel)| !spec_arg_str(arguments, snake, camel).trim().is_empty()),
    }
}

fn is_related_widget_create_call(
    dependent_arguments: &Value,
    candidate_name: &str,
    candidate_arguments: &Value,
) -> bool {
    if !is_creating_flowpilot_widget_call(candidate_name, candidate_arguments)
        || !calls_may_share_app(dependent_arguments, candidate_arguments)
    {
        return false;
    }
    let dependent_page = spec_arg_str(dependent_arguments, "page_id", "pageId").trim();
    let candidate_page = spec_arg_str(candidate_arguments, "page_id", "pageId").trim();
    if !dependent_page.is_empty() && !candidate_page.is_empty() {
        return dependent_page == candidate_page;
    }
    let dependent_route = spec_arg_str(dependent_arguments, "route", "route").trim();
    let candidate_route = spec_arg_str(candidate_arguments, "route", "route").trim();
    dependent_route.is_empty() || candidate_route.is_empty() || dependent_route == candidate_route
}

/// The serialization lane a call belongs to, or `None` when it may run alongside anything.
///
/// A round is scheduled by lane rather than as one all-or-nothing decision: calls sharing a lane
/// run sequentially in the model's declared order, and different lanes run concurrently. This is
/// what makes a plan's fan-out real — the workflow, its page and its tables touch disjoint state
/// (FlowScript drafts, A2UI surfaces, tables/overlays), so building them at the same time is safe,
/// while two edits of the SAME board still cannot interleave.
///
/// Lanes are deliberately coarser than the underlying gates: the per-board nested-run semaphore and
/// the frontend board-edit lock remain the authority on mutual exclusion. This only decides what the
/// round is allowed to *attempt* concurrently, so an unrecognised or unresolved target falls back to
/// a broader lane rather than a narrower one.
fn platform_tool_serialization_lane(name: &str, arguments: &Value) -> Option<String> {
    let arg = |snake: &str, camel: &str| {
        let value = spec_arg_str(arguments, snake, camel).trim();
        (!value.is_empty()).then(|| value.to_string())
    };
    let app = || arg("app_id", "appId").unwrap_or_else(|| "*".to_string());

    // The three authoring specialists are laned by the state they OWN, not by their approval spec.
    // `data_studio_agent` in particular needs no approval and so reads as read-only to the effect
    // classifier, yet two data builds on one app absolutely do contend.
    match name {
        "flowpilot_board" if is_editing_flowpilot_board_call(name, arguments) => {
            // An unresolved board target may create or adopt the app's first board, so it shares
            // the app's board lane; a named target only contends with edits of that same board.
            return Some(match arg("board_id", "boardId") {
                Some(board) => format!("board:{}:{board}", app()),
                None => format!("board:{}:unresolved", app()),
            });
        }
        // Page authoring is keyed by the page it targets, so building several distinct pages of one
        // app in a single wavefront does not queue.
        "flowpilot_widget" => {
            return Some(
                match arg("page_id", "pageId")
                    .or_else(|| arg("route", "route"))
                    .or_else(|| arg("page_name", "pageName"))
                {
                    Some(page) => format!("widget:{}:{page}", app()),
                    None => format!("widget:{}:unresolved", app()),
                },
            );
        }
        // Tables and overlays are app-scoped state.
        "data_studio_agent" => return Some(format!("data:{}", app())),
        _ => {}
    }

    if !platform_tool_requires_ordered_execution(name, arguments) {
        return None;
    }
    // Everything else that mutates keeps the historical guarantee: one shared lane, executed in the
    // exact order the model declared it. These calls are cheap relative to a delegated build, so
    // serializing them costs nothing and keeps unknown side-effect policy safe by default.
    Some("sequential".to_string())
}

fn is_workflow_event_upsert_call(name: &str, arguments: &Value) -> bool {
    name == "upsert_event"
        && spec_arg_str(arguments, "page_id", "pageId")
            .trim()
            .is_empty()
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

fn is_page_event_upsert_call(name: &str, arguments: &Value) -> bool {
    name == "upsert_event"
        && !spec_arg_str(arguments, "page_id", "pageId")
            .trim()
            .is_empty()
}

/// Page lifecycle wiring and page Event registration need identities that only the completed
/// authoring tools can make authoritative. Lanes can order calls, but they cannot repair guessed
/// ids that the model authored before seeing those results, so defer the dependent call to the
/// next model round instead of racing it against page/board persistence.
fn same_round_page_dependency_guard_result(
    name: &str,
    arguments: &Value,
    round_has_editing_board_call: bool,
    round_has_widget_call: bool,
) -> Option<String> {
    let sets_page_lifecycle = name == "set_page_load_event";
    let waits_for_page = round_has_widget_call
        && (sets_page_lifecycle || is_page_event_upsert_call(name, arguments));
    let waits_for_board = round_has_editing_board_call && sets_page_lifecycle;
    if !waits_for_page && !waits_for_board {
        return None;
    }

    let pending = match (waits_for_page, waits_for_board) {
        (true, true) => "flowpilot_widget and flowpilot_board",
        (true, false) => "flowpilot_widget",
        (false, true) => "flowpilot_board",
        (false, false) => unreachable!(),
    };
    Some(
        json!({
            "status": "error",
            "code": "page_authoring_dependency_pending",
            "retryable": true,
            "next_action": "wait_for_page_and_board_authoring_results_then_retry",
            "message": format!(
                "{name} cannot run in the same assistant round as {pending}. Wait for authoring to succeed, use the exact persisted page_id, board_id, and event node ids from those results, then retry in the next assistant round. No page lifecycle or Event registration was changed."
            )
        })
        .to_string(),
    )
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

        let tool_definitions: Vec<_> = platform_loop_tool_specs(memory.is_some())
            .into_iter()
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
        // Whether the turn ended with a real closing answer, or with an explicit notice explaining
        // that it did not. `full_response` accumulates every round's narration, so it can no longer
        // stand in for "the model actually synthesized an answer".
        let mut answer_closed = false;
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
            // A stopped run must stop spending immediately. The outer `select!` only fires between
            // awaits it owns, so with a long tool round in flight it can be minutes late.
            if bridge.is_cancelled().await {
                return Err(flow_like_types::anyhow!("Run cancelled"));
            }
            // Round boundaries are the one place a user turn can be added without breaking the
            // assistant → tool-result ordering providers require.
            merge_steering_into_prompt(&mut current_prompt, &bridge.drain_steering().await);
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

            // Every round's streamed text is part of the reply the user already watched — keep the
            // final message identical to the live stream. Text from rounds that also called tools
            // must not silently drop out of the persisted/final response.
            full_response.push_str(&iteration_text);
            // A round that ends without tool calls IS the synthesis.
            let round_produced_text = !iteration_text.trim().is_empty();

            if tool_calls.is_empty() {
                answer_closed |= round_produced_text;
                break;
            }

            // A provider should not emit calls for tools that were not advertised. If it does on
            // the reserved synthesis turn, stop safely instead of executing an unbudgeted action.
            if !tools_enabled {
                if round_produced_text {
                    answer_closed = true;
                } else {
                    if !full_response.is_empty() && !full_response.ends_with('\n') {
                        full_response.push_str("\n\n");
                    }
                    full_response.push_str(
                        "The research tools completed, but the model did not produce a final synthesis within the tool budget.",
                    );
                    // The turn is now explicitly closed out; the terminal guard must not add a
                    // second notice on top of this one.
                    answer_closed = true;
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

            // Group the round into serialization lanes, then run the lanes concurrently. A lane
            // preserves the model's declared order internally; laneless (read-only) calls each get
            // their own lane so they never wait on anything.
            let mut lanes: Vec<Vec<usize>> = Vec::new();
            let mut lane_index: HashMap<String, usize> = HashMap::new();
            for (index, tool_call) in tool_calls.iter().enumerate() {
                match platform_tool_serialization_lane(
                    &tool_call.function.name,
                    &tool_call.function.arguments,
                ) {
                    Some(lane) => match lane_index.get(&lane) {
                        Some(&existing) => lanes[existing].push(index),
                        None => {
                            lane_index.insert(lane, lanes.len());
                            lanes.push(vec![index]);
                        }
                    },
                    None => lanes.push(vec![index]),
                }
            }

            let lane_futures: Vec<_> = lanes
                .into_iter()
                .map(|lane| {
                    let bridge = bridge.clone();
                    let memory = memory.clone();
                    let web_research_session = web_research_session.clone();
                    let calls: Vec<_> = lane
                        .into_iter()
                        .map(|index| {
                            let raw_arguments = &tool_calls[index].function.arguments;
                            let round_has_related_editing_board_call =
                                tool_calls.iter().any(|candidate| {
                                    is_related_editing_board_call(
                                        raw_arguments,
                                        &candidate.function.name,
                                        &candidate.function.arguments,
                                    )
                                });
                            let round_has_related_widget_create_call =
                                tool_calls.iter().any(|candidate| {
                                    is_related_widget_create_call(
                                        raw_arguments,
                                        &candidate.function.name,
                                        &candidate.function.arguments,
                                    )
                                });
                            (
                                index,
                                tool_calls[index].id.clone(),
                                tool_calls[index].function.name.clone(),
                                tool_calls[index].function.arguments.clone(),
                                prepared_arguments[index].clone(),
                                round_has_related_editing_board_call,
                                round_has_related_widget_create_call,
                            )
                        })
                        .collect();
                    async move {
                        let mut lane_results = Vec::with_capacity(calls.len());
                        for (
                            index,
                            id,
                            name,
                            raw_arguments,
                            prepared,
                            round_has_related_editing_board_call,
                            round_has_related_widget_create_call,
                        ) in calls
                        {
                            if let Some(output) = same_round_page_dependency_guard_result(
                                &name,
                                &raw_arguments,
                                round_has_related_editing_board_call,
                                round_has_related_widget_create_call,
                            ) {
                                lane_results.push((index, (id, name, output, Vec::new())));
                                continue;
                            }
                            if let Some(output) = same_round_workflow_event_guard_result(
                                &name,
                                &raw_arguments,
                                round_has_related_editing_board_call,
                            ) {
                                lane_results.push((index, (id, name, output, Vec::new())));
                                continue;
                            }
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
                            lane_results.push((index, (id, name, output, images)));
                        }
                        lane_results
                    }
                })
                .collect();

            // Reassemble in the model's declared call order: a tool result block must line up with
            // the tool_call ids of the assistant message that produced it, whatever order the lanes
            // happened to finish in.
            let mut ordered: Vec<Option<(String, String, String, Vec<PlatformToolImageUrl>)>> =
                (0..tool_calls.len()).map(|_| None).collect();
            for lane_results in futures::future::join_all(lane_futures).await {
                for (index, result) in lane_results {
                    ordered[index] = Some(result);
                }
            }
            let tool_results: Vec<(String, String, String, Vec<PlatformToolImageUrl>)> =
                ordered.into_iter().flatten().collect();

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

        // Ending without a closing answer needs saying even when mid-run narration left
        // `full_response` non-empty — that narration is not a synthesis. The notice is appended
        // rather than substituted so the text the user already watched stream is preserved.
        if !answer_closed {
            if full_response.trim().is_empty() {
                let fallback = "The research run ended without a usable final synthesis. No additional web action was taken; please retry or narrow the question.".to_string();
                if let Some(callback) = &on_token {
                    callback(fallback.clone());
                }
                full_response = fallback;
            } else {
                let fallback = "\n\nThe run ended without a final synthesis — the notes above are mid-run narration, not a complete answer. Please retry or narrow the question.";
                if let Some(callback) = &on_token {
                    callback(fallback.to_string());
                }
                full_response.push_str(fallback);
            }
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

/// Tools advertised to the rig/Bits platform loop.
///
/// The public-web tools moved off the shared orchestrator set and onto the Research scope, which
/// only the tool-driven backends can host. This loop cannot spawn a nested scope, and it already
/// implements the search/open/archive handlers inline — so it keeps them directly; dropping them
/// here would remove web research from the Bits backend outright rather than relocating it. For the
/// same reason `research_agent` must NOT be advertised: delegating it would open a Research scope
/// this backend cannot run, and the host's capability notice would come back to the orchestrator
/// looking exactly like research findings. The prompt variant paired with this set is
/// [`WebResearchCapability::Inline`](super::assistant::WebResearchCapability).
fn platform_loop_tool_specs(memory_enabled: bool) -> Vec<PlatformToolSpec> {
    global_assistant_tool_specs(memory_enabled)
        .into_iter()
        .filter(|spec| spec.name != RESEARCH_AGENT_TOOL)
        .chain(public_web_tool_specs())
        .collect()
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

    /// This loop researches the web itself. Advertising the delegating specialist as well makes the
    /// model hand every web question to a Research scope this backend cannot start.
    #[test]
    fn platform_loop_holds_web_tools_and_hides_the_research_specialist() {
        let names: Vec<&str> = platform_loop_tool_specs(true)
            .iter()
            .map(|spec| spec.name)
            .collect();
        assert!(!names.contains(&RESEARCH_AGENT_TOOL));
        assert!(names.contains(&INTERNET_SEARCH_TOOL));
        assert!(names.contains(&OPEN_URL_TOOL));
        assert!(names.contains(&ARCHIVE_LOOKUP_TOOL));
        assert!(names.contains(&MEMORY_SEARCH_TOOL));
    }

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

        assert!(platform_tool_serialization_lane("list_apps", &list_args).is_none());
        assert!(
            platform_tool_serialization_lane("describe_app_interface", &describe_args).is_none()
        );
    }

    #[test]
    fn side_effecting_platform_calls_keep_model_order_within_their_lane() {
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

        assert!(platform_tool_serialization_lane("list_apps", &list_args).is_none());
        assert_eq!(
            platform_tool_serialization_lane("flowpilot_board", &board_args),
            Some("board:app:unresolved".to_string())
        );
        // Generic mutating tools keep the historical single-file lane.
        assert_eq!(
            platform_tool_serialization_lane("upsert_event", &event_args),
            Some("sequential".to_string())
        );
    }

    #[test]
    fn authoring_lanes_let_one_feature_build_its_parts_at_once() {
        let board =
            json!({ "app_id": "app", "board_id": "b1", "instruction": "Build the workflow" });
        let widget =
            json!({ "app_id": "app", "route": "/dashboard", "instruction": "Build the page" });
        let data = json!({ "app_id": "app", "instruction": "Create the tables" });

        let board_lane = platform_tool_serialization_lane("flowpilot_board", &board);
        let widget_lane = platform_tool_serialization_lane("flowpilot_widget", &widget);
        let data_lane = platform_tool_serialization_lane("data_studio_agent", &data);

        assert_eq!(board_lane, Some("board:app:b1".to_string()));
        assert_eq!(widget_lane, Some("widget:app:/dashboard".to_string()));
        assert_eq!(data_lane, Some("data:app".to_string()));
        assert_ne!(board_lane, widget_lane);
        assert_ne!(board_lane, data_lane);
        assert_ne!(widget_lane, data_lane);
    }

    #[test]
    fn board_lane_separates_distinct_boards_but_not_same_board_edits() {
        let first = json!({ "app_id": "app", "board_id": "b1", "instruction": "Ingest" });
        let second = json!({ "app_id": "app", "board_id": "b2", "instruction": "Report" });
        let same = json!({ "app_id": "app", "board_id": "b1", "instruction": "Extend ingest" });

        assert_ne!(
            platform_tool_serialization_lane("flowpilot_board", &first),
            platform_tool_serialization_lane("flowpilot_board", &second)
        );
        assert_eq!(
            platform_tool_serialization_lane("flowpilot_board", &first),
            platform_tool_serialization_lane("flowpilot_board", &same)
        );
        // Two board calls with no resolved target may both adopt or create the app's first board,
        // so they must not be allowed to race.
        let unresolved_a = json!({ "app_id": "app", "instruction": "Ingest" });
        let unresolved_b = json!({ "app_id": "app", "instruction": "Report" });
        assert_eq!(
            platform_tool_serialization_lane("flowpilot_board", &unresolved_a),
            platform_tool_serialization_lane("flowpilot_board", &unresolved_b)
        );
        // Different apps never contend.
        let other_app = json!({ "app_id": "other", "instruction": "Ingest" });
        assert_ne!(
            platform_tool_serialization_lane("flowpilot_board", &unresolved_a),
            platform_tool_serialization_lane("flowpilot_board", &other_app)
        );
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
    fn mixed_page_and_workflow_target_is_not_deferred_as_a_workflow_event() {
        let invalid_mixed_args = json!({
            "app_id": "app",
            "name": "Dashboard",
            "page_id": "page",
            "board_id": "board",
            "node_id": "entry",
            "route": "/dashboard",
        });

        assert!(!is_workflow_event_upsert_call(
            "upsert_event",
            &invalid_mixed_args,
        ));
        assert!(
            same_round_workflow_event_guard_result("upsert_event", &invalid_mixed_args, true,)
                .is_none()
        );
    }

    #[test]
    fn page_event_upsert_is_deferred_when_page_is_authored_in_same_round() {
        let page_event_args = json!({
            "app_id": "app",
            "name": "Dashboard",
            "page_id": "guessed-page",
            "route": "/dashboard",
        });
        let output =
            same_round_page_dependency_guard_result("upsert_event", &page_event_args, false, true)
                .expect("page Event must wait for the authoritative page result");
        let payload: Value = serde_json::from_str(&output).expect("structured guard result");

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "page_authoring_dependency_pending");
        assert_eq!(payload["retryable"], true);
    }

    #[test]
    fn page_lifecycle_is_deferred_for_same_round_page_or_board_authoring() {
        let lifecycle_args = json!({
            "app_id": "app",
            "page_id": "page",
            "board_id": "board",
            "on_load_event_id": "entry",
        });

        for (has_board, has_widget) in [(true, false), (false, true), (true, true)] {
            let output = same_round_page_dependency_guard_result(
                "set_page_load_event",
                &lifecycle_args,
                has_board,
                has_widget,
            )
            .expect("lifecycle wiring must wait for same-round authoring");
            let payload: Value = serde_json::from_str(&output).expect("structured guard result");
            assert_eq!(payload["code"], "page_authoring_dependency_pending");
            assert_eq!(
                payload["next_action"],
                "wait_for_page_and_board_authoring_results_then_retry"
            );
        }
    }

    #[test]
    fn page_dependencies_are_allowed_after_authoring_results_exist() {
        let lifecycle_args = json!({
            "app_id": "app",
            "page_id": "page",
            "board_id": "board",
            "on_load_event_id": "entry",
        });
        assert!(
            same_round_page_dependency_guard_result(
                "set_page_load_event",
                &lifecycle_args,
                false,
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn dependency_matching_ignores_unrelated_apps_pages_and_widget_edits() {
        let dependent = json!({
            "app_id": "app-a",
            "page_id": "page-a",
            "board_id": "board-a",
        });
        assert!(!is_related_widget_create_call(
            &dependent,
            "flowpilot_widget",
            &json!({
                "mode": "create",
                "app_id": "app-b",
                "page_id": "page-a",
            }),
        ));
        assert!(!is_related_widget_create_call(
            &dependent,
            "flowpilot_widget",
            &json!({
                "mode": "create",
                "app_id": "app-a",
                "page_id": "page-b",
            }),
        ));
        assert!(!is_related_widget_create_call(
            &dependent,
            "flowpilot_widget",
            &json!({
                "mode": "edit",
                "app_id": "app-a",
                "page_id": "page-a",
            }),
        ));
        assert!(!is_related_editing_board_call(
            &dependent,
            "flowpilot_board",
            &json!({
                "mode": "edit",
                "app_id": "app-a",
                "board_id": "board-b",
            }),
        ));
    }

    #[test]
    fn dependency_matching_keeps_unresolved_related_targets_conservative() {
        let dependent = json!({ "app_id": "app-a", "page_id": "page-a" });
        assert!(is_related_widget_create_call(
            &dependent,
            "flowpilot_widget",
            &json!({ "mode": "create", "app_id": "app-a" }),
        ));
        assert!(is_related_editing_board_call(
            &dependent,
            "flowpilot_board",
            &json!({ "mode": "edit", "app_id": "app-a" }),
        ));
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
    fn terminal_bridge_statuses_distinguish_failures_from_validation_progress() {
        for status in [
            "error",
            "failed",
            "timeout",
            "timed_out",
            "denied",
            "cancelled",
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
        for status in [
            "ok",
            "done",
            "queued",
            "applied",
            "completed",
            // The validator call completed and its candidate lifecycle carries the
            // unresolved/repaired state. An explicit provider is_error still fails.
            "validation_errors",
            "draft_needs_repair",
        ] {
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
