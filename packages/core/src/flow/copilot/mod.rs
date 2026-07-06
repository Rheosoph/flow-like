//! Flow Copilot - AI-powered graph editing assistant
//!
//! This module provides the Copilot struct which enables natural language
//! interaction with flow graphs, supporting both explanation and modification.

mod context;
mod declarations;
pub mod memory;
pub mod platform;
mod provider;
mod search;
pub mod stream;
pub mod tool_spec;
mod tools;
mod types;
mod validation;

pub use context::{
    EdgeContext, GraphContext, LayerContext, NodeContext, PinContext, prepare_context,
};
pub use provider::{CatalogProvider, node_to_metadata, pin_to_metadata};
/// Re-export of the rig tool trait so non-rig adapter crates can bound on it (e.g. to derive
/// backend-native tools from the shared rig definitions) without depending on rig directly.
pub use rig::tool::Tool as RigTool;
pub use search::{
    SearchQueryAnalysis, analyze_search_query, enrich_node_metadata, render_catalog_search_results,
    score_catalog_metadata, search_result_hint_lines,
};
pub use tools::{
    CatalogTool, EditFlowScriptArgs, EditFlowScriptTool, EmitCommandsArgs, EmitCommandsTool,
    FilterCategoryArgs, FilterCategoryTool, FindConnectableNodesArgs, FindConnectableNodesTool,
    GetCurrentFlowScriptArgs, GetCurrentFlowScriptTool, GetDeclarationsArgs, GetDeclarationsTool,
    GetNodeDetailsArgs, GetNodeDetailsTool, GetUnconfiguredNodesTool, ListBoardNodesTool,
    QueryLogsArgs, QueryLogsTool, SearchArgs, SearchByPinArgs, SearchByPinTool,
    SearchTemplatesArgs, SearchTemplatesTool, ThinkingArgs, board_has_no_nodes,
    build_find_connectable_nodes_output, build_list_board_nodes_output, build_node_details_output,
    build_unconfigured_nodes_output, get_tool_description, render_edit_flowscript_result,
    tool_definition_parts,
};
pub use types::{
    AgentType, BoardCommand, ChatImage, ChatMessage, ChatRole, Connection, CopilotResponse, Edge,
    NodeMetadata, NodePosition, PinMetadata, PlaceholderPinDef, PlanStep, PlanStepStatus,
    RunContext, StreamEvent, Suggestion, TemplateInfo,
};
pub use validation::{EmitValidationOutcome, ValidationIssue, validate_emit_commands};

use std::sync::Arc;

use flow_like_model_provider::llm::CompletionClientDyn;
use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::Completion,
    message::{
        AssistantContent, DocumentSourceKind, Image, ImageDetail, ImageMediaType,
        ToolResult as RigToolResult, ToolResultContent, UserContent,
    },
    streaming::StreamedAssistantContent,
    tools::ThinkTool,
};
use serde_json::json;

use crate::app::App;
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters, Metadata};
use crate::flow::board::Board;
use crate::profile::Profile;
use crate::state::FlowLikeState;
use flow_like_model_provider::provider::ModelProvider;

// Note: Tool args types are re-exported publicly from `pub use tools::{ ... }` above

/// The main Copilot struct that provides AI-powered graph editing
pub struct Copilot {
    state: Arc<FlowLikeState>,
    catalog_provider: Arc<dyn CatalogProvider>,
    profile: Option<Arc<Profile>>,
    templates: Vec<TemplateInfo>,
    /// Current template ID if editing a template (prioritized in search)
    current_template_id: Option<String>,
}

impl Copilot {
    /// Create a new Copilot - always loads templates from profile
    pub async fn new(
        state: Arc<FlowLikeState>,
        catalog_provider: Arc<dyn CatalogProvider>,
        profile: Option<Arc<Profile>>,
        current_template_id: Option<String>,
    ) -> Result<Self> {
        let templates = if let Some(ref profile) = profile {
            Self::load_templates_from_profile(&state, profile)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self {
            state,
            catalog_provider,
            profile,
            templates,
            current_template_id,
        })
    }

