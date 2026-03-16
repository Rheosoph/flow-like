use crate::state::{TauriFlowLikeState, TauriSettingsState};
use async_trait::async_trait;
use flow_like::a2ui::SurfaceComponent;
use flow_like::copilot::{
    CopilotScope, UIActionContext, UnifiedChatMessage, UnifiedContext, UnifiedCopilot,
    UnifiedCopilotResponse,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{
    BoardCommand, CatalogProvider, NodeMetadata, PinMetadata, RunContext,
};
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like_catalog::get_catalog;
use std::sync::Arc;
use tauri::{AppHandle, State, ipc::Channel};

/// Desktop implementation of the catalog provider for node search
struct DesktopCatalogProvider {
    _state: TauriFlowLikeState,
}

fn pin_to_metadata(p: &Pin) -> PinMetadata {
    let is_generic = p.data_type == VariableType::Generic;
    let enforce_schema = p
        .options
        .as_ref()
        .and_then(|o| o.enforce_schema)
        .unwrap_or(false);
    let valid_values = p.options.as_ref().and_then(|o| o.valid_values.clone());

    PinMetadata {
        name: p.name.clone(),
        friendly_name: p.friendly_name.clone(),
        description: p.description.clone(),
        data_type: format!("{:?}", p.data_type),
        value_type: format!("{:?}", p.value_type),
        schema: p.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

#[async_trait]
impl CatalogProvider for DesktopCatalogProvider {
    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let catalog = get_catalog();
        let query_lower = query.to_lowercase();
        let query_tokens: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for logic in catalog {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let friendly_lower = node.friendly_name.to_lowercase();
            let desc_lower = node.description.to_lowercase();

            let category = name_lower.split("::").nth(1).unwrap_or("");

            let mut score = 0i32;

            if name_lower.contains(&query_lower) {
                score += 100;
            }
            if friendly_lower.contains(&query_lower) {
                score += 90;
            }

            for token in &query_tokens {
                if name_lower.contains(token) {
                    score += 30;
                }
                if friendly_lower.contains(token) {
                    score += 25;
                }
                if category.contains(token) {
                    score += 20;
                }
                if desc_lower.contains(token) {
                    score += 10;
                }
            }

            let name_parts: Vec<&str> = name_lower.split([':', '_']).collect();
            for token in &query_tokens {
                if name_parts.iter().any(|part| part == token) {
                    score += 15;
                }
            }

            if score > 0 {
                scored_matches.push((
                    score,
                    NodeMetadata {
                        name: node.name,
                        friendly_name: node.friendly_name,
                        description: node.description,
                        inputs: node
                            .pins
                            .values()
                            .filter(|p| p.pin_type == PinType::Input)
                            .map(pin_to_metadata)
                            .collect(),
                        outputs: node
                            .pins
                            .values()
                            .filter(|p| p.pin_type == PinType::Output)
                            .map(pin_to_metadata)
                            .collect(),
                        category: Some(category.to_string()),
                    },
                ));
            }
        }

        scored_matches.sort_by(|a, b| b.0.cmp(&a.0));
        scored_matches
            .into_iter()
            .take(10)
            .map(|(_, meta)| meta)
            .collect()
    }

    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata> {
        let catalog = get_catalog();
        let pin_type = pin_type.to_lowercase();
        let mut matches = Vec::new();

        for logic in catalog {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            let has_matching_pin = node.pins.values().any(|p| {
                let is_correct_direction = if is_input {
                    p.pin_type == PinType::Input
                } else {
                    p.pin_type == PinType::Output
                };
                is_correct_direction
                    && format!("{:?}", p.data_type)
                        .to_lowercase()
                        .contains(&pin_type)
            });

            if has_matching_pin {
                matches.push(NodeMetadata {
                    name: node.name,
                    friendly_name: node.friendly_name,
                    description: node.description,
                    inputs: node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Input)
                        .map(pin_to_metadata)
                        .collect(),
                    outputs: node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Output)
                        .map(pin_to_metadata)
                        .collect(),
                    category: Some(category.to_string()),
                });
            }
            if matches.len() >= 10 {
                break;
            }
        }
        matches
    }

    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata> {
        let catalog = get_catalog();
        let category_prefix = category_prefix.to_lowercase();
        let mut matches = Vec::new();

        for logic in catalog {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            if category.starts_with(&category_prefix) || name_lower.contains(&category_prefix) {
                matches.push(NodeMetadata {
                    name: node.name,
                    friendly_name: node.friendly_name,
                    description: node.description,
                    inputs: node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Input)
                        .map(pin_to_metadata)
                        .collect(),
                    outputs: node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Output)
                        .map(pin_to_metadata)
                        .collect(),
                    category: Some(category.to_string()),
                });
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        let catalog = get_catalog();
        catalog.iter().map(|logic| logic.get_node().name).collect()
    }
}

