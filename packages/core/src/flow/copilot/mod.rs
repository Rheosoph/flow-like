//! Flow Copilot - AI-powered graph editing assistant
//!
//! This module provides the Copilot struct which enables natural language
//! interaction with flow graphs, supporting both explanation and modification.

mod context;
mod provider;
mod tools;
mod types;

pub use context::{
    EdgeContext, GraphContext, LayerContext, NodeContext, PinContext, prepare_context,
};
pub use provider::CatalogProvider;
pub use tools::{
    CatalogTool, EmitCommandsArgs, EmitCommandsTool, FilterCategoryArgs, FilterCategoryTool,
    GetNodeDetailsArgs, GetNodeDetailsTool, QueryLogsArgs, QueryLogsTool, SearchArgs,
    SearchByPinArgs, SearchByPinTool, SearchTemplatesArgs, SearchTemplatesTool, ThinkingArgs,
    get_tool_description,
};
pub use types::{
    AgentType, BoardCommand, ChatImage, ChatMessage, ChatRole, Connection, CopilotResponse, Edge,
    NodeMetadata, NodePosition, PinMetadata, PlaceholderPinDef, PlanStep, PlanStepStatus,
    RunContext, StreamEvent, Suggestion, TemplateInfo,
};

use std::sync::Arc;