    /// Load all templates from the user's profile apps
    async fn load_templates_from_profile(
        state: &Arc<FlowLikeState>,
        profile: &Profile,
    ) -> Result<Vec<TemplateInfo>> {
        let mut templates = Vec::new();

        let app_ids: Vec<String> = profile
            .apps
            .as_ref()
            .map(|apps| apps.iter().map(|a| a.app_id.clone()).collect())
            .unwrap_or_default();

        for app_id in app_ids {
            // Try to load the app
            let app = match App::load(app_id.clone(), state.clone()).await {
                Ok(app) => app,
                Err(_) => continue,
            };

            // Load templates from this app
            for template_id in &app.templates {
                let template_info = match Self::load_template_info(&app, template_id).await {
                    Ok(info) => info,
                    Err(_) => continue,
                };
                templates.push(template_info);
            }
        }

        Ok(templates)
    }

    /// Load template info (metadata + structure analysis)
    async fn load_template_info(app: &App, template_id: &str) -> Result<TemplateInfo> {
        // Get template metadata
        let meta = app
            .get_template_meta(template_id, None)
            .await
            .unwrap_or_else(|_| Metadata::default());

        // Load the template board to analyze its structure
        let template_board = app.open_template(template_id.to_string(), None).await?;

        // Extract unique node types used in this template
        let node_types: Vec<String> = template_board
            .nodes
            .values()
            .map(|n| n.name.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .take(10) // Limit to avoid bloating context
            .collect();

        Ok(TemplateInfo {
            id: template_id.to_string(),
            app_id: app.id.clone(),
            name: meta.name,
            description: meta.description,
            tags: meta.tags,
            node_count: template_board.nodes.len(),
            node_types,
        })
    }

    /// Main entry point - unified agent that can both explain and modify
    pub async fn chat<F>(
        &self,
        board: &Board,
        selected_node_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<ChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        run_context: Option<RunContext>,
        on_token: Option<F>,
    ) -> Result<CopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        println!(
            "[Copilot::chat] Starting chat with run_context: {:?}",
            run_context
        );

        let context = prepare_context(board, selected_node_ids)?;
        let context_json = flow_like_types::json::to_string_pretty(&context)?;

        // Render the board as FlowScript (with stable `//@n:<id>` anchors) so the agent edits the
        // text surface by default and reconcile can match identities on apply.
        let flowscript = crate::flow::ast::board_to_flowscript(
            board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        // Only include node type names (not full paths) for context efficiency
        let available_nodes = self.catalog_provider.get_all_nodes().await;
        let node_count = available_nodes.len();

        let (model_name, completion_client) = self.get_model(model_id, token).await?;

        // Build a compact system prompt
        let system_prompt = Self::build_system_prompt(
            &context_json,
            &flowscript,
            node_count,
            !self.templates.is_empty(),
            run_context.is_some(),
        );

        let graph_context = Arc::new(context.clone());
        let board_for_tools = Arc::new(board.clone());

        let mut agent_builder = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .tool(ThinkTool)
            .tool(GetNodeDetailsTool {
                graph_context: graph_context.clone(),
            })
            .tool(ListBoardNodesTool {
                graph_context: graph_context.clone(),
            })
            .tool(GetUnconfiguredNodesTool {
                graph_context: graph_context.clone(),
            })
            .tool(FindConnectableNodesTool {
                provider: self.catalog_provider.clone(),
                graph_context: graph_context.clone(),
            })
            .tool(EmitCommandsTool)
            .tool(CatalogTool {
                provider: self.catalog_provider.clone(),
            })
            .tool(GetCurrentFlowScriptTool {
                board: board_for_tools.clone(),
            })
            .tool(GetDeclarationsTool {
                provider: self.catalog_provider.clone(),
            })
            .tool(EditFlowScriptTool {
                board: board_for_tools.clone(),
                provider: self.catalog_provider.clone(),
            })
            .tool(SearchByPinTool {
                provider: self.catalog_provider.clone(),
            })
            .tool(FilterCategoryTool {
                provider: self.catalog_provider.clone(),
            });

        // Only add templates tool if we have templates
        if !self.templates.is_empty() {
            agent_builder = agent_builder.tool(SearchTemplatesTool {
                templates: self.templates.clone(),
                current_template_id: self.current_template_id.clone(),
            });
        }

        // Add logs query tool if run context is provided
        if run_context.is_some() {
            println!(
                "[Copilot::chat] Adding QueryLogsTool with run_context: {:?}",
                run_context
            );
            agent_builder = agent_builder.tool(QueryLogsTool {
                state: self.state.clone(),
                run_context: run_context.clone(),
            });
        } else {
            println!("[Copilot::chat] No run_context provided, QueryLogsTool NOT added");
        }

        let agent = agent_builder.build();

        let prompt = user_prompt.clone();

        // Helper to convert media type string to ImageMediaType
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
            text: prompt.clone(),
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
                    text: prompt.clone(),
                    additional_params: None,
                }))
            }),
        };

        // Convert chat history to rig message format (including images)
        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| {
                match msg.role {
                    ChatRole::User => {
                        let mut contents: Vec<UserContent> =
                            vec![UserContent::Text(rig::message::Text {
                                text: msg.content.clone(),
                                additional_params: None,
                            })];

                        // Add images if present
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

                        // Use many() which returns Result, handle the error case
                        match OneOrMany::many(contents) {
                            Ok(content) => Some(rig::message::Message::User { content }),
                            Err(_) => None, // Empty contents, skip this message
                        }
                    }
                    ChatRole::Assistant => Some(rig::message::Message::Assistant {
                        id: None,
                        content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                            text: msg.content.clone(),
                            additional_params: None,
                        })),
                    }),
                }
            })
            .collect();

        let mut full_response = String::new();
        let mut all_commands: Vec<BoardCommand> = Vec::new();
        let max_iterations = 10u64;
        let max_discovery_rounds_before_emit = 4u64;
        let force_emit_instruction = "You have enough context. Stop searching or planning. In your next response, call edit_flowscript with the full edited FlowScript document. Preserve all existing //@n anchors you keep. Leave allow_deletions false unless the user explicitly asked to delete existing board items. Write new workflow nodes as concrete unanchored FlowScript calls inside a function/event block using declarations from get_declarations, and let edit_flowscript translate the text into commands. Do not submit TODOs, function stubs, implementation-plan comments, lists of node names, or top-level node-call assignments. Use emit_commands only for layout-only MoveNode or non-FlowScript visual/modeling changes. If edit_flowscript returns validation errors, fix the FlowScript and call edit_flowscript again; do not answer in text instead.";
        let force_emit_escalation = "STOP analyzing. You were already instructed to submit the FlowScript and you called more read/analysis tools instead. In your NEXT response call edit_flowscript with your best complete draft — an imperfect draft that gets validated and fixed beats another analysis round. Do not call any other tool first.";
        let mut plan_step_counter = 0u32;
        let mut invalid_emit_attempts = 0u8;
        let mut discovery_rounds_without_emit = 0u64;
        let mut forced_emit_prompt_sent = false;
        let mut forced_text_retries = 0u8;
        let mut last_emit_validation: Option<String> = None;
        let mut successful_emit_message: Option<String> = None;
        let mut last_flowscript_workspace: Option<String> = None;
        let mut current_prompt = prompt_message.clone();

        for iteration in 0..max_iterations {
            // Send iteration start event
            if let Some(ref callback) = on_token {
                plan_step_counter += 1;
                callback(stream::plan_step_frame(
                    format!("iteration_{}", iteration),
                    if iteration == 0 {
                        "Analyzing request...".to_string()
                    } else {
                        "Processing tool results...".to_string()
                    },
                    PlanStepStatus::InProgress,
                    "analyze",
                ));
            }

            // Build completion request - tools are already attached via agent builder
            let request = agent
                .completion(current_prompt.clone(), current_history.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Completion error: {}", e))?;

            // Stream the response
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
                    StreamedAssistantContent::ToolCallDelta { .. } => {
                        // Deltas are accumulated into the final ToolCall
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

                        // Send reasoning as a plan step (streaming update)
                        if let Some(ref callback) = on_token {
                            // Create or update the reasoning step
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(stream::plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                    StreamedAssistantContent::Final(_) => {
                        // Mark reasoning step as completed
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            callback(stream::plan_step_frame(
                                step_id.clone(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::Completed,
                                "think",
                            ));
                        }
                        reasoning_step_id = None;
                        current_reasoning.clear();
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        current_reasoning.push_str(&reasoning);

                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(stream::plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                }
            }

            // Mark reasoning step as completed if stream ended while reasoning
            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                callback(stream::plan_step_frame(
                    step_id.clone(),
                    current_reasoning.trim().to_string(),
                    PlanStepStatus::Completed,
                    "think",
                ));
            }

            // Mark iteration analysis as complete
            if let Some(ref callback) = on_token {
                callback(stream::plan_step_frame(
                    format!("iteration_{}", iteration),
                    if iteration == 0 {
                        "Analysis complete".to_string()
                    } else {
                        "Tool results processed".to_string()
                    },
                    PlanStepStatus::Completed,
                    "analyze",
                ));
            }

            // Collect all tool calls first for parallel execution
            let tool_calls: Vec<_> = response_contents
                .iter()
                .filter_map(|content| {
                    if let AssistantContent::ToolCall(tool_call) = content {
                        Some(tool_call.clone())
                    } else {
                        None
                    }
                })
                .collect();

            let tool_calls_found = !tool_calls.is_empty();

            if tool_calls_found {
                current_history.push(current_prompt.clone());
                let command_count_before_round = all_commands.len();
                let mut emit_attempted_this_round = false;

                // Announce all tool calls starting
                let mut frame_ids: Vec<String> = Vec::new();
                for tool_call in &tool_calls {
                    plan_step_counter += 1;
                    let frame_id = if tool_call.id.is_empty() {
                        format!("step_{}", plan_step_counter)
                    } else {
                        tool_call.id.clone()
                    };
                    let step_description = get_tool_description(
                        &tool_call.function.name,
                        &tool_call.function.arguments,
                    );

                    if let Some(ref callback) = on_token {
                        callback(stream::tool_start_frame(
                            &frame_id,
                            &tool_call.function.name,
                            Some(&step_description),
                        ));
                    }

                    frame_ids.push(frame_id);
                }

                // Execute all tools in parallel
                let tool_futures: Vec<_> = tool_calls
                    .iter()
                    .map(|tool_call| {
                        let name = tool_call.function.name.clone();
                        let arguments = tool_call.function.arguments.clone();
                        let id = tool_call.id.clone();
                        let ctx = run_context.clone();
                        let graph_ctx = context.clone();
                        let board_ctx = board_for_tools.clone();
                        async move {
                            let output = self
                                .execute_tool(
                                    &name,
                                    arguments,
                                    ctx.as_ref(),
                                    &graph_ctx,
                                    &board_ctx,
                                )
                                .await;
                            (id, name, output)
                        }
                    })
                    .collect();

                let tool_results: Vec<(String, String, String)> =
                    futures::future::join_all(tool_futures).await;

                // Process results and emit completion events
                for (i, (id, name, tool_output)) in tool_results.iter().enumerate() {
                    println!(
                        "[Copilot] Tool '{}' (id={}) output length: {} chars",
                        name,
                        id,
                        tool_output.len()
                    );

                    if name == "edit_flowscript"
                        && let Some(workspace) = Self::parse_flowscript_workspace(tool_output)
                    {
                        last_flowscript_workspace = Some(workspace);
                        if let Some(ref callback) = on_token
                            && let Some(payload) =
                                Self::extract_tag_content(tool_output, "flowscript_workspace")
                        {
                            callback(format!(
                                "<flowscript_workspace>{}</flowscript_workspace>",
                                payload
                            ));
                        }
                    }

                    // Parse commands from emit_commands tool output
                    if name == "emit_commands" || name == "edit_flowscript" {
                        emit_attempted_this_round = true;
                        let parsed = Self::parse_commands(tool_output);
                        println!("[Copilot] emit_commands parsed {} commands:", parsed.len());
                        for (idx, cmd) in parsed.iter().enumerate() {
                            println!("[Copilot]   [{}] {:?}", idx, cmd);
                        }

                        if parsed.is_empty() {
                            invalid_emit_attempts = invalid_emit_attempts.saturating_add(1);
                            last_emit_validation = Some(tool_output.clone());
                        } else {
                            invalid_emit_attempts = 0;
                            last_emit_validation = None;

                            // Deduplicate: only add commands that don't already exist
                            for cmd in parsed {
                                let is_duplicate = all_commands
                                    .iter()
                                    .any(|existing| Self::commands_are_duplicate(existing, &cmd));
                                if !is_duplicate {
                                    all_commands.push(cmd);
                                } else {
                                    println!("[Copilot] Skipping duplicate command");
                                }
                            }

                            println!(
                                "[Copilot] all_commands now has {} total commands (after dedup)",
                                all_commands.len()
                            );

                            let cleaned = Self::clean_message(tool_output);
                            let cleaned = Self::clean_validation_message(&cleaned);
                            if !cleaned.is_empty() {
                                successful_emit_message = Some(cleaned);
                            }
                        }
                    }

                    // Emit tool completion
                    if let (Some(callback), Some(frame_id)) = (&on_token, frame_ids.get(i)) {
                        callback(stream::tool_end_frame(frame_id, name, "done"));
                    }
                }

                let commands_added_this_round = all_commands.len() > command_count_before_round;
                if commands_added_this_round {
                    break;
                }

                // Re-arm the force prompt on EVERY discovery round past the budget: weaker
                // models routinely ignore a single nudge and keep calling read tools until
                // the iteration cap, ending the run without ever submitting an edit.
                let force_emit_next = if emit_attempted_this_round {
                    discovery_rounds_without_emit = 0;
                    false
                } else {
                    discovery_rounds_without_emit = discovery_rounds_without_emit.saturating_add(1);
                    all_commands.is_empty()
                        && discovery_rounds_without_emit >= max_discovery_rounds_before_emit
                };

                // Add assistant message with tool calls to history
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

                // Add all tool results to history as a single User message
                // This is required for Gemini API which expects tool results to immediately follow
                // the assistant's tool call message in a single message. We use that
                // combined tool-result message as the prompt for the next turn.
                if !tool_results.is_empty() {
                    let mut tool_result_contents: Vec<UserContent> = tool_results
                        .iter()
                        .map(|(tool_id, _tool_name, tool_output)| {
                            UserContent::ToolResult(RigToolResult {
                                id: tool_id.clone(),
                                call_id: None,
                                content: OneOrMany::one(ToolResultContent::text(
                                    tool_output.clone(),
                                )),
                            })
                        })
                        .collect();

                    if force_emit_next {
                        // Escalate when the first instruction was ignored.
                        let text = if forced_emit_prompt_sent {
                            force_emit_escalation.to_string()
                        } else {
                            force_emit_instruction.to_string()
                        };
                        forced_emit_prompt_sent = true;
                        tool_result_contents.push(UserContent::Text(rig::message::Text {
                            text,
                            additional_params: None,
                        }));
                    }

                    let combined_tool_results = if tool_result_contents.len() == 1 {
                        OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
                    } else {
                        OneOrMany::many(tool_result_contents)
                            .expect("tool_result_contents should have at least 2 elements")
                    };

                    let tool_result_msg = rig::message::Message::User {
                        content: combined_tool_results,
                    };
                    current_prompt = tool_result_msg;
                }

                if invalid_emit_attempts >= 3 {
                    println!(
                        "[Copilot] Stopping after {} invalid emit_commands attempts",
                        invalid_emit_attempts
                    );
                    break;
                }
            } else {
                // Text-only round without an edit: push back up to twice — a single push is
                // not enough for models that reply with a plan instead of calling tools.
                if all_commands.is_empty()
                    && forced_text_retries < 2
                    && iteration + 1 < max_iterations
                {
                    let text = if forced_emit_prompt_sent || forced_text_retries > 0 {
                        force_emit_escalation.to_string()
                    } else {
                        force_emit_instruction.to_string()
                    };
                    forced_text_retries += 1;
                    forced_emit_prompt_sent = true;
                    current_history.push(current_prompt.clone());
                    current_history.push(rig::message::Message::Assistant {
                        id: None,
                        content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                            OneOrMany::one(AssistantContent::Text(rig::message::Text {
                                text: iteration_text.clone(),
                                additional_params: None,
                            }))
                        }),
                    });
                    current_prompt = rig::message::Message::User {
                        content: OneOrMany::one(UserContent::Text(rig::message::Text {
                            text,
                            additional_params: None,
                        })),
                    };
                    continue;
                }

                // No tool calls, we're done
                full_response.push_str(&iteration_text);
                break;
            }

            // Continue to next iteration (agent will see tool results and continue)
            if iteration == max_iterations - 1 {
                break;
            }
        }

        let has_commands = !all_commands.is_empty();
        println!(
            "[Copilot] Final response: {} total commands, agent_type={:?}",
            all_commands.len(),
            if has_commands {
                AgentType::Edit
            } else {
                AgentType::Explain
            }
        );

        // Log the serialized response for debugging
        let cleaned_message = Self::clean_message(&full_response);
        let final_message = if cleaned_message.is_empty() {
            if has_commands {
                successful_emit_message
                    .unwrap_or_else(|| "I queued workflow changes for review.".to_string())
            } else {
                last_emit_validation
                    .as_deref()
                    .map(|message| Self::clean_validation_message(&Self::clean_message(message)))
                    .unwrap_or_default()
            }
        } else {
            cleaned_message
        };

        let response = CopilotResponse {
            agent_type: if has_commands {
                AgentType::Edit
            } else {
                AgentType::Explain
            },
            message: final_message,
            commands: all_commands,
            suggestions: vec![],
            flowscript_workspace: last_flowscript_workspace,
        };

        if let Ok(json) = serde_json::to_string(&response) {
            println!("[Copilot] Response JSON length: {} chars", json.len());
            if !response.commands.is_empty() {
                println!(
                    "[Copilot] First command serialized: {:?}",
                    serde_json::to_string(&response.commands[0])
                );
            }
        }

        Ok(response)
    }

    /// Build a compact system prompt to reduce context size
    fn build_system_prompt(
        context_json: &str,
        flowscript: &str,
        node_count: usize,
        has_templates: bool,
        has_run_context: bool,
    ) -> String {
        crate::copilot::prompts::board_system_prompt(
            context_json,
            flowscript,
            node_count,
            has_templates,
            has_run_context,
        )
    }

    /// Execute a tool by name and return the result
    async fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        run_context: Option<&RunContext>,
        graph_context: &GraphContext,
        board: &Board,
    ) -> String {
        match name {
            "think" => {
                if let Ok(args) = serde_json::from_value::<ThinkingArgs>(arguments) {
                    format!("Thinking: {}", args.thought)
                } else {
                    "Thinking...".to_string()
                }
            }
            "get_node_details" => {
                if let Ok(args) = serde_json::from_value::<GetNodeDetailsArgs>(arguments) {
                    build_node_details_output(&args.node_id, graph_context)
                } else {
                    "Failed to parse node ID".to_string()
                }
            }
            "emit_commands" => match serde_json::from_value::<EmitCommandsArgs>(arguments) {
                Ok(args) => {
                    println!(
                        "[Copilot] emit_commands: {} commands, json length: {} chars",
                        args.commands.len(),
                        serde_json::to_string(&args.commands)
                            .unwrap_or_default()
                            .len()
                    );

                    let validation = validation::validate_emit_commands(
                        &args,
                        graph_context,
                        self.catalog_provider.as_ref(),
                    )
                    .await;

                    validation::render_emit_commands_result(&args, &validation)
                }
                Err(e) => {
                    println!("[Copilot] emit_commands: Failed to parse args: {:?}", e);
                    format!("Failed to parse commands: {}", e)
                }
            },
            "list_board_nodes" => build_list_board_nodes_output(graph_context),
            "get_unconfigured_nodes" => build_unconfigured_nodes_output(graph_context),
            "find_connectable_nodes" => {
                match serde_json::from_value::<FindConnectableNodesArgs>(arguments) {
                    Ok(args) => build_find_connectable_nodes_output(
                        graph_context,
                        self.catalog_provider.as_ref(),
                        args,
                    )
                    .await
                    .unwrap_or_else(|err| err.to_string()),
                    Err(err) => format!("Failed to parse connectable-node args: {}", err),
                }
            }
            "catalog_search" => {
                if let Ok(args) = serde_json::from_value::<SearchArgs>(arguments) {
                    let matches = self.catalog_provider.search(&args.query).await;
                    render_catalog_search_results(&matches)
                } else {
                    "[]".to_string()
                }
            }
            "get_declarations" => match serde_json::from_value::<GetDeclarationsArgs>(arguments) {
                Ok(args) => self.catalog_provider.get_declarations(&args.query).await,
                Err(e) => format!("Failed to parse declarations query: {}", e),
            },
            "get_current_flowscript" => crate::flow::ast::board_to_flowscript(
                board,
                &crate::flow::ast::RenderOptions {
                    anchors: true,
                    ..Default::default()
                },
            ),
            "edit_flowscript" => match serde_json::from_value::<EditFlowScriptArgs>(arguments) {
                Ok(args) => {
                    let catalog = self.catalog_provider.get_all_metadata().await;
                    let result = crate::flow::ast::reconcile_text_with_catalog(
                        board,
                        &args.flowscript,
                        &catalog,
                    );
                    render_edit_flowscript_result(
                        &args.flowscript,
                        &result,
                        board_has_no_nodes(board),
                        args.allow_deletions,
                    )
                }
                Err(e) => format!("Failed to parse FlowScript edit: {}", e),
            },
            "search_by_pin" => {
                if let Ok(args) = serde_json::from_value::<SearchByPinArgs>(arguments) {
                    let matches = self
                        .catalog_provider
                        .search_by_pin_type(&args.pin_type, args.is_input)
                        .await;
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "filter_category" => {
                if let Ok(args) = serde_json::from_value::<FilterCategoryArgs>(arguments) {
                    let matches = self
                        .catalog_provider
                        .filter_by_category(&args.category_prefix)
                        .await;
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "search_templates" => {
                if let Ok(args) = serde_json::from_value::<SearchTemplatesArgs>(arguments) {
                    let query_lower = args.query.to_lowercase();
                    let mut matches: Vec<&TemplateInfo> = self
                        .templates
                        .iter()
                        .filter(|t| {
                            // Skip current template being edited
                            if let Some(ref current_id) = self.current_template_id
                                && &t.id == current_id
                            {
                                return false;
                            }
                            t.name.to_lowercase().contains(&query_lower)
                                || t.description.to_lowercase().contains(&query_lower)
                                || t.tags
                                    .iter()
                                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                                || t.node_types
                                    .iter()
                                    .any(|nt| nt.to_lowercase().contains(&query_lower))
                        })
                        .take(5)
                        .collect();
                    // Sort by relevance
                    matches.sort_by(|a, b| {
                        let a_name_match = a.name.to_lowercase().contains(&query_lower);
                        let b_name_match = b.name.to_lowercase().contains(&query_lower);
                        b_name_match.cmp(&a_name_match)
                    });
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "query_logs" => {
                #[cfg(feature = "flow-runtime")]
                {
                    if let Some(ctx) = run_context {
                        let args = serde_json::from_value::<QueryLogsArgs>(arguments).unwrap_or(
                            QueryLogsArgs {
                                filter: None,
                                limit: None,
                            },
                        );

                        let limit = args.limit.unwrap_or(50).min(100);
                        let filter = args.filter.unwrap_or_default();

                        let log_meta = crate::flow::execution::LogMeta {
                            app_id: ctx.app_id.clone(),
                            run_id: ctx.run_id.clone(),
                            board_id: ctx.board_id.clone(),
                            start: 0,
                            end: 0,
                            log_level: 0,
                            version: String::new(),
                            nodes: None,
                            logs: None,
                            node_id: String::new(),
                            event_version: None,
                            event_id: String::new(),
                            payload: vec![],
                            is_remote: false,
                        };

                        match self
                            .state
                            .query_run(&log_meta, &filter, Some(limit), Some(0))
                            .await
                        {
                            Ok(logs) => {
                                if logs.is_empty() {
                                    if filter.is_empty() {
                                        "No logs found for this run.".to_string()
                                    } else {
                                        "No logs matching your filter criteria.".to_string()
                                    }
                                } else {
                                    let formatted: Vec<serde_json::Value> = logs.iter().map(|log| {
                                        json!({
                                            "level": match log.log_level {
                                                crate::flow::execution::LogLevel::Debug => "Debug",
                                                crate::flow::execution::LogLevel::Info => "Info",
                                                crate::flow::execution::LogLevel::Warn => "Warn",
                                                crate::flow::execution::LogLevel::Error => "Error",
                                                crate::flow::execution::LogLevel::Fatal => "Fatal",
                                            },
                                            "message": log.message,
                                            "node_id": log.node_id,
                                        })
                                    }).collect();
                                    serde_json::to_string_pretty(&formatted).unwrap_or_default()
                                }
                            }
                            Err(e) => format!("Failed to query logs: {}", e),
                        }
                    } else {
                        "No run context available. Please select a run first.".to_string()
                    }
                }
                #[cfg(not(feature = "flow-runtime"))]
                {
                    let _ = run_context; // Suppress unused variable warning
                    "Log querying is not available in this build.".to_string()
                }
            }
            _ => {
                println!("[Copilot] Unknown tool requested: {}", name);
                format!("Unknown tool: {}", name)
            }
        }
    }

    /// Parse commands from the agent's response
    fn parse_commands(response: &str) -> Vec<BoardCommand> {
        // Look for <commands>...</commands> tags
        if let Some(json_str) = Self::extract_tag_content(response, "commands") {
            if let Ok(commands) = serde_json::from_str::<Vec<BoardCommand>>(json_str) {
                return commands;
            }
        }
        vec![]
    }

    /// Check if two commands are duplicates (same type and key identifiers)
    fn commands_are_duplicate(a: &BoardCommand, b: &BoardCommand) -> bool {
        match (a, b) {
            (
                BoardCommand::AddNode {
                    node_type: t1,
                    ref_id: r1,
                    ..
                },
                BoardCommand::AddNode {
                    node_type: t2,
                    ref_id: r2,
                    ..
                },
            ) => t1 == t2 && r1 == r2,
            (
                BoardCommand::AddPlaceholder {
                    name: n1,
                    ref_id: r1,
                    ..
                },
                BoardCommand::AddPlaceholder {
                    name: n2,
                    ref_id: r2,
                    ..
                },
            ) => n1 == n2 || r1 == r2,
            (
                BoardCommand::RemoveNode { node_id: id1, .. },
                BoardCommand::RemoveNode { node_id: id2, .. },
            ) => id1 == id2,
            (
                BoardCommand::ConnectPins {
                    from_node: f1,
                    from_pin: fp1,
                    to_node: t1,
                    to_pin: tp1,
                    ..
                },
                BoardCommand::ConnectPins {
                    from_node: f2,
                    from_pin: fp2,
                    to_node: t2,
                    to_pin: tp2,
                    ..
                },
            ) => f1 == f2 && fp1 == fp2 && t1 == t2 && tp1 == tp2,
            _ => false,
        }
    }

    /// Clean the message by removing command tags
    fn clean_message(response: &str) -> String {
        let mut result = response.to_string();
        Self::strip_tag_block(&mut result, "commands");
        Self::strip_tag_block(&mut result, "flowscript_workspace");
        result.trim().to_string()
    }

    fn clean_validation_message(response: &str) -> String {
        let mut result = response.to_string();
        Self::strip_tag_block(&mut result, "validation");
        result.trim().to_string()
    }

    fn parse_flowscript_workspace(response: &str) -> Option<String> {
        let payload = Self::extract_tag_content(response, "flowscript_workspace")?;
        let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
        if let Some(source) = value.as_str() {
            return Some(source.to_string());
        }
        value
            .get("source")
            .and_then(|source| source.as_str())
            .map(str::to_string)
    }

    fn extract_tag_content<'a>(response: &'a str, tag: &str) -> Option<&'a str> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = response.find(&open)?;
        let remainder = &response[start + open.len()..];
        let end = remainder.find(&close)?;
        Some(&remainder[..end])
    }

    fn strip_tag_block(result: &mut String, tag: &str) {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        while let Some(start) = result.find(&open) {
            let search_from = start + open.len();
            let Some(relative_end) = result[search_from..].find(&close) else {
                break;
            };
            let end = search_from + relative_end + close.len();
            result.replace_range(start..end, "");
        }
    }

    /// Get the model for the agent
    async fn get_model<'a>(
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
        let completion = provider.into_client();

        Ok((default_model, completion))
    }
}