/// Unified copilot chat command that handles both board and UI generation
#[tauri::command]
pub async fn copilot_chat(
    app_handle: AppHandle,
    state: State<'_, TauriFlowLikeState>,
    // Scope selection
    scope: CopilotScope,
    // Board context (optional for Frontend scope)
    board: Option<Board>,
    selected_node_ids: Option<Vec<String>>,
    // UI context (optional for Board scope)
    current_surface: Option<Vec<SurfaceComponent>>,
    selected_component_ids: Option<Vec<String>>,
    // Common parameters
    user_prompt: String,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    token: Option<String>,
    // Extended context
    run_context: Option<RunContext>,
    action_context: Option<UIActionContext>,
    // Streaming channel
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    // Check if using Copilot SDK (model_id starts with "copilot:")
    if let Some(ref id) = model_id
        && let Some(copilot_model) = id.strip_prefix("copilot:")
    {
        return copilot_sdk_chat_internal(
            copilot_model,
            scope,
            board.as_ref(),
            selected_node_ids.as_deref().unwrap_or(&[]),
            current_surface.as_ref(),
            user_prompt,
            history.unwrap_or_default(),
            channel,
        )
        .await;
    }

    println!(
        "[copilot_chat] Called with scope: {:?}, run_context: {:?}",
        scope, run_context
    );

    let selected_node_ids = selected_node_ids.unwrap_or_default();
    let selected_component_ids = selected_component_ids.unwrap_or_default();
    let history = history.unwrap_or_default();

    let state_clone = state.0.clone();

    let profile = TauriSettingsState::current_profile(&app_handle)
        .await
        .ok()
        .map(|p| Arc::new(p.hub_profile));

    // Only create catalog provider if we might need it (Board or Both scope)
    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend => None,
        _ => Some(Arc::new(DesktopCatalogProvider {
            _state: state.inner().clone(),
        })),
    };

    let copilot = UnifiedCopilot::new(state_clone, catalog_provider, profile, None)
        .await
        .map_err(|e| e.to_string())?;

    let on_token = Some(move |token: String| {
        let _ = channel.send(token);
    });

    // Build unified context
    let context = if run_context.is_some() || action_context.is_some() {
        Some(UnifiedContext {
            scope,
            run_context,
            action_context,
        })
    } else {
        None
    };

    copilot
        .chat(
            scope,
            board.as_ref(),
            &selected_node_ids,
            current_surface.as_ref(),
            &selected_component_ids,
            user_prompt,
            history,
            model_id,
            token,
            context,
            on_token,
        )
        .await
        .map_err(|e| e.to_string())
}