use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    client::completion::CompletionClientDyn,
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

        // Only include node type names (not full paths) for context efficiency
        let available_nodes = self.catalog_provider.get_all_nodes().await;
        let node_count = available_nodes.len();

        let (model_name, completion_client) = self.get_model(model_id, token).await?;

        // Build a compact system prompt
        let system_prompt = Self::build_system_prompt(
            &context_json,
            node_count,
            !self.templates.is_empty(),
            run_context.is_some(),
        );

        let mut agent_builder = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .tool(ThinkTool)
            .tool(GetNodeDetailsTool {
                graph_context: Arc::new(context.clone()),
            })
            .tool(EmitCommandsTool)
            .tool(CatalogTool {
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

        // Convert chat history to rig message format (including images)
        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| {
                match msg.role {
                    ChatRole::User => {
                        let mut contents: Vec<UserContent> =
                            vec![UserContent::Text(rig::message::Text {
                                text: msg.content.clone(),
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
                        })),
                    }),
                }
            })
            .collect();

        let mut full_response = String::new();
        let mut all_commands: Vec<BoardCommand> = Vec::new();
        let max_iterations = 10u64;
        let mut plan_step_counter = 0u32;

        for iteration in 0..max_iterations {
            // Send iteration start event
            if let Some(ref callback) = on_token {
                plan_step_counter += 1;
                let step_event = StreamEvent::PlanStep(PlanStep {
                    id: format!("iteration_{}", iteration),
                    description: if iteration == 0 {
                        "Analyzing request...".to_string()
                    } else {
                        "Processing tool results...".to_string()
                    },
                    status: PlanStepStatus::InProgress,
                    tool_name: Some("analyze".to_string()),
                });
                callback(format!(
                    "<plan_step>{}</plan_step>",
                    serde_json::to_string(&step_event).unwrap_or_default()
                ));
            }

            // Build completion request - tools are already attached via agent builder
            let request = agent
                .completion(prompt.clone(), current_history.clone())
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
                    StreamedAssistantContent::ToolCall(tool_call) => {
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta { .. } => {
                        // Deltas are accumulated into the final ToolCall
                    }
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let reasoning_text = reasoning.reasoning.join("\n");
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

                            let step_event = StreamEvent::PlanStep(PlanStep {
                                id: reasoning_step_id.clone().unwrap(),
                                description: current_reasoning.trim().to_string(),
                                status: PlanStepStatus::InProgress,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                    }
                    StreamedAssistantContent::Final(_) => {
                        // Mark reasoning step as completed
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            let step_event = StreamEvent::PlanStep(PlanStep {
                                id: step_id.clone(),
                                description: current_reasoning.trim().to_string(),
                                status: PlanStepStatus::Completed,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
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

                            let step_event = StreamEvent::PlanStep(PlanStep {
                                id: reasoning_step_id.clone().unwrap(),
                                description: current_reasoning.trim().to_string(),
                                status: PlanStepStatus::InProgress,
                                tool_name: Some("think".to_string()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                    }
                }
            }

            // Mark reasoning step as completed if stream ended while reasoning
            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                let step_event = StreamEvent::PlanStep(PlanStep {
                    id: step_id.clone(),
                    description: current_reasoning.trim().to_string(),
                    status: PlanStepStatus::Completed,
                    tool_name: Some("think".to_string()),
                });
                callback(format!(
                    "<plan_step>{}</plan_step>",
                    serde_json::to_string(&step_event).unwrap_or_default()
                ));
            }

            // Mark iteration analysis as complete
            if let Some(ref callback) = on_token {
                let step_event = StreamEvent::PlanStep(PlanStep {
                    id: format!("iteration_{}", iteration),
                    description: if iteration == 0 {
                        "Analysis complete".to_string()
                    } else {
                        "Tool results processed".to_string()
                    },
                    status: PlanStepStatus::Completed,
                    tool_name: Some("analyze".to_string()),
                });
                callback(format!(
                    "<plan_step>{}</plan_step>",
                    serde_json::to_string(&step_event).unwrap_or_default()
                ));
            }

            full_response.push_str(&iteration_text);

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
                // Emit plan steps for all tool calls starting
                let mut step_ids: Vec<(String, String, u32)> = Vec::new();
                for tool_call in &tool_calls {
                    plan_step_counter += 1;
                    let step_id = format!("step_{}", plan_step_counter);
                    let step_description = get_tool_description(
                        &tool_call.function.name,
                        &tool_call.function.arguments,
                    );

                    if let Some(ref callback) = on_token {
                        callback(format!("tool_call:{}", tool_call.function.name));
                        let step_event = StreamEvent::PlanStep(PlanStep {
                            id: step_id.clone(),
                            description: step_description.clone(),
                            status: PlanStepStatus::InProgress,
                            tool_name: Some(tool_call.function.name.clone()),
                        });
                        callback(format!(
                            "<plan_step>{}</plan_step>",
                            serde_json::to_string(&step_event).unwrap_or_default()
                        ));
                    }

                    step_ids.push((step_id, step_description, plan_step_counter));
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
                        async move {
                            let output = self
                                .execute_tool(&name, arguments, ctx.as_ref(), &graph_ctx)
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

                    // Parse commands from emit_commands tool output
                    if name == "emit_commands" {
                        let parsed = Self::parse_commands(tool_output);
                        println!("[Copilot] emit_commands parsed {} commands:", parsed.len());
                        for (idx, cmd) in parsed.iter().enumerate() {
                            println!("[Copilot]   [{}] {:?}", idx, cmd);
                        }

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
                    }

                    // Emit plan step completion
                    if let Some(ref callback) = on_token {
                        if let Some((step_id, step_description, _)) = step_ids.get(i) {
                            let step_event = StreamEvent::PlanStep(PlanStep {
                                id: step_id.clone(),
                                description: step_description.clone(),
                                status: PlanStepStatus::Completed,
                                tool_name: Some(name.clone()),
                            });
                            callback(format!(
                                "<plan_step>{}</plan_step>",
                                serde_json::to_string(&step_event).unwrap_or_default()
                            ));
                        }
                        callback("tool_result:done".to_string());
                    }
                }

                // Add assistant message with tool calls to history
                let assistant_msg = rig::message::Message::Assistant {
                    id: None,
                    content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                        OneOrMany::one(AssistantContent::Text(rig::message::Text {
                            text: String::new(),
                        }))
                    }),
                };
                current_history.push(assistant_msg);

                // Add all tool results to history as a single User message
                // This is required for Gemini API which expects tool results to immediately follow
                // the assistant's tool call message in a single message
                if !tool_results.is_empty() {
                    let tool_result_contents: Vec<UserContent> = tool_results
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

                    let combined_tool_results = if tool_result_contents.len() == 1 {
                        OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
                    } else {
                        OneOrMany::many(tool_result_contents)
                            .expect("tool_result_contents should have at least 2 elements")
                    };

                    let tool_result_msg = rig::message::Message::User {
                        content: combined_tool_results,
                    };
                    current_history.push(tool_result_msg);
                }
            } else {
                // No tool calls, we're done
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
        let response = CopilotResponse {
            agent_type: if has_commands {
                AgentType::Edit
            } else {
                AgentType::Explain
            },
            message: Self::clean_message(&full_response),
            commands: all_commands,
            suggestions: vec![],
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
        node_count: usize,
        has_templates: bool,
        has_run_context: bool,
    ) -> String {
        crate::copilot::prompts::board_system_prompt(
            context_json,
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
                    // Find the node in the context
                    let node = graph_context.nodes.iter().find(|n| n.id == args.node_id);

                    match node {
                        Some(node_ctx) => {
                            // Build detailed output including all connections
                            let incoming_edges: Vec<_> = graph_context
                                .edges
                                .iter()
                                .filter(|e| e.to_node_id == args.node_id)
                                .map(|e| {
                                    json!({
                                        "from_node": e.from_node_id,
                                        "from_pin": e.from_pin_name,
                                        "to_pin": e.to_pin_name
                                    })
                                })
                                .collect();

                            let outgoing_edges: Vec<_> = graph_context
                                .edges
                                .iter()
                                .filter(|e| e.from_node_id == args.node_id)
                                .map(|e| {
                                    json!({
                                        "from_pin": e.from_pin_name,
                                        "to_node": e.to_node_id,
                                        "to_pin": e.to_pin_name
                                    })
                                })
                                .collect();

                            let details = json!({
                                "id": node_ctx.id,
                                "node_type": node_ctx.node_type,
                                "friendly_name": node_ctx.friendly_name,
                                "position": { "x": node_ctx.position.0, "y": node_ctx.position.1 },
                                "size": { "width": node_ctx.estimated_size.0, "height": node_ctx.estimated_size.1 },
                                "inputs": node_ctx.inputs.iter().map(|p| {
                                    json!({
                                        "name": p.name,
                                        "type": p.type_name,
                                        "default_value": p.default_value
                                    })
                                }).collect::<Vec<_>>(),
                                "outputs": node_ctx.outputs.iter().map(|p| {
                                    json!({
                                        "name": p.name,
                                        "type": p.type_name
                                    })
                                }).collect::<Vec<_>>(),
                                "incoming_connections": incoming_edges,
                                "outgoing_connections": outgoing_edges,
                                "is_selected": graph_context.selected_nodes.contains(&args.node_id)
                            });

                            serde_json::to_string_pretty(&details).unwrap_or_default()
                        }
                        None => format!(
                            "Node with ID '{}' not found in the current graph",
                            args.node_id
                        ),
                    }
                } else {
                    "Failed to parse node ID".to_string()
                }
            }
            "emit_commands" => match serde_json::from_value::<EmitCommandsArgs>(arguments) {
                Ok(args) => {
                    let commands_json = serde_json::to_string(&args.commands).unwrap_or_default();
                    println!(
                        "[Copilot] emit_commands: {} commands, json length: {} chars",
                        args.commands.len(),
                        commands_json.len()
                    );
                    format!(
                        "<commands>{}</commands>\n\n{}",
                        commands_json, args.explanation
                    )
                }
                Err(e) => {
                    println!("[Copilot] emit_commands: Failed to parse args: {:?}", e);
                    format!("Failed to parse commands: {}", e)
                }
            },
            "catalog_search" => {
                if let Ok(args) = serde_json::from_value::<SearchArgs>(arguments) {
                    let matches = self.catalog_provider.search(&args.query).await;
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
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
        if let Some(start) = response.find("<commands>")
            && let Some(end) = response.find("</commands>")
        {
            let json_str = &response[start + 10..end];
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
        // Remove <commands>...</commands> block
        let mut result = response.to_string();
        if let Some(start) = result.find("<commands>")
            && let Some(end) = result.find("</commands>")
        {
            result = format!("{}{}", &result[..start], &result[end + 11..]);
        }
        result.trim().to_string()
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
            .build(&bit, self.state.clone(), token)
            .await?;
        let default_model = model.default_model().await.unwrap_or("gpt-4o".to_string());
        let provider = model.provider().await?;
        let completion = provider.into_client();

        Ok((default_model, completion))
    }
}