/// Internal function to handle Copilot SDK chat
async fn copilot_sdk_chat_internal(
    model_id: &str,
    scope: CopilotScope,
    board: Option<&Board>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    user_prompt: String,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    use super::copilot_sdk_tools::{create_board_tools, create_frontend_tools};
    use copilot_sdk::SessionEventData;
    use flow_like::flow::copilot::prepare_context;

    let guard = COPILOT_CLIENT.lock().await;
    let client = guard
        .as_ref()
        .ok_or("Copilot SDK not running. Please start it first.")?;

    // Build graph context for board tools (only if in Board or Both scope)
    let graph_context = match scope {
        CopilotScope::Board | CopilotScope::Both => {
            if let Some(board) = board {
                prepare_context(board, selected_node_ids).ok().map(Arc::new)
            } else {
                None
            }
        }
        CopilotScope::Frontend => None,
    };

    // Create tools based on scope
    let tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> = match scope {
        CopilotScope::Board => create_board_tools(graph_context),
        CopilotScope::Frontend => create_frontend_tools(),
        CopilotScope::Both => {
            let mut all_tools = create_board_tools(graph_context);
            all_tools.extend(create_frontend_tools());
            all_tools
        }
    };

    // Extract just the Tool definitions for SessionConfig
    let tool_defs: Vec<copilot_sdk::Tool> = tools.iter().map(|(t, _)| t.clone()).collect();

    // Build context from history for the system message
    let mut context_parts = vec![];
    for msg in &history {
        let role = match msg.role {
            flow_like::flow::copilot::ChatRole::User => "User",
            flow_like::flow::copilot::ChatRole::Assistant => "Assistant",
        };
        context_parts.push(format!("{}: {}", role, msg.content));
    }

    // Build system prompt from the shared prompts module
    let mut system_content = match scope {
        CopilotScope::Board => flow_like::copilot::prompts::board_sdk_system_prompt(),
        CopilotScope::Frontend => flow_like::copilot::prompts::frontend_sdk_system_prompt(),
        CopilotScope::Both => flow_like::copilot::prompts::general_system_prompt(),
    };

    // Add current UI surface context for Frontend/Both scopes
    if matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
        && let Some(components) = current_surface
        && !components.is_empty()
    {
        let components_json =
            serde_json::to_string_pretty(components).unwrap_or_else(|_| "[]".to_string());
        system_content.push_str(&format!(
                    "\n\n## CURRENT UI COMPONENTS\nThe user has the following existing UI. You can modify or extend it:\n```json\n{}\n```",
                    components_json
                ));
    }

    // Add conversation history
    if !context_parts.is_empty() {
        system_content.push_str(&format!(
            "\n\nConversation history:\n{}",
            context_parts.join("\n\n")
        ));
    }

    // Exclude built-in Copilot tools that shouldn't be used (file editing, shell commands).
    // Do NOT set available_tools — it can conflict with custom tool visibility in the CLI.
    // Custom tools (emit_ui, get_component_schema, emit_commands, etc.) are always available
    // via the `tools` array in the session config.
    let excluded_tools = match scope {
        CopilotScope::Frontend => Some(vec![
            "Read".to_string(),
            "Edit".to_string(),
            "Write".to_string(),
            "shell".to_string(),
            "powershell".to_string(),
            "bash".to_string(),
            "Grep".to_string(),
            "listDir".to_string(),
            "Search".to_string(),
            "Insert".to_string(),
            "Replace".to_string(),
            "CreateFile".to_string(),
        ]),
        _ => None,
    };

    let config = copilot_sdk::SessionConfig {
        model: Some(model_id.to_string()),
        streaming: true,
        tools: tool_defs,
        excluded_tools,
        request_permission: Some(false),
        system_message: Some(copilot_sdk::SystemMessageConfig {
            content: Some(system_content),
            mode: Some(copilot_sdk::SystemMessageMode::Replace),
        }),
        infinite_sessions: Some(copilot_sdk::InfiniteSessionConfig::enabled()),
        ..Default::default()
    };

    let session = client
        .create_session(config)
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    // Register tool handlers
    for (tool, handler) in tools {
        session
            .register_tool_with_handler(tool, Some(handler))
            .await;
    }

    // Approve all permission requests so the CLI never blocks tool execution
    session
        .register_permission_handler(|_req| copilot_sdk::PermissionRequestResult::approved())
        .await;

    let mut events = session.subscribe();
    session
        .send(user_prompt.as_str())
        .await
        .map_err(|e| format!("Failed to send message: {}", e))?;

    let mut full_response = String::new();
    let mut extracted_commands: Vec<BoardCommand> = Vec::new();
    let mut extracted_components: Vec<SurfaceComponent> = Vec::new();
    let mut extracted_canvas_settings: Option<serde_json::Value> = None;
    let mut extracted_root_component_id: Option<String> = None;

    loop {
        match events.recv().await {
            Ok(event) => match &event.data {
                SessionEventData::AssistantMessageDelta(delta) => {
                    full_response.push_str(&delta.delta_content);
                    let _ = channel.send(delta.delta_content.clone());
                }
                SessionEventData::AssistantMessage(msg) => {
                    // Don't overwrite accumulated content unless it's truly final
                    if full_response.is_empty() {
                        full_response = msg.content.clone();
                    }
                }
                SessionEventData::ToolExecutionStart(tool_event) => {
                    // Send tool start event to frontend
                    let tool_msg = format!(
                        "<tool_start>{{\"tool\":\"{}\",\"status\":\"running\"}}</tool_start>",
                        tool_event.tool_name
                    );
                    let _ = channel.send(tool_msg);
                }
                SessionEventData::ToolExecutionComplete(tool_complete) => {
                    if let Some(ref result) = tool_complete.result
                        && let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&result.content)
                    {
                        // Extract commands from emit_commands tool (status: "queued")
                        if parsed.get("status").and_then(|s| s.as_str()) == Some("queued")
                            && let Some(cmds) = parsed.get("commands")
                            && let Ok(commands) =
                                serde_json::from_value::<Vec<BoardCommand>>(cmds.clone())
                        {
                            let cmd_event = format!(
                                "<commands>{}</commands>",
                                serde_json::to_string(&commands).unwrap_or_default()
                            );
                            let _ = channel.send(cmd_event);
                            extracted_commands.extend(commands);
                        }
                        // Extract components from emit_ui tool (status: "rendered")
                        if parsed.get("status").and_then(|s| s.as_str()) == Some("rendered") {
                            // Extract canvasSettings
                            if let Some(canvas) = parsed.get("canvasSettings") {
                                extracted_canvas_settings = Some(canvas.clone());
                            }
                            // Extract rootComponentId
                            if let Some(root_id) =
                                parsed.get("rootComponentId").and_then(|v| v.as_str())
                            {
                                extracted_root_component_id = Some(root_id.to_string());
                            }
                            // Extract components
                            if let Some(comps) = parsed.get("components")
                                && let Ok(components) =
                                    serde_json::from_value::<Vec<SurfaceComponent>>(comps.clone())
                            {
                                // Send components WITH canvas settings to frontend
                                let comp_event = format!(
                                    "<components>{}</components>",
                                    serde_json::to_string(&components).unwrap_or_default()
                                );
                                let _ = channel.send(comp_event);
                                // Also send canvas settings
                                if let Some(ref canvas) = extracted_canvas_settings {
                                    let canvas_event = format!(
                                        "<canvas_settings>{}</canvas_settings>",
                                        serde_json::to_string(canvas).unwrap_or_default()
                                    );
                                    let _ = channel.send(canvas_event);
                                }
                                extracted_components.extend(components);
                            }
                        }
                    }

                    // Send tool completion event to frontend
                    let status = if tool_complete.success {
                        "done"
                    } else {
                        "error"
                    };
                    let tool_msg = format!(
                        "<tool_end>{{\"tool_call_id\":\"{}\",\"status\":\"{}\"}}</tool_end>",
                        tool_complete.tool_call_id, status
                    );
                    let _ = channel.send(tool_msg);
                }
                SessionEventData::SessionIdle(_) => {
                    break;
                }
                SessionEventData::SessionError(err) => {
                    return Err(format!("Session error: {:?}", err));
                }
                _ => {}
            },
            Err(e) => {
                println!("[copilot_sdk_chat] Event receive error: {}", e);
                break;
            }
        }
    }

    // ── Fallback: if the model didn't call emit_ui but dumped JSON in the
    // response text, extract components from there so they still show up.
    if extracted_components.is_empty()
        && matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
    {
        let surface =
            flow_like::a2ui::copilot::extract_surface_from_response(&full_response);
        if !surface.components.is_empty() {
            println!(
                "[copilot_sdk_chat] Fallback: extracted {} components from text response",
                surface.components.len()
            );
            // Forward to frontend via channel so streaming UI picks them up
            let comp_event = format!(
                "<components>{}</components>",
                serde_json::to_string(&surface.components).unwrap_or_default()
            );
            let _ = channel.send(comp_event);
            if let Some(ref canvas) = surface.canvas_settings {
                let canvas_event = format!(
                    "<canvas_settings>{}</canvas_settings>",
                    serde_json::to_string(canvas).unwrap_or_default()
                );
                let _ = channel.send(canvas_event);
            }

            extracted_components = surface.components;
            if extracted_canvas_settings.is_none() {
                extracted_canvas_settings = surface.canvas_settings;
            }
            if extracted_root_component_id.is_none() {
                extracted_root_component_id = surface.root_component_id;
            }
        }
    }

    Ok(UnifiedCopilotResponse {
        message: full_response,
        commands: extracted_commands,
        suggestions: vec![],
        components: extracted_components,
        canvas_settings: extracted_canvas_settings,
        root_component_id: extracted_root_component_id,
        active_scope: scope,
    })
}

// =============================================================================
// GitHub Copilot SDK Direct Integration
// =============================================================================

use copilot_sdk::{Client, LogLevel};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Global Copilot client instance (singleton) - uses tokio::sync::Mutex for async safety
static COPILOT_CLIENT: Lazy<Mutex<Option<Client>>> = Lazy::new(|| Mutex::new(None));

/// Model info returned from GitHub Copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotModelInfo {
    pub id: String,
    pub name: String,
}

/// Auth status returned from GitHub Copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAuthStatus {
    pub authenticated: bool,
    pub login: Option<String>,
}

/// Collect extra bin directories that are typically absent from a bundled-app
/// PATH (Homebrew, nvm, volta, fnm, mise, pnpm, bun, npm-global, …).
fn extra_bin_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let Some(home) = dirs_next::home_dir() else {
        return vec![];
    };

    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".volta/bin"),
        home.join(".bun/bin"),
        home.join(".local/share/pnpm"),
        home.join(".local/bin"),
    ];

    // nvm – scan all installed node versions
    let nvm_dir = std::env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("bin"));
        }
    }

    // fnm
    if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("installation/bin"));
        }
    }

    // mise / rtx node shims
    dirs.push(home.join(".local/share/mise/shims"));

    // npm global prefix variants
    dirs.push(home.join(".npm-global/bin"));
    dirs.push(home.join(".npm-packages/bin"));
    dirs.push(home.join(".npm/bin"));

    dirs
}

/// Resolve the Copilot CLI path, searching beyond the (possibly limited) bundled-app PATH.
///
/// On macOS/Linux, apps launched from Finder/Dock inherit a minimal PATH that
/// excludes npm-global, nvm, volta, mise, and Homebrew directories. This
/// function probes those common locations so that prod builds can find the CLI.
fn find_copilot_cli_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    if let Ok(p) = std::env::var("COPILOT_CLI_PATH") {
        let p = PathBuf::from(p.trim());
        if p.exists() {
            return Some(p);
        }
    }

    for dir in &extra_bin_dirs() {
        let candidate = dir.join("copilot");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// Build an augmented PATH that prepends the extra bin directories to the
/// current PATH so that the spawned copilot CLI process (a Node.js script)
/// can locate `node` and other tools even in production builds.
fn augmented_path() -> String {
    let extra: Vec<String> = extra_bin_dirs()
        .into_iter()
        .filter(|d| d.exists())
        .map(|d| d.to_string_lossy().into_owned())
        .collect();

    let current = std::env::var("PATH").unwrap_or_default();
    if extra.is_empty() {
        return current;
    }

    format!("{}:{}", extra.join(":"), current)
}

/// Start the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_start(
    use_stdio: Option<bool>,
    cli_url: Option<String>,
) -> Result<(), String> {
    let use_stdio = use_stdio.unwrap_or(true);

    let mut builder = Client::builder()
        .use_stdio(use_stdio)
        .log_level(LogLevel::Error);

    if let Some(url) = cli_url {
        builder = builder.cli_url(url);
    } else if let Some(cli_path) = find_copilot_cli_path() {
        builder = builder.cli_path(cli_path);
    }

    // In production builds the app inherits a minimal PATH that often does
    // not include directories where `node` lives. The copilot CLI is a
    // Node.js script (#!/usr/bin/env node), so the spawned process needs
    // node on its PATH. Augment PATH with common Node/tool directories.
    builder = builder.env("PATH", augmented_path());

    let client = builder
        .build()
        .map_err(|e| format!("Failed to build Copilot client: {}", e))?;
    client
        .start()
        .await
        .map_err(|e| format!("Failed to start Copilot client: {}", e))?;

    let mut guard = COPILOT_CLIENT.lock().await;
    *guard = Some(client);

    Ok(())
}

/// Stop the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_stop() -> Result<(), String> {
    let client = {
        let mut guard = COPILOT_CLIENT.lock().await;
        guard.take()
    };

    if let Some(client) = client {
        let stop_errors = client.stop().await;
        if !stop_errors.is_empty() {
            return Err(format!("Failed to stop Copilot client: {:?}", stop_errors));
        }
    }

    Ok(())
}

/// Check if the Copilot SDK client is running
#[tauri::command]
pub async fn copilot_sdk_is_running() -> Result<bool, String> {
    let guard = COPILOT_CLIENT.lock().await;
    Ok(guard.is_some())
}

/// List available GitHub Copilot models
#[tauri::command]
pub async fn copilot_sdk_list_models() -> Result<Vec<CopilotModelInfo>, String> {
    let guard = COPILOT_CLIENT.lock().await;
    let client = guard.as_ref().ok_or("Copilot client not started")?;
    let models = client
        .list_models()
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;

    Ok(models
        .iter()
        .map(|m| CopilotModelInfo {
            id: m.id.clone(),
            name: m.name.clone(),
        })
        .collect())
}

/// Get GitHub Copilot authentication status
#[tauri::command]
pub async fn copilot_sdk_get_auth_status() -> Result<CopilotAuthStatus, String> {
    let guard = COPILOT_CLIENT.lock().await;
    let client = guard.as_ref().ok_or("Copilot client not started")?;
    let status = client
        .get_auth_status()
        .await
        .map_err(|e| format!("Failed to get auth status: {}", e))?;

    Ok(CopilotAuthStatus {
        authenticated: status.is_authenticated,
        login: status.login.clone(),
    })
}

// =============================================================================
// Specialized Agents Configuration
// =============================================================================

/// Specialized agent type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecializedAgentType {
    General,
    Frontend,
    Backend,
}

/// System prompts for specialized agents — delegate to the shared prompts module
/// in `flow_like::copilot::prompts` for consistency between bits and SDK paths.
fn frontend_agent_prompt() -> String {
    flow_like::copilot::prompts::frontend_sdk_system_prompt()
}

fn backend_agent_prompt() -> String {
    flow_like::copilot::prompts::board_sdk_system_prompt()
}

fn general_agent_prompt() -> String {
    flow_like::copilot::prompts::general_system_prompt()
}

/// Get the system prompt for a specialized agent
fn get_agent_prompt(agent_type: &SpecializedAgentType) -> String {
    match agent_type {
        SpecializedAgentType::General => general_agent_prompt(),
        SpecializedAgentType::Frontend => frontend_agent_prompt(),
        SpecializedAgentType::Backend => backend_agent_prompt(),
    }
}

/// Create a session with a specialized agent using Copilot SDK
#[tauri::command]
pub async fn copilot_sdk_create_agent_session(
    agent_type: SpecializedAgentType,
    model_id: Option<String>,
) -> Result<String, String> {
    let guard = COPILOT_CLIENT.lock().await;
    let client = guard.as_ref().ok_or("Copilot client not started")?;

    let system_prompt = get_agent_prompt(&agent_type);

    let config = copilot_sdk::SessionConfig {
        model: model_id,
        streaming: true,
        system_message: Some(copilot_sdk::SystemMessageConfig {
            content: Some(system_prompt),
            mode: Some(copilot_sdk::SystemMessageMode::Append),
        }),
        infinite_sessions: Some(copilot_sdk::InfiniteSessionConfig::enabled()),
        ..Default::default()
    };

    let session = client
        .create_session(config)
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    Ok(session.session_id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_client() -> Option<Client> {
        let cli_path = find_copilot_cli_path();
        if cli_path.is_none() {
            eprintln!("SKIP: copilot CLI not found");
            return None;
        }

        let mut builder = Client::builder().use_stdio(true).log_level(LogLevel::Error);

        if let Some(path) = cli_path {
            builder = builder.cli_path(path);
        }
        builder = builder.env("PATH", augmented_path());

        Some(builder.build().expect("Client::builder().build() failed"))
    }

    async fn start_test_client() -> Option<Client> {
        let client = build_test_client()?;
        match client.start().await {
            Ok(()) => Some(client),
            Err(e) => {
                let err_str = format!("{:?}", e);
                if err_str.contains("ProtocolMismatch") {
                    eprintln!(
                        "SKIP: protocol mismatch — SDK expects v{}, CLI reports v3. \
                         Update copilot-sdk dependency.",
                        copilot_sdk::SDK_PROTOCOL_VERSION
                    );
                } else {
                    eprintln!("SKIP: client.start() failed: {}", err_str);
                }
                None
            }
        }
    }

    #[test]
    fn extra_bin_dirs_contains_common_locations() {
        let dirs = extra_bin_dirs();
        assert!(!dirs.is_empty(), "extra_bin_dirs should not be empty");

        let paths_str: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        let has_homebrew = paths_str.iter().any(|p| p.contains("homebrew"));
        let has_usr_local = paths_str.iter().any(|p| p.contains("/usr/local/bin"));
        assert!(
            has_homebrew || has_usr_local,
            "Should include /opt/homebrew/bin or /usr/local/bin. Got: {:?}",
            paths_str
        );
    }

    #[test]
    fn augmented_path_includes_existing_dirs() {
        let path = augmented_path();
        assert!(!path.is_empty(), "augmented_path should not be empty");
        // Must contain original PATH
        let current = std::env::var("PATH").unwrap_or_default();
        assert!(
            path.contains(&current),
            "augmented PATH should contain original PATH"
        );
    }

    #[test]
    fn augmented_path_has_node_accessible() {
        let path = augmented_path();
        let found_node = path.split(':').any(|dir| {
            let candidate = std::path::Path::new(dir).join("node");
            candidate.exists()
        });
        assert!(
            found_node,
            "augmented PATH should include a directory containing `node`. PATH = {}",
            path
        );
    }

    #[test]
    fn find_copilot_cli_resolves() {
        let cli_path = find_copilot_cli_path();
        assert!(
            cli_path.is_some(),
            "find_copilot_cli_path() returned None — the `copilot` CLI binary is not installed or not on PATH. \
             Searched in: {:?}",
            extra_bin_dirs()
                .iter()
                .filter(|d| d.exists())
                .collect::<Vec<_>>()
        );
        if let Some(ref p) = cli_path {
            assert!(
                p.exists(),
                "resolved copilot CLI path does not exist: {:?}",
                p
            );
        }
    }

    #[tokio::test]
    async fn copilot_sdk_client_starts_and_stops() {
        let Some(client) = build_test_client() else {
            return;
        };

        let start_result = client.start().await;

        if let Err(ref e) = start_result {
            let err_str = format!("{:?}", e);
            if err_str.contains("ProtocolMismatch") {
                panic!(
                    "COPILOT SDK PROTOCOL MISMATCH: The copilot-sdk Rust crate (protocol v{}) \
                     is incompatible with the installed Copilot CLI (protocol v3). \
                     Update the copilot-sdk dependency in Cargo.toml to a version supporting \
                     protocol v3. Error: {}",
                    copilot_sdk::SDK_PROTOCOL_VERSION,
                    err_str
                );
            }
            panic!("client.start() failed: {:?}", e);
        }

        let stop_errors = client.stop().await;
        assert!(
            stop_errors.is_empty(),
            "client.stop() had errors: {:?}",
            stop_errors
        );
    }

    #[tokio::test]
    async fn copilot_sdk_auth_status() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let auth = client.get_auth_status().await;
        assert!(auth.is_ok(), "get_auth_status() failed: {:?}", auth.err());

        let status = auth.unwrap();
        println!(
            "Auth status: authenticated={}, login={:?}",
            status.is_authenticated, status.login
        );
        assert!(
            status.is_authenticated,
            "Copilot is not authenticated. Run `copilot auth login` first."
        );

        let _ = client.stop().await;
    }

    #[tokio::test]
    async fn copilot_sdk_list_models() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let models = client.list_models().await;
        assert!(models.is_ok(), "list_models() failed: {:?}", models.err());

        let models = models.unwrap();
        println!("Available models ({}):", models.len());
        for m in &models {
            println!("  - {} ({})", m.name, m.id);
        }
        assert!(
            !models.is_empty(),
            "No models returned from Copilot SDK — check subscription/auth"
        );

        let _ = client.stop().await;
    }

    #[tokio::test]
    async fn copilot_sdk_create_session_and_chat() {
        let Some(client) = start_test_client().await else {
            return;
        };

        let config = copilot_sdk::SessionConfig {
            streaming: true,
            ..Default::default()
        };

        let session = client.create_session(config).await;
        assert!(
            session.is_ok(),
            "create_session() failed: {:?}",
            session.err()
        );
        let session = session.unwrap();

        let mut events = session.subscribe();
        let send_result = session.send("Reply with only the word 'pong'").await;
        assert!(
            send_result.is_ok(),
            "session.send() failed: {:?}",
            send_result.err()
        );

        let mut got_response = false;
        let mut full_response = String::new();
        let timeout = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                match events.recv().await {
                    Ok(event) => match &event.data {
                        copilot_sdk::SessionEventData::AssistantMessageDelta(delta) => {
                            full_response.push_str(&delta.delta_content);
                        }
                        copilot_sdk::SessionEventData::AssistantMessage(msg) => {
                            if full_response.is_empty() {
                                full_response = msg.content.clone();
                            }
                            got_response = true;
                        }
                        copilot_sdk::SessionEventData::SessionIdle(_) => break,
                        copilot_sdk::SessionEventData::SessionError(err) => {
                            panic!("Session error: {:?}", err);
                        }
                        _ => {}
                    },
                    Err(e) => {
                        panic!("Event receive error: {}", e);
                    }
                }
            }
        })
        .await;

        assert!(timeout.is_ok(), "Chat timed out after 30s");
        assert!(
            !full_response.is_empty(),
            "Got empty response from Copilot session"
        );
        println!("Chat response: {}", full_response);

        let _ = client.stop().await;
    }
}
