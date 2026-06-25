use crate::state::{TauriFlowLikeState, TauriSettingsState};
use async_trait::async_trait;
use flow_like::a2ui::SurfaceComponent;
use flow_like::copilot::{
    ChatImage, CopilotScope, UIActionContext, UnifiedChatMessage, UnifiedContext, UnifiedCopilot,
    UnifiedCopilotResponse,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{
    BoardCommand, CatalogProvider, GraphContext, NodeMetadata, PinMetadata, RunContext,
    enrich_node_metadata, score_catalog_metadata,
};
use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like_catalog::get_catalog;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, State, ipc::Channel};

/// Desktop implementation of the catalog provider for node search
struct DesktopCatalogProvider {
    nodes: Arc<Vec<Node>>,
}

impl DesktopCatalogProvider {
    fn new(injected_nodes: Option<Vec<Node>>) -> Self {
        let mut nodes = static_catalog_nodes();

        if let Some(injected_nodes) = injected_nodes {
            let mut wasm_node_keys: HashSet<(String, String)> = nodes
                .iter()
                .filter_map(|node| {
                    node.wasm
                        .as_ref()
                        .map(|wasm| (wasm.package_id.clone(), node.name.clone()))
                })
                .collect();

            for node in injected_nodes {
                let Some(wasm) = node.wasm.as_ref() else {
                    continue;
                };

                if wasm_node_keys.insert((wasm.package_id.clone(), node.name.clone())) {
                    nodes.push(node);
                }
            }
        }

        Self {
            nodes: Arc::new(nodes),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }
}

fn static_catalog_nodes() -> Vec<Node> {
    get_catalog()
        .into_iter()
        .map(|logic| logic.get_node())
        .collect()
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
        default_value: p
            .default_value
            .as_ref()
            .map(|value| String::from_utf8_lossy(value).to_string())
            .filter(|value| !value.is_empty() && value != "null"),
        schema: p.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

fn node_to_metadata(node: &Node) -> NodeMetadata {
    let derived_category = node
        .name
        .to_lowercase()
        .split("::")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let category = if derived_category.is_empty() {
        node.category.clone()
    } else {
        derived_category
    };

    let mut inputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Input)
        .collect();
    inputs.sort_by_key(|p| (p.index, p.name.clone()));

    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output)
        .collect();
    outputs.sort_by_key(|p| (p.index, p.name.clone()));

    enrich_node_metadata(NodeMetadata {
        name: node.name.clone(),
        friendly_name: node.friendly_name.clone(),
        description: node.description.clone(),
        inputs: inputs.into_iter().map(pin_to_metadata).collect(),
        outputs: outputs.into_iter().map(pin_to_metadata).collect(),
        category: Some(category),
        required_inputs: Vec::new(),
        companion_nodes: Vec::new(),
        capability_tags: Vec::new(),
    })
}

#[async_trait]
impl CatalogProvider for DesktopCatalogProvider {
    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for node in self.nodes.iter() {
            let metadata = node_to_metadata(node);
            let score = score_catalog_metadata(&metadata, query);

            if score > 0 {
                scored_matches.push((score, metadata));
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
        let pin_type = pin_type.to_lowercase();
        let mut matches = Vec::new();

        for node in self.nodes.iter() {
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
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 10 {
                break;
            }
        }
        matches
    }

    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata> {
        let category_prefix = category_prefix.to_lowercase();
        let mut matches = Vec::new();

        for node in self.nodes.iter() {
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            if category.starts_with(&category_prefix) || name_lower.contains(&category_prefix) {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
        self.nodes
            .iter()
            .find(|node| node.name == node_type)
            .map(node_to_metadata)
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        self.nodes.iter().map(|node| node.name.clone()).collect()
    }

    async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
        self.nodes.iter().map(node_to_metadata).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentBackendKind {
    GithubCopilot,
    Codex,
    ClaudeCode,
}

impl FlowPilotAgentBackendKind {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "copilot" | "github" | "github-copilot" | "github_copilot" => Some(Self::GithubCopilot),
            "codex" | "openai-codex" | "openai_codex" => Some(Self::Codex),
            "claude" | "claude-code" | "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    fn from_model_prefix(value: &str) -> Option<(Self, &str)> {
        for (prefix, backend) in [
            ("copilot:", Self::GithubCopilot),
            ("github-copilot:", Self::GithubCopilot),
            ("codex:", Self::Codex),
            ("claude-code:", Self::ClaudeCode),
            ("claude:", Self::ClaudeCode),
        ] {
            if let Some(model_id) = value.strip_prefix(prefix) {
                return Some((backend, model_id));
            }
        }

        None
    }

    fn label(self) -> &'static str {
        match self {
            Self::GithubCopilot => "GitHub Copilot",
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }

    fn cli_name(self) -> &'static str {
        match self {
            Self::GithubCopilot => "copilot",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
        }
    }

    fn env_path_var(self) -> &'static str {
        match self {
            Self::GithubCopilot => "COPILOT_CLI_PATH",
            Self::Codex => "CODEX_CLI_PATH",
            Self::ClaudeCode => "CLAUDE_CODE_CLI_PATH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowPilotChatBackend {
    Bits,
    Agent(FlowPilotAgentBackendKind),
}

#[derive(Debug, Clone)]
struct FlowPilotModelSelection {
    backend: FlowPilotChatBackend,
    model_id: Option<String>,
}

impl FlowPilotModelSelection {
    fn parse(model_id: Option<String>) -> Self {
        let Some(model_id) = model_id else {
            return Self {
                backend: FlowPilotChatBackend::Bits,
                model_id: None,
            };
        };

        if let Some((backend, stripped_model_id)) =
            FlowPilotAgentBackendKind::from_model_prefix(&model_id)
        {
            return Self {
                backend: FlowPilotChatBackend::Agent(backend),
                model_id: Some(stripped_model_id.to_string()),
            };
        }

        Self {
            backend: FlowPilotChatBackend::Bits,
            model_id: Some(model_id),
        }
    }
}

fn copilot_attachment_extension(media_type: &str) -> &'static str {
    match media_type.to_lowercase().as_str() {
        "image/jpeg" | "jpeg" | "jpg" => "jpg",
        "image/png" | "png" => "png",
        "image/gif" | "gif" => "gif",
        "image/webp" | "webp" => "webp",
        _ => "bin",
    }
}

fn build_copilot_attachments(images: &[ChatImage]) -> Result<Vec<UserMessageAttachment>, String> {
    use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};

    let attachment_dir = std::env::temp_dir().join("flow-like-copilot-attachments");
    std::fs::create_dir_all(&attachment_dir)
        .map_err(|e| format!("Failed to create Copilot attachment directory: {}", e))?;

    images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            let bytes = STANDARD
                .decode(&image.data)
                .map_err(|e| format!("Failed to decode prompt image {}: {}", index + 1, e))?;
            let extension = copilot_attachment_extension(&image.media_type);
            let file_name = format!("{}.{}", blake3::hash(&bytes).to_hex(), extension);
            let file_path = attachment_dir.join(file_name);

            if !file_path.exists() {
                std::fs::write(&file_path, &bytes).map_err(|e| {
                    format!(
                        "Failed to write Copilot attachment {}: {}",
                        file_path.display(),
                        e
                    )
                })?;
            }

            Ok(UserMessageAttachment {
                attachment_type: AttachmentType::File,
                path: file_path.to_string_lossy().into_owned(),
                display_name: format!("prompt-image-{}.{}", index + 1, extension),
            })
        })
        .collect()
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
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: Option<Vec<String>>,
    // UI context (optional for Board scope)
    current_surface: Option<Vec<SurfaceComponent>>,
    selected_component_ids: Option<Vec<String>>,
    // Common parameters
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    token: Option<String>,
    // Extended context
    run_context: Option<RunContext>,
    action_context: Option<UIActionContext>,
    // Streaming channel
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let model_selection = FlowPilotModelSelection::parse(model_id);
    if let FlowPilotChatBackend::Agent(agent_backend) = model_selection.backend {
        return match agent_backend {
            FlowPilotAgentBackendKind::GithubCopilot => {
                let model_id = model_selection
                    .model_id
                    .as_deref()
                    .filter(|model_id| !model_id.trim().is_empty())
                    .ok_or_else(|| "GitHub Copilot backend requires a model id".to_string())?;

                copilot_sdk_chat_internal(
                    app_handle.clone(),
                    model_id,
                    scope,
                    board.as_ref(),
                    catalog_nodes,
                    selected_node_ids.as_deref().unwrap_or(&[]),
                    current_surface.as_ref(),
                    user_prompt,
                    current_images,
                    history.unwrap_or_default(),
                    channel,
                )
                .await
            }
            FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
                let model_id = model_selection
                    .model_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string());

                external_code_agent_chat_internal(
                    app_handle.clone(),
                    agent_backend,
                    &model_id,
                    scope,
                    board.as_ref(),
                    catalog_nodes,
                    selected_node_ids.as_deref().unwrap_or(&[]),
                    current_surface.as_ref(),
                    user_prompt,
                    history.unwrap_or_default(),
                    channel,
                )
                .await
            }
        };
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
        _ => Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes))),
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
            current_images,
            history,
            model_selection.model_id,
            token,
            context,
            on_token,
        )
        .await
        .map_err(|e| e.to_string())
}

async fn external_code_agent_chat_internal(
    app_handle: AppHandle,
    backend: FlowPilotAgentBackendKind,
    model_id: &str,
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    user_prompt: String,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let mut surface = build_flowpilot_agent_surface(
        scope,
        board,
        catalog_nodes,
        selected_node_ids,
        current_surface,
        &history,
        &user_prompt,
    );
    surface.capabilities.tool_protocol = FlowPilotAgentTransportKind::Mcp;

    let cli = find_cli_resolution(backend, Some(&app_handle)).ok_or_else(|| {
        format!(
            "{} CLI was not found. Install it or set {} to the executable path.",
            backend.label(),
            backend.env_path_var()
        )
    })?;

    let tools = build_flowpilot_sdk_tools(app_handle, scope, &surface);
    let tool_names = tools
        .iter()
        .map(|(tool, _)| tool.name.clone())
        .collect::<Vec<_>>();
    let tool_name_summary = tool_names.join(", ");
    send_external_progress_event(
        &channel,
        "external-agent",
        &format!(
            "Starting {} with shared FlowPilot MCP tools: {}",
            backend.label(),
            tool_name_summary
        ),
    );

    let mcp_bridge = FlowPilotMcpBridge::start(tools).await?;
    let prompt = build_external_agent_prompt(&surface.system_content, &user_prompt);
    let invocation =
        ExternalAgentInvocation::new(backend, cli, model_id, &mcp_bridge.url, prompt, tool_names)?;

    send_external_progress_event(
        &channel,
        "external-agent",
        &format!("Using {} via {}", backend.label(), mcp_bridge.url),
    );

    let agent_result = run_external_agent_invocation(invocation, channel).await;
    mcp_bridge.shutdown().await;
    let agent_output = agent_result?;

    Ok(UnifiedCopilotResponse {
        message: if agent_output.trim().is_empty() {
            format!(
                "{} completed without a final text response.",
                backend.label()
            )
        } else {
            agent_output
        },
        commands: drain_side_effect_commands(&surface.side_effect_commands),
        suggestions: Vec::new(),
        components: Vec::new(),
        canvas_settings: None,
        root_component_id: None,
        flowscript_workspace: None,
        active_scope: scope,
    })
}

/// Internal function to handle Copilot SDK chat
async fn copilot_sdk_chat_internal(
    app_handle: AppHandle,
    model_id: &str,
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    use copilot_sdk::SessionEventData;

    const MAX_WORKFLOW_IDLE_CONTINUATIONS: u8 = 2;

    let guard = COPILOT_CLIENT.lock().await;
    let client = guard
        .as_ref()
        .ok_or("Copilot SDK not running. Please start it first.")?;
    let original_user_prompt = user_prompt.clone();

    let surface = build_flowpilot_agent_surface(
        scope,
        board,
        catalog_nodes,
        selected_node_ids,
        current_surface,
        &history,
        &original_user_prompt,
    );
    let side_effect_commands = surface.side_effect_commands.clone();
    let workflow_edit_request = surface.workflow_edit_request;

    let tools = build_flowpilot_sdk_tools(app_handle, scope, &surface);

    // Extract just the Tool definitions for SessionConfig
    let tool_defs: Vec<copilot_sdk::Tool> = tools.iter().map(|(t, _)| t.clone()).collect();

    // Names of our reviewed custom tools. The CLI may surface a permission request for these
    // before running them; we approve those and deny everything else (built-in file/shell tools).
    let allowed_tool_names: std::collections::HashSet<String> =
        tool_defs.iter().map(|t| t.name.clone()).collect();
    let available_tools = Some(allowed_tool_names.iter().cloned().collect::<Vec<_>>());
    let permission_allowed_tool_names = allowed_tool_names.clone();

    // Whitelist reviewed custom tools and also exclude known built-ins as a defense in depth.
    // This keeps FlowPilot in its virtual workflow/UI workspace and prevents file/shell draft
    // attempts from surfacing as permission errors.
    let excluded_tools = Some(vec![
        "Read".to_string(),
        "Edit".to_string(),
        "Write".to_string(),
        "Glob".to_string(),
        "LS".to_string(),
        "Task".to_string(),
        "WebFetch".to_string(),
        "WebSearch".to_string(),
        "NotebookEdit".to_string(),
        "shell".to_string(),
        "powershell".to_string(),
        "bash".to_string(),
        "Grep".to_string(),
        "listDir".to_string(),
        "list_dir".to_string(),
        "read_file".to_string(),
        "write_file".to_string(),
        "edit_file".to_string(),
        "create_file".to_string(),
        "Search".to_string(),
        "Insert".to_string(),
        "Replace".to_string(),
        "CreateFile".to_string(),
    ]);

    let config = copilot_sdk::SessionConfig {
        model: Some(model_id.to_string()),
        streaming: true,
        tools: tool_defs,
        available_tools,
        excluded_tools,
        request_permission: Some(true),
        system_message: Some(copilot_sdk::SystemMessageConfig {
            content: Some(surface.system_content),
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

    // FlowPilot only exposes reviewed custom tools. Approve permission requests for those
    // tools (the CLI surfaces one before invoking them) and deny anything else so built-in
    // file/shell tools cannot run.
    session
        .register_permission_handler(move |req| {
            let tool_name = req.extension_data.get("toolName").and_then(|v| v.as_str());
            match tool_name {
                Some(name) if permission_allowed_tool_names.contains(name) => {
                    copilot_sdk::PermissionRequestResult::approved()
                }
                _ => copilot_sdk::PermissionRequestResult::denied(),
            }
        })
        .await;

    let mut events = session.subscribe();
    let attachments = current_images
        .as_ref()
        .filter(|images| !images.is_empty())
        .map(|images| build_copilot_attachments(images))
        .transpose()?;

    session
        .send(MessageOptions {
            prompt: user_prompt,
            attachments,
            mode: None,
        })
        .await
        .map_err(|e| format!("Failed to send message: {}", e))?;

    let mut full_response = String::new();
    let mut extracted_commands: Vec<BoardCommand> = Vec::new();
    let mut extracted_components: Vec<SurfaceComponent> = Vec::new();
    let mut extracted_canvas_settings: Option<serde_json::Value> = None;
    let mut extracted_root_component_id: Option<String> = None;
    let mut extracted_flowscript_workspace: Option<String> = None;
    let mut last_validated_commands: Option<Vec<BoardCommand>> = None;
    let mut last_validated_components: Option<(
        Vec<SurfaceComponent>,
        Option<serde_json::Value>,
        Option<String>,
    )> = None;
    let mut workflow_idle_continuations = 0u8;
    let mut tool_names_by_call_id: HashMap<String, String> = HashMap::new();

    loop {
        match events.recv().await {
            Ok(event) => match &event.data {
                SessionEventData::AssistantMessageDelta(delta) => {
                    full_response.push_str(&delta.delta_content);
                    if !workflow_edit_request {
                        let _ = channel.send(delta.delta_content.clone());
                    }
                }
                SessionEventData::AssistantMessage(msg) => {
                    // Don't overwrite accumulated content unless it's truly final
                    if full_response.is_empty() {
                        full_response = msg.content.clone();
                    }
                }
                SessionEventData::ToolExecutionStart(tool_event) => {
                    tool_names_by_call_id.insert(
                        tool_event.tool_call_id.clone(),
                        tool_event.tool_name.clone(),
                    );

                    if tool_event.tool_name == "edit_flowscript"
                        && let Some(arguments) = &tool_event.arguments
                        && let Some(workspace) =
                            arguments.get("flowscript").and_then(|value| value.as_str())
                    {
                        extracted_flowscript_workspace = Some(workspace.to_string());
                        let payload = serde_json::json!({
                            "source": workspace,
                            "status": "submitted",
                        });
                        let workspace_event = format!(
                            "<flowscript_workspace>{}</flowscript_workspace>",
                            serde_json::to_string(&payload).unwrap_or_default()
                        );
                        let _ = channel.send(workspace_event);
                    }

                    // Send tool start event to frontend
                    send_stream_json_event(
                        &channel,
                        "tool_start",
                        &serde_json::json!({
                            "tool_call_id": tool_event.tool_call_id,
                            "tool": tool_event.tool_name,
                            "status": "running",
                            "summary": summarize_tool_arguments(&tool_event.tool_name, tool_event.arguments.as_ref()),
                            "arguments_preview": preview_tool_arguments(&tool_event.tool_name, tool_event.arguments.as_ref()),
                        }),
                    );
                }
                SessionEventData::ToolExecutionProgress(progress) => {
                    send_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": progress.tool_call_id,
                            "message": progress.progress_message,
                        }),
                    );
                }
                SessionEventData::ToolExecutionPartialResult(partial) => {
                    send_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": partial.tool_call_id,
                            "message": truncate_for_preview(&partial.partial_output, 1200),
                        }),
                    );
                }
                SessionEventData::ToolExecutionComplete(tool_complete) => {
                    let completed_tool_name = tool_names_by_call_id
                        .get(&tool_complete.tool_call_id)
                        .cloned()
                        .or_else(|| tool_complete.mcp_tool_name.clone())
                        .unwrap_or_else(|| "tool".to_string());
                    let result_content = tool_complete
                        .result
                        .as_ref()
                        .map(|result| result.content.as_str());

                    if let Some(ref result) = tool_complete.result
                        && let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&result.content)
                    {
                        let status = parsed.get("status").and_then(|s| s.as_str());

                        if let Some(workspace) = parsed
                            .get("flowscript_workspace")
                            .and_then(|value| value.as_str())
                        {
                            extracted_flowscript_workspace = Some(workspace.to_string());
                            let payload = serde_json::json!({
                                "source": workspace,
                                "status": status.unwrap_or("unknown"),
                            });
                            let workspace_event = format!(
                                "<flowscript_workspace>{}</flowscript_workspace>",
                                serde_json::to_string(&payload).unwrap_or_default()
                            );
                            let _ = channel.send(workspace_event);
                        }

                        // Some models, especially Claude/Sonnet variants, stop after a
                        // successful validate_* call. Remember valid payloads so idle
                        // handling can still surface the reviewable action to the board.
                        if status == Some("valid") {
                            if let Some(cmds) = parsed.get("commands")
                                && let Ok(commands) =
                                    serde_json::from_value::<Vec<BoardCommand>>(cmds.clone())
                            {
                                last_validated_commands = Some(commands);
                            }

                            if let Some(comps) = parsed.get("components")
                                && let Ok(components) =
                                    serde_json::from_value::<Vec<SurfaceComponent>>(comps.clone())
                            {
                                let canvas = parsed.get("canvasSettings").cloned();
                                let root_id = parsed
                                    .get("rootComponentId")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                                last_validated_components = Some((components, canvas, root_id));
                            }
                        } else if status == Some("validation_errors") {
                            if parsed.get("commands").is_some() {
                                last_validated_commands = None;
                            }
                            if parsed.get("components").is_some() {
                                last_validated_components = None;
                            }
                        }

                        // Extract commands from emit_commands tool (status: "queued")
                        if status == Some("queued")
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
                            last_validated_commands = None;
                        }
                        // Extract components from emit_ui tool (status: "rendered")
                        if status == Some("rendered") {
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
                                last_validated_components = None;
                            }
                        }
                    }

                    // Send tool completion event to frontend
                    let result_status = result_content.and_then(extract_json_status);
                    let status = if !tool_complete.success {
                        "error"
                    } else if completed_tool_name == "edit_flowscript"
                        && matches!(
                            result_status.as_deref(),
                            Some("validation_errors" | "no_changes")
                        )
                    {
                        "error"
                    } else {
                        "done"
                    };
                    let error_message = tool_complete.error.as_ref().map(|error| {
                        if error.message.is_empty() {
                            "Tool failed".to_string()
                        } else {
                            error.message.clone()
                        }
                    });
                    send_stream_json_event(
                        &channel,
                        "tool_end",
                        &serde_json::json!({
                            "tool_call_id": tool_complete.tool_call_id,
                            "tool": completed_tool_name,
                            "status": status,
                            "result_status": result_status,
                            "result_summary": summarize_tool_result(result_content, error_message.as_deref()),
                            "result_preview": result_content.map(|content| preview_tool_result(content)),
                            "error": error_message,
                        }),
                    );
                }
                SessionEventData::SessionIdle(_) => {
                    if extracted_commands.is_empty()
                        && let Some(commands) = last_validated_commands.take()
                    {
                        send_commands_event(&channel, &commands);
                        extracted_commands.extend(commands);
                    }

                    if extracted_commands.is_empty() {
                        let commands = drain_side_effect_commands(&side_effect_commands);
                        if !commands.is_empty() {
                            send_commands_event(&channel, &commands);
                            extracted_commands.extend(commands);
                        }
                    }

                    if extracted_components.is_empty()
                        && let Some((components, canvas_settings, root_component_id)) =
                            last_validated_components.take()
                    {
                        let comp_event = format!(
                            "<components>{}</components>",
                            serde_json::to_string(&components).unwrap_or_default()
                        );
                        let _ = channel.send(comp_event);

                        if let Some(canvas) = canvas_settings {
                            let canvas_event = format!(
                                "<canvas_settings>{}</canvas_settings>",
                                serde_json::to_string(&canvas).unwrap_or_default()
                            );
                            let _ = channel.send(canvas_event);
                            extracted_canvas_settings = Some(canvas);
                        }

                        extracted_root_component_id = root_component_id;
                        extracted_components = components;
                    }

                    if workflow_edit_request
                        && extracted_commands.is_empty()
                        && workflow_idle_continuations < MAX_WORKFLOW_IDLE_CONTINUATIONS
                    {
                        workflow_idle_continuations = workflow_idle_continuations.saturating_add(1);
                        full_response.clear();
                        let prompt = workflow_edit_continuation_prompt(
                            &original_user_prompt,
                            extracted_flowscript_workspace.as_deref(),
                            workflow_idle_continuations,
                        );
                        session
                            .send(MessageOptions {
                                prompt,
                                attachments: None,
                                mode: None,
                            })
                            .await
                            .map_err(|e| {
                                format!("Failed to continue workflow edit session: {}", e)
                            })?;
                        continue;
                    }

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

    if extracted_commands.is_empty() {
        let commands = drain_side_effect_commands(&side_effect_commands);
        if !commands.is_empty() {
            send_commands_event(&channel, &commands);
            extracted_commands.extend(commands);
        }
    }

    // ── Fallback: if the model didn't call emit_ui but dumped JSON in the
    // response text, extract components from there so they still show up.
    if extracted_components.is_empty()
        && matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
    {
        let surface = flow_like::a2ui::copilot::extract_surface_from_response(&full_response);
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

    let final_message = if workflow_edit_request {
        if !extracted_commands.is_empty() {
            "Queued workflow changes for review. Fill placeholder secrets before running."
                .to_string()
        } else if extracted_flowscript_workspace.is_some() {
            "FlowScript draft needs attention: edit_flowscript did not derive board commands. Check the latest validation/result details in the process log, then revise the workspace and submit again."
                .to_string()
        } else {
            "FlowPilot could not produce board commands or a FlowScript draft for this request."
                .to_string()
        }
    } else {
        full_response
    };

    Ok(UnifiedCopilotResponse {
        message: final_message,
        commands: extracted_commands,
        suggestions: vec![],
        components: extracted_components,
        canvas_settings: extracted_canvas_settings,
        root_component_id: extracted_root_component_id,
        flowscript_workspace: extracted_flowscript_workspace,
        active_scope: scope,
    })
}

fn send_stream_json_event(channel: &Channel<String>, tag: &str, payload: &serde_json::Value) {
    let event = format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(payload).unwrap_or_default()
    );
    let _ = channel.send(event);
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            result.push_str("...");
            break;
        }
        result.push(ch);
    }
    result
}

fn line_count(value: &str) -> usize {
    value.lines().count().max(usize::from(!value.is_empty()))
}

fn summarize_tool_arguments(tool_name: &str, arguments: Option<&serde_json::Value>) -> String {
    let Some(arguments) = arguments else {
        return "No arguments".to_string();
    };

    match tool_name {
        "get_declarations" | "catalog_search" | "search_by_pin" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching".to_string()),
        "internet_search" => arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(|query| format!("query: {query}"))
            .unwrap_or_else(|| "Searching web".to_string()),
        "database_tool" | "storage_tool" => arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(|operation| {
                let target = arguments
                    .get("table_name")
                    .or_else(|| arguments.get("tableName"))
                    .or_else(|| arguments.get("path"))
                    .or_else(|| arguments.get("prefix"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if target.is_empty() {
                    operation.to_string()
                } else {
                    format!("{operation}: {target}")
                }
            })
            .unwrap_or_else(|| "Preparing frontend operation".to_string()),
        "execute_event" => arguments
            .get("event_id")
            .or_else(|| arguments.get("eventId"))
            .and_then(|value| value.as_str())
            .map(|event_id| format!("event: {event_id}"))
            .unwrap_or_else(|| "Executing event".to_string()),
        "ask_user" => arguments
            .get("question")
            .and_then(|value| value.as_str())
            .map(|question| truncate_for_preview(question, 180))
            .unwrap_or_else(|| "Requesting user input".to_string()),
        "edit_flowscript" => arguments
            .get("flowscript")
            .and_then(|value| value.as_str())
            .map(|flowscript| {
                format!(
                    "{} lines, {} chars",
                    line_count(flowscript),
                    flowscript.chars().count()
                )
            })
            .unwrap_or_else(|| "Submitting FlowScript".to_string()),
        "emit_commands" | "validate_commands" => arguments
            .get("commands")
            .and_then(|value| value.as_array())
            .map(|commands| format!("{} command(s)", commands.len()))
            .unwrap_or_else(|| "Preparing commands".to_string()),
        "emit_ui" | "validate_ui" => arguments
            .get("components")
            .and_then(|value| value.as_array())
            .map(|components| format!("{} component(s)", components.len()))
            .unwrap_or_else(|| "Preparing UI".to_string()),
        _ => preview_tool_arguments(tool_name, Some(arguments)),
    }
}

fn preview_tool_arguments(tool_name: &str, arguments: Option<&serde_json::Value>) -> String {
    let Some(arguments) = arguments else {
        return "{}".to_string();
    };

    let mut preview = arguments.clone();
    if tool_name == "edit_flowscript"
        && let Some(flowscript) = preview.get_mut("flowscript")
        && let Some(source) = flowscript.as_str()
    {
        *flowscript = serde_json::Value::String(format!(
            "<FlowScript: {} lines, {} chars>",
            line_count(source),
            source.chars().count()
        ));
    }

    truncate_for_preview(
        &serde_json::to_string_pretty(&preview).unwrap_or_else(|_| preview.to_string()),
        2200,
    )
}

fn extract_json_status(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(|status| status.as_str().map(str::to_string))
        })
}

fn summarize_tool_result(content: Option<&str>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return error.to_string();
    }

    let Some(content) = content else {
        return "Completed".to_string();
    };

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
        let status = parsed
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("done");
        let command_count = parsed
            .get("commands")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let component_count = parsed
            .get("components")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let error_count = parsed
            .get("errors")
            .and_then(|value| value.as_array())
            .map(Vec::len);
        let diagnostic_count = parsed
            .get("diagnostics")
            .and_then(|value| value.as_array())
            .map(Vec::len);

        let mut parts = vec![status.replace('_', " ")];
        if let Some(count) = command_count {
            parts.push(format!("{count} command(s)"));
        }
        if let Some(count) = component_count {
            parts.push(format!("{count} component(s)"));
        }
        if let Some(count) = error_count.filter(|count| *count > 0) {
            parts.push(format!("{count} error(s)"));
        }
        if let Some(count) = diagnostic_count.filter(|count| *count > 0) {
            parts.push(format!("{count} diagnostic(s)"));
        }
        return parts.join(" · ");
    }

    truncate_for_preview(content.trim(), 240)
}

fn preview_tool_result(content: &str) -> String {
    if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(workspace) = parsed.get_mut("flowscript_workspace")
            && let Some(source) = workspace.as_str()
        {
            *workspace = serde_json::Value::String(format!(
                "<FlowScript: {} lines, {} chars>",
                line_count(source),
                source.chars().count()
            ));
        }
        return truncate_for_preview(
            &serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| parsed.to_string()),
            2800,
        );
    }

    truncate_for_preview(content.trim(), 2800)
}

fn is_workflow_edit_request(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    if is_read_only_workflow_request(&prompt) {
        return false;
    }

    let edit_verbs = [
        "add",
        "apply",
        "automate",
        "build",
        "connect",
        "create",
        "draft",
        "embed",
        "fetch",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "repair",
        "store",
        "translate",
        "update",
        "wire",
    ];
    let workflow_terms = [
        "automation",
        "board",
        "database",
        "db",
        "email",
        "flow",
        "flowscript",
        "gmail",
        "imap",
        "lancedb",
        "mail",
        "node",
        "nodes",
        "open database",
        "pipeline",
        "smtp",
        "vector",
        "workflow",
        "api call",
        "edge",
        "edges",
        "execution",
        "pin",
        "pins",
        "success output",
        "error output",
    ];

    edit_verbs.iter().any(|verb| prompt.contains(verb))
        && workflow_terms.iter().any(|term| prompt.contains(term))
}

fn is_read_only_workflow_request(prompt: &str) -> bool {
    let read_only_terms = [
        "are these",
        "can this",
        "check",
        "debug",
        "diagnose",
        "does this",
        "error",
        "explain",
        "how does",
        "inspect",
        "is this",
        "issue",
        "not working",
        "problem",
        "review",
        "show me",
        "tell me",
        "what does",
        "what is",
        "what's wrong",
        "where",
        "which",
        "why",
    ];
    if !read_only_terms.iter().any(|term| prompt.contains(term)) {
        return false;
    }

    let mutation_terms = [
        "add",
        "apply",
        "automate",
        "build",
        "change",
        "create",
        "delete",
        "draft",
        "fix",
        "generate",
        "insert",
        "make",
        "modify",
        "remove",
        "repair",
        "store",
        "translate",
        "update",
    ];

    !mutation_terms.iter().any(|term| prompt.contains(term))
}

fn drain_side_effect_commands(store: &Arc<StdMutex<Vec<BoardCommand>>>) -> Vec<BoardCommand> {
    match store.lock() {
        Ok(mut commands) => commands.drain(..).collect(),
        Err(_) => Vec::new(),
    }
}

fn build_flowpilot_sdk_tools(
    app_handle: AppHandle,
    scope: CopilotScope,
    surface: &FlowPilotAgentSurface,
) -> Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)> {
    use super::{
        copilot_sdk_tools::{create_board_tools, create_frontend_tools, create_runtime_tools},
        frontend_tool_bridge::FrontendToolBridge,
    };

    let mut tools = match scope {
        CopilotScope::Board => create_board_tools(
            surface.graph_context.clone(),
            surface.board_arc.clone(),
            surface.catalog_provider.clone(),
            Some(surface.side_effect_commands.clone()),
        ),
        CopilotScope::Frontend => create_frontend_tools(),
        CopilotScope::Both => {
            let mut all_tools = create_board_tools(
                surface.graph_context.clone(),
                surface.board_arc.clone(),
                surface.catalog_provider.clone(),
                Some(surface.side_effect_commands.clone()),
            );
            all_tools.extend(create_frontend_tools());
            all_tools
        }
    };
    tools.extend(create_runtime_tools(FrontendToolBridge::new(app_handle)));
    tools
}

#[derive(Clone)]
struct FlowPilotMcpTool {
    definition: copilot_sdk::Tool,
    handler: copilot_sdk::ToolHandler,
}

#[derive(Clone)]
struct FlowPilotMcpServer {
    tools: Arc<HashMap<String, FlowPilotMcpTool>>,
}

impl FlowPilotMcpServer {
    fn new(tools: Arc<HashMap<String, FlowPilotMcpTool>>) -> Self {
        Self { tools }
    }

    fn to_mcp_tool(tool: &copilot_sdk::Tool) -> rmcp::model::Tool {
        let schema = match &tool.parameters_schema {
            serde_json::Value::Object(_) => tool.parameters_schema.clone(),
            _ => serde_json::json!({ "type": "object", "properties": {} }),
        };

        rmcp::model::Tool::new(
            tool.name.clone(),
            tool.description.clone(),
            rmcp::model::object(schema),
        )
    }
}

impl rmcp::ServerHandler for FlowPilotMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(
            "FlowPilot tools share the exact board/frontend/runtime capabilities used by Bits and GitHub Copilot. For workflow changes on an existing board, call get_current_flowscript first, then edit_flowscript with the full edited document. Create workflow functions as FlowScript `function ... { ... }` declarations, not manual command JSON. Use emit_commands only for layout/non-FlowScript changes and answer the user in text when no board/UI edit is required.",
        )
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        let mut tools = self
            .tools
            .values()
            .map(|tool| Self::to_mcp_tool(&tool.definition))
            .collect::<Vec<_>>();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            tools,
            ..Default::default()
        }))
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tools
            .get(name)
            .map(|tool| Self::to_mcp_tool(&tool.definition))
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        let tool_name = request.name.to_string();
        let tool = self.tools.get(tool_name.as_str()).cloned();
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());

        async move {
            let Some(tool) = tool else {
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Unknown FlowPilot tool: {tool_name}"),
                    None,
                ));
            };

            let definition_name = tool.definition.name.clone();
            let handler = tool.handler.clone();
            tracing::debug!(tool = %definition_name, "FlowPilot MCP tool call started");

            let result = tokio::task::spawn_blocking(move || (handler)(&definition_name, &args))
                .await
                .map_err(|error| {
                    rmcp::ErrorData::internal_error(
                        format!("FlowPilot MCP tool task failed: {error}"),
                        None,
                    )
                })?;

            if result.result_type == "error" || result.error.is_some() {
                tracing::warn!(
                    tool = %tool_name,
                    error = ?result.error,
                    "FlowPilot MCP tool call returned an error"
                );
            } else {
                tracing::debug!(tool = %tool_name, "FlowPilot MCP tool call completed");
            }

            Ok(flowpilot_tool_result_to_mcp(result))
        }
    }
}

fn flowpilot_tool_result_to_mcp(
    result: copilot_sdk::ToolResultObject,
) -> rmcp::model::CallToolResult {
    if result.result_type == "error" || result.error.is_some() {
        rmcp::model::CallToolResult::error(vec![rmcp::model::Content::text(
            result
                .error
                .unwrap_or_else(|| result.text_result_for_llm.clone()),
        )])
    } else {
        rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(
            result.text_result_for_llm,
        )])
    }
}

struct FlowPilotMcpBridge {
    url: String,
    cancellation_token: rmcp::transport::streamable_http_server::StreamableHttpServerConfig,
    server_task: tokio::task::JoinHandle<()>,
}

impl FlowPilotMcpBridge {
    async fn start(
        tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)>,
    ) -> Result<Self, String> {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };

        let tools = Arc::new(
            tools
                .into_iter()
                .map(|(definition, handler)| {
                    (
                        definition.name.clone(),
                        FlowPilotMcpTool {
                            definition,
                            handler,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
        );

        let mut config = StreamableHttpServerConfig::default();
        config.stateful_mode = true;
        config.sse_keep_alive = None;
        let cancellation_token = config.clone();
        let service_tools = tools.clone();
        let service: StreamableHttpService<FlowPilotMcpServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(FlowPilotMcpServer::new(service_tools.clone())),
                Default::default(),
                config,
            );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind FlowPilot MCP server: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read FlowPilot MCP address: {e}"))?;
        let shutdown_token = cancellation_token.cancellation_token.clone();
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown_token.cancelled_owned().await })
                .await;
        });

        Ok(Self {
            url: format!("http://{addr}/mcp"),
            cancellation_token,
            server_task,
        })
    }

    async fn shutdown(self) {
        self.cancellation_token.cancellation_token.cancel();
        let _ = self.server_task.await;
    }
}

struct ExternalAgentInvocation {
    backend: FlowPilotAgentBackendKind,
    executable: std::path::PathBuf,
    path_dirs: Vec<PathBuf>,
    args: Vec<String>,
    prompt: String,
    final_output_path: Option<std::path::PathBuf>,
}

impl ExternalAgentInvocation {
    fn new(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
    ) -> Result<Self, String> {
        match backend {
            FlowPilotAgentBackendKind::Codex => {
                Ok(Self::codex(backend, cli, model_id, mcp_url, prompt))
            }
            FlowPilotAgentBackendKind::ClaudeCode => {
                Self::claude(backend, cli, model_id, mcp_url, prompt, tool_names)
            }
            FlowPilotAgentBackendKind::GithubCopilot => Err(
                "GitHub Copilot uses the direct SDK backend, not the external runner.".to_string(),
            ),
        }
    }

    fn codex(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        mcp_url: &str,
        prompt: String,
    ) -> Self {
        // Mirrors @openai/codex-sdk's stdio protocol: spawn
        // `codex exec --experimental-json`, pass config overrides as repeated
        // --config entries, and stream JSONL events from stdout.
        let mut args = vec![
            "exec".to_string(),
            "--experimental-json".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
            "--cd".to_string(),
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .display()
                .to_string(),
            "--skip-git-repo-check".to_string(),
            "--config".to_string(),
            format!("mcp_servers.flowpilot.url={:?}", mcp_url),
            "--config".to_string(),
            "mcp_servers.flowpilot.startup_timeout_sec=10".to_string(),
            "--config".to_string(),
            "mcp_servers.flowpilot.tool_timeout_sec=600".to_string(),
            "--config".to_string(),
            "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\"".to_string(),
            "--config".to_string(),
            "features.use_rmcp_client=true".to_string(),
            "--config".to_string(),
            "approval_policy=\"never\"".to_string(),
        ];
        let allow_explicit_codex_model =
            std::env::var_os("FLOWPILOT_CODEX_ALLOW_EXPLICIT_MODEL").is_some();
        if allow_explicit_codex_model && !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }

        Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            prompt,
            final_output_path: None,
        }
    }

    fn claude(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
    ) -> Result<Self, String> {
        let mcp_config_path = std::env::temp_dir().join(format!(
            "flowpilot-claude-mcp-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "flowpilot": {
                    "type": "http",
                    "url": mcp_url
                }
            }
        });
        std::fs::write(
            &mcp_config_path,
            serde_json::to_vec_pretty(&mcp_config)
                .map_err(|e| format!("Failed to serialize Claude MCP config: {e}"))?,
        )
        .map_err(|e| format!("Failed to write Claude MCP config: {e}"))?;

        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--strict-mcp-config".to_string(),
            "--mcp-config".to_string(),
            mcp_config_path.display().to_string(),
        ];
        if !tool_names.is_empty() {
            let allowed_mcp_tools = tool_names
                .iter()
                .map(|name| format!("mcp__flowpilot__{name}"))
                .collect::<Vec<_>>()
                .join(",");
            args.extend([
                "--tools".to_string(),
                allowed_mcp_tools.clone(),
                "--allowedTools".to_string(),
                allowed_mcp_tools,
                "--permission-mode".to_string(),
                "dontAsk".to_string(),
            ]);
        }
        if !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }
        args.push(prompt.clone());

        Ok(Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            prompt: String::new(),
            final_output_path: Some(mcp_config_path),
        })
    }
}

fn build_external_agent_prompt(system_content: &str, user_prompt: &str) -> String {
    format!(
        r#"SYSTEM INSTRUCTIONS
{system_content}

You are running through an external code-agent CLI connected to FlowPilot's shared MCP tools. Do not use shell/file-edit tools for workflow or UI edits. Use the FlowPilot MCP tools. For workflow changes on an existing board, first call get_current_flowscript, then call get_declarations as needed, then call edit_flowscript with the full edited FlowScript document. Preserve all kept `//@n:<id>` anchors, and leave allow_deletions false unless the user explicitly asked to delete existing board items. Create workflow functions as FlowScript `function ... {{ ... }}` declarations, not manual command JSON. If the user asks for explanation or no edit is needed, answer in normal text.

USER REQUEST
{user_prompt}"#
    )
}

async fn run_external_agent_invocation(
    invocation: ExternalAgentInvocation,
    channel: Channel<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || run_external_agent_invocation_blocking(invocation, channel))
        .await
        .map_err(|e| format!("External agent task failed: {e}"))?
}

fn run_external_agent_invocation_blocking(
    invocation: ExternalAgentInvocation,
    channel: Channel<String>,
) -> Result<String, String> {
    let mut command = Command::new(&invocation.executable);
    command
        .args(&invocation.args)
        .env("PATH", augmented_path_with_dirs(&invocation.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        format!(
            "Failed to start {} CLI at {}: {e}",
            invocation.backend.label(),
            invocation.executable.display()
        )
    })?;

    if !invocation.prompt.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(invocation.prompt.as_bytes())
                .and_then(|_| stdin.flush())
                .map_err(|e| {
                    format!(
                        "Failed to send prompt to {}: {e}",
                        invocation.backend.label()
                    )
                })?;
        }
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{} did not expose stdout", invocation.backend.label()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{} did not expose stderr", invocation.backend.label()))?;

    let stderr_handle = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
        lines
    });

    let mut final_text = String::new();
    let mut streamed_text = String::new();
    let mut stream_state = ExternalAgentStreamState::default();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(error) = external_agent_error_text(&value) {
                return Err(format!("{} failed: {error}", invocation.backend.label()));
            }

            if let Some(workspace_event) = external_agent_flowscript_workspace_event(&value) {
                let _ = channel.send(workspace_event);
            }

            if let Some(event) = external_agent_process_event(&value) {
                let _ = channel.send(event);
            } else if let Some(label) = external_agent_progress_label(&value) {
                send_external_progress_event(&channel, "external-agent", &label);
            }

            if let Some(delta) =
                external_agent_stream_delta(invocation.backend, &value, &mut stream_state)
                && !delta.is_empty()
            {
                streamed_text.push_str(&delta);
                let _ = channel.send(delta);
            }
            if let Some(result) = external_agent_result_text(invocation.backend, &value) {
                final_text = result;
            }
        } else {
            send_external_progress_event(&channel, "external-agent", &line);
        }
    }

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for {}: {e}", invocation.backend.label()))?;
    let stderr_lines = stderr_handle.join().unwrap_or_default();

    if let Some(path) = &invocation.final_output_path {
        if invocation.backend == FlowPilotAgentBackendKind::Codex
            && let Ok(text) = std::fs::read_to_string(path)
            && !text.trim().is_empty()
        {
            final_text = text;
        }
        let _ = std::fs::remove_file(path);
    }

    if !status.success() {
        let stderr_text = stderr_lines.join("\n");
        return Err(format!(
            "{} exited with status {}{}",
            invocation.backend.label(),
            status,
            if stderr_text.is_empty() {
                String::new()
            } else {
                format!(":\n{stderr_text}")
            }
        ));
    }

    if final_text.trim().is_empty() {
        final_text = streamed_text;
    }

    Ok(final_text.trim().to_string())
}

#[derive(Default)]
struct ExternalAgentStreamState {
    agent_message_text_by_id: HashMap<String, String>,
    last_agent_message_id: Option<String>,
    has_streamed_assistant_text: bool,
}

impl ExternalAgentStreamState {
    fn decorate_agent_delta(&mut self, item_id: &str, delta: &str) -> String {
        if delta.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        if self.has_streamed_assistant_text
            && self.last_agent_message_id.as_deref() != Some(item_id)
            && !delta.starts_with('\n')
        {
            out.push_str("\n\n");
        }
        self.last_agent_message_id = Some(item_id.to_string());
        self.has_streamed_assistant_text = true;
        out.push_str(delta);
        out
    }
}

fn external_agent_process_event(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if !matches!(event_type, "item.started" | "item.completed") {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tool");
    let server_name = item
        .get("server")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("flowpilot");
    let tool_call_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("external-{server_name}-{tool_name}"));

    if event_type == "item.started" {
        return Some(flowpilot_stream_tag(
            "tool_start",
            &serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool": tool_name,
                "summary": format!("{server_name}/{tool_name}"),
            }),
        ));
    }

    let error = item
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some(flowpilot_stream_tag(
        "tool_end",
        &serde_json::json!({
            "tool_call_id": tool_call_id,
            "tool": tool_name,
            "status": if error.is_some() { "error" } else { "done" },
            "result_summary": error.unwrap_or_else(|| "completed".to_string()),
        }),
    ))
}

fn external_agent_flowscript_workspace_event(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if item_type != "mcp_tool_call" {
        return None;
    }

    let tool_name = item
        .get("tool")
        .or_else(|| item.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !is_edit_flowscript_tool_name(tool_name) {
        return None;
    }

    let arguments = external_agent_tool_arguments(item)?;
    let flowscript = extract_flowscript_source_from_tool_arguments(&arguments)?;
    if flowscript.trim().is_empty() {
        return None;
    }

    Some(flowpilot_stream_tag(
        "flowscript_workspace",
        &serde_json::json!({
            "source": flowscript,
            "status": "submitted",
        }),
    ))
}

fn is_edit_flowscript_tool_name(tool_name: &str) -> bool {
    tool_name == "edit_flowscript" || tool_name.ends_with("__edit_flowscript")
}

fn external_agent_tool_arguments(item: &serde_json::Value) -> Option<serde_json::Value> {
    for key in ["arguments", "args", "input", "params", "parameters"] {
        if let Some(value) = item.get(key)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    for pointer in [
        "/call/arguments",
        "/function/arguments",
        "/request/arguments",
        "/tool_call/arguments",
    ] {
        if let Some(value) = item.pointer(pointer)
            && let Some(arguments) = normalize_external_tool_arguments(value)
        {
            return Some(arguments);
        }
    }

    None
}

fn normalize_external_tool_arguments(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .or_else(|| Some(serde_json::Value::String(text.clone())))
        }
        _ => Some(value.clone()),
    }
}

fn extract_flowscript_source_from_tool_arguments(value: &serde_json::Value) -> Option<String> {
    extract_flowscript_source_from_tool_arguments_inner(value, 0)
}

fn extract_flowscript_source_from_tool_arguments_inner(
    value: &serde_json::Value,
    depth: u8,
) -> Option<String> {
    if depth > 4 {
        return None;
    }

    match value {
        serde_json::Value::Object(map) => {
            for key in ["flowscript", "script", "source", "content"] {
                if let Some(source) = map.get(key).and_then(serde_json::Value::as_str)
                    && !source.trim().is_empty()
                {
                    return Some(source.to_string());
                }
            }

            for key in ["arguments", "args", "input", "params", "parameters"] {
                if let Some(nested) = map.get(key)
                    && let Some(source) =
                        extract_flowscript_source_from_tool_arguments_inner(nested, depth + 1)
                {
                    return Some(source);
                }
            }

            None
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                return extract_flowscript_source_from_tool_arguments_inner(&parsed, depth + 1);
            }
            Some(text.clone())
        }
        _ => None,
    }
}

fn external_agent_stream_delta(
    backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    match backend {
        FlowPilotAgentBackendKind::Codex => codex_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::ClaudeCode => generic_agent_message_delta(value, state),
        FlowPilotAgentBackendKind::GithubCopilot => None,
    }
}

fn codex_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if matches!(
        event_type,
        "agent_message_delta" | "assistant_message_delta"
    ) {
        let item_id = value
            .get("item_id")
            .or_else(|| value.get("itemId"))
            .or_else(|| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("codex-agent-message");
        let delta = value
            .get("delta")
            .or_else(|| value.pointer("/item/delta"))
            .or_else(|| value.get("text"))
            .and_then(serde_json::Value::as_str)?;
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    if !matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) {
        return None;
    }

    let item = value.get("item")?;
    let item_type = item
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(item_type, "agent_message" | "assistant_message") {
        return None;
    }

    let item_id = item
        .get("id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("codex-agent-message");

    if let Some(delta) = item.get("delta").and_then(serde_json::Value::as_str) {
        {
            let previous = state
                .agent_message_text_by_id
                .entry(item_id.to_string())
                .or_default();
            previous.push_str(delta);
        }
        return Some(state.decorate_agent_delta(item_id, delta));
    }

    let full_text = item.get("text").and_then(serde_json::Value::as_str)?;
    let delta = {
        let previous = state
            .agent_message_text_by_id
            .entry(item_id.to_string())
            .or_default();
        let delta = if full_text.starts_with(previous.as_str()) {
            full_text[previous.len()..].to_string()
        } else if previous.is_empty() {
            full_text.to_string()
        } else {
            String::new()
        };
        *previous = full_text.to_string();
        delta
    };

    Some(state.decorate_agent_delta(item_id, &delta))
}

fn generic_agent_message_delta(
    value: &serde_json::Value,
    state: &mut ExternalAgentStreamState,
) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !event_type.contains("delta") && !event_type.contains("message") {
        return None;
    }

    let item_id = value
        .get("item_id")
        .or_else(|| value.get("itemId"))
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("external-agent-message");
    let delta = value
        .get("delta")
        .or_else(|| value.pointer("/message/delta"))
        .or_else(|| value.pointer("/item/delta"))
        .and_then(serde_json::Value::as_str)?;

    Some(state.decorate_agent_delta(item_id, delta))
}

fn send_external_progress_event(channel: &Channel<String>, event_id: &str, message: &str) {
    let event = flowpilot_stream_tag(
        "tool_progress",
        &serde_json::json!({
            "tool_call_id": event_id,
            "message": message,
        }),
    );
    let _ = channel.send(event);
}

fn flowpilot_stream_tag(tag: &str, value: &serde_json::Value) -> String {
    format!(
        "<{tag}>{}</{tag}>",
        serde_json::to_string(value).unwrap_or_default()
    )
}

fn external_agent_progress_label(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)?;

    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let status = item
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        match item_type {
            "mcp_tool_call" => {
                let tool = item
                    .get("tool")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Completed {tool}"));
                }
                return Some(format!("Using {tool}..."));
            }
            "command_execution" => {
                let command = item
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("command");
                if event_type == "item.completed" || status == "completed" {
                    return Some(format!("Command completed: {command}"));
                }
                return Some(format!("Running command: {command}"));
            }
            "file_change" => {
                if event_type == "item.completed" || status == "completed" {
                    return Some("File changes completed".to_string());
                }
                return Some("Applying file changes...".to_string());
            }
            "web_search" => {
                let query = item
                    .get("query")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("web");
                return Some(format!("Searching {query}..."));
            }
            "error" => {
                if let Some(message) = item.get("message").and_then(serde_json::Value::as_str) {
                    return Some(format!("Error: {message}"));
                }
            }
            _ => {}
        }
    }

    if event_type.contains("tool") {
        let name = value
            .get("name")
            .or_else(|| value.pointer("/tool/name"))
            .or_else(|| value.pointer("/item/name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool");
        return Some(format!("Using {name}..."));
    }

    if event_type.contains("error") {
        return Some(format!("{}...", event_type.replace('_', " ")));
    }

    None
}

fn external_agent_error_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type == "turn.failed" {
        return value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    if event_type == "error" {
        return value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }

    if matches!(
        event_type,
        "item.started" | "item.updated" | "item.completed"
    ) && let Some(item) = value.get("item")
    {
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if item_type == "error" {
            return item
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }

        if item_type == "mcp_tool_call" {
            return item
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }

    None
}

fn external_agent_result_text(
    backend: FlowPilotAgentBackendKind,
    value: &serde_json::Value,
) -> Option<String> {
    if backend == FlowPilotAgentBackendKind::Codex {
        return codex_agent_result_text(value);
    }

    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if let Some(text) = codex_agent_result_text(value) {
        return Some(text);
    }

    if !event_type.contains("result") && !event_type.contains("final") {
        return None;
    }

    let text = extract_external_agent_text(value);
    (!text.trim().is_empty()).then_some(text)
}

fn codex_agent_result_text(value: &serde_json::Value) -> Option<String> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if event_type == "item.completed" {
        let item = value.get("item")?;
        let item_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if matches!(item_type, "agent_message" | "assistant_message") {
            return item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string);
        }
    }

    None
}

fn extract_external_agent_text(value: &serde_json::Value) -> String {
    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "text" | "content" | "delta" | "message" | "result" | "summary"
                    ) {
                        match child {
                            serde_json::Value::String(text) => {
                                if !looks_like_machine_status(text) {
                                    out.push(text.clone());
                                }
                            }
                            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                collect(child, out);
                            }
                            _ => {}
                        }
                    } else if matches!(
                        child,
                        serde_json::Value::Array(_) | serde_json::Value::Object(_)
                    ) {
                        collect(child, out);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut parts = Vec::new();
    collect(value, &mut parts);
    parts.join("")
}

fn looks_like_machine_status(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed == "started"
        || trimmed == "completed"
}

fn send_commands_event(channel: &Channel<String>, commands: &[BoardCommand]) {
    if commands.is_empty() {
        return;
    }

    let cmd_event = format!(
        "<commands>{}</commands>",
        serde_json::to_string(commands).unwrap_or_default()
    );
    let _ = channel.send(cmd_event);
}

fn workflow_edit_continuation_prompt(
    original_user_prompt: &str,
    latest_workspace: Option<&str>,
    attempt: u8,
) -> String {
    let workspace_note = if latest_workspace.is_some() {
        "You already submitted a FlowScript draft, but it did not queue board commands. Use the validation/tool result context, fix the FlowScript, and call edit_flowscript again."
    } else {
        "You did not produce a FlowScript workspace or board commands yet."
    };

    format!(
        r#"INTERNAL FLOWPILOT CONTINUATION #{attempt}
{workspace_note}

Do not ask the user to confirm. Do not say "Create draft", "go ahead", "tell me if", or similar.
Use placeholders for unknown credentials/data. Your next assistant turn must call tools, and for workflow behavior it must end by calling edit_flowscript with the full FlowScript document. The turn is not complete until board commands are queued or FlowScript validation diagnostics are visible in the workspace.

Original user request:
{original_user_prompt}"#
    )
}

// =============================================================================
// GitHub Copilot SDK Direct Integration
// =============================================================================

use copilot_sdk::{AttachmentType, Client, LogLevel, MessageOptions, UserMessageAttachment};
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

/// Global Copilot client instance (singleton) - uses tokio::sync::Mutex for async safety
static COPILOT_CLIENT: Lazy<Mutex<Option<Client>>> = Lazy::new(|| Mutex::new(None));
static EXTERNAL_AGENT_BACKENDS: Lazy<Mutex<std::collections::HashSet<FlowPilotAgentBackendKind>>> =
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

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
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotBackendStatus {
    pub backend: FlowPilotAgentBackendKind,
    pub label: String,
    pub available: bool,
    pub running: bool,
    pub executable: Option<String>,
    pub message: Option<String>,
    pub transport: FlowPilotAgentTransportKind,
    pub capabilities: FlowPilotAgentCapabilitySet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentTransportKind {
    DirectSdkTools,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotAgentCapabilitySet {
    pub prompt_source: String,
    pub tool_protocol: FlowPilotAgentTransportKind,
    pub tool_names: Vec<String>,
}

impl FlowPilotAgentCapabilitySet {
    fn shared_for(scope: CopilotScope, has_board: bool, has_graph_context: bool) -> Self {
        let mut tool_names: Vec<&'static str> = Vec::new();

        if matches!(scope, CopilotScope::Board | CopilotScope::Both) {
            tool_names.extend(["catalog_search", "validate_commands", "emit_commands"]);
            tool_names.push("get_declarations");
            if has_board {
                tool_names.push("edit_flowscript");
            }
            if has_graph_context {
                tool_names.extend([
                    "get_node_details",
                    "get_unconfigured_nodes",
                    "list_board_nodes",
                ]);
            }
        }

        if matches!(scope, CopilotScope::Frontend | CopilotScope::Both) {
            tool_names.extend(["validate_ui", "emit_ui", "get_component_schema"]);
        }

        tool_names.extend([
            "internet_search",
            "database_tool",
            "storage_tool",
            "execute_event",
            "ask_user",
        ]);

        tool_names.sort_unstable();
        tool_names.dedup();

        Self {
            prompt_source: "flow_like::copilot::prompts".to_string(),
            tool_protocol: FlowPilotAgentTransportKind::DirectSdkTools,
            tool_names: tool_names.into_iter().map(str::to_string).collect(),
        }
    }

    fn for_status(transport: FlowPilotAgentTransportKind) -> Self {
        let mut capabilities = Self::shared_for(CopilotScope::Both, true, true);
        capabilities.tool_protocol = transport;
        capabilities
    }
}

struct FlowPilotAgentSurface {
    graph_context: Option<Arc<GraphContext>>,
    board_arc: Option<Arc<Board>>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Arc<StdMutex<Vec<BoardCommand>>>,
    system_content: String,
    workflow_edit_request: bool,
    capabilities: FlowPilotAgentCapabilitySet,
}

fn build_flowpilot_agent_surface(
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    history: &[UnifiedChatMessage],
    original_user_prompt: &str,
) -> FlowPilotAgentSurface {
    use flow_like::flow::copilot::prepare_context;

    let graph_context = match scope {
        CopilotScope::Board | CopilotScope::Both => board
            .and_then(|board| prepare_context(board, selected_node_ids).ok())
            .map(Arc::new),
        CopilotScope::Frontend => None,
    };

    let board_arc: Option<Arc<Board>> = match scope {
        CopilotScope::Board | CopilotScope::Both => board.map(|b| Arc::new(b.clone())),
        CopilotScope::Frontend => None,
    };

    let desktop_catalog_provider = match scope {
        CopilotScope::Board | CopilotScope::Both => {
            Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes)))
        }
        CopilotScope::Frontend => None,
    };

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Board | CopilotScope::Both => desktop_catalog_provider
            .as_ref()
            .map(|provider| provider.clone() as Arc<dyn CatalogProvider>),
        CopilotScope::Frontend => None,
    };

    let board_flowscript = board_arc.as_ref().map(|board| {
        flow_like::flow::ast::board_to_flowscript(
            board,
            &flow_like::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    });

    let catalog_node_count = desktop_catalog_provider
        .as_ref()
        .map(|provider| provider.len())
        .unwrap_or_else(|| flow_like_catalog::get_catalog().len());

    let workflow_edit_request = matches!(scope, CopilotScope::Board | CopilotScope::Both)
        && board_arc.is_some()
        && is_workflow_edit_request(original_user_prompt);

    let mut system_content = match scope {
        CopilotScope::Board => match board_flowscript.as_deref() {
            Some(flowscript) => flow_like::copilot::prompts::board_sdk_flowscript_system_prompt(
                flowscript,
                catalog_node_count,
            ),
            None => flow_like::copilot::prompts::board_sdk_system_prompt(),
        },
        CopilotScope::Frontend => flow_like::copilot::prompts::frontend_sdk_system_prompt(),
        CopilotScope::Both => {
            let mut prompt = flow_like::copilot::prompts::general_system_prompt();
            if let Some(flowscript) = board_flowscript.as_deref() {
                prompt.push_str("\n\n");
                prompt.push_str(&flow_like::copilot::prompts::flowscript_board_context(
                    flowscript,
                    catalog_node_count,
                ));
            }
            prompt
        }
    };

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

    let mut context_parts = vec![];
    for msg in history {
        let role = match msg.role {
            flow_like::flow::copilot::ChatRole::User => "User",
            flow_like::flow::copilot::ChatRole::Assistant => "Assistant",
        };
        context_parts.push(format!("{}: {}", role, msg.content));
    }
    if !context_parts.is_empty() {
        system_content.push_str(&format!(
            "\n\nConversation history:\n{}",
            context_parts.join("\n\n")
        ));
    }

    let capabilities = FlowPilotAgentCapabilitySet::shared_for(
        scope,
        board_arc.is_some(),
        graph_context.is_some(),
    );

    FlowPilotAgentSurface {
        graph_context,
        board_arc,
        catalog_provider,
        side_effect_commands: Arc::new(StdMutex::new(Vec::new())),
        system_content,
        workflow_edit_request,
        capabilities,
    }
}

#[derive(Debug, Clone)]
struct FlowPilotBackendStartOptions {
    use_stdio: bool,
    cli_url: Option<String>,
    app_handle: Option<AppHandle>,
}

#[async_trait]
trait FlowPilotAgentBackend: Send + Sync {
    fn kind(&self) -> FlowPilotAgentBackendKind;
    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn is_running(&self) -> Result<bool, String>;
    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String>;
    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String>;
    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let kind = self.kind();
        let executable = find_cli_resolution(kind, app_handle)
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);

        FlowPilotBackendStatus {
            backend: kind,
            label: kind.label().to_string(),
            available,
            running,
            executable,
            message: None,
            transport: FlowPilotAgentTransportKind::DirectSdkTools,
            capabilities: FlowPilotAgentCapabilitySet::for_status(
                FlowPilotAgentTransportKind::DirectSdkTools,
            ),
        }
    }
}

struct GithubCopilotBackend;

struct ExternalCodeAgentBackend {
    kind: FlowPilotAgentBackendKind,
}

fn agent_backend(kind: FlowPilotAgentBackendKind) -> Box<dyn FlowPilotAgentBackend> {
    match kind {
        FlowPilotAgentBackendKind::GithubCopilot => Box::new(GithubCopilotBackend),
        FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
            Box::new(ExternalCodeAgentBackend { kind })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliResolutionSource {
    EnvOverride,
    BundledResource,
    CodexStandalone,
    CodexNpmPackage,
    Path,
    IdeExtensionFallback,
}

#[derive(Debug, Clone)]
struct CliResolution {
    executable: PathBuf,
    path_dirs: Vec<PathBuf>,
    source: CliResolutionSource,
}

impl CliResolution {
    fn new(executable: PathBuf, source: CliResolutionSource) -> Self {
        Self {
            executable,
            path_dirs: Vec::new(),
            source,
        }
    }

    fn with_path_dirs(
        executable: PathBuf,
        source: CliResolutionSource,
        path_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            executable,
            path_dirs,
            source,
        }
    }
}

fn codex_binary_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

fn codex_target() -> Option<(&'static str, &'static str)> {
    let target = if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-musl"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-musl"
        } else {
            return None;
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-apple-darwin"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            return None;
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc"
        } else {
            return None;
        }
    } else {
        return None;
    };

    let package = match target {
        "x86_64-unknown-linux-musl" => "@openai/codex-linux-x64",
        "aarch64-unknown-linux-musl" => "@openai/codex-linux-arm64",
        "x86_64-apple-darwin" => "@openai/codex-darwin-x64",
        "aarch64-apple-darwin" => "@openai/codex-darwin-arm64",
        "x86_64-pc-windows-msvc" => "@openai/codex-win32-x64",
        "aarch64-pc-windows-msvc" => "@openai/codex-win32-arm64",
        _ => return None,
    };

    Some((target, package))
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
    dirs.extend(codex_standalone_visible_dirs(&home));

    dirs.sort();
    dirs.dedup();
    dirs
}

fn codex_standalone_visible_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(install_dir) = std::env::var("CODEX_INSTALL_DIR") {
        let trimmed = install_dir.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed));
        }
    }

    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    dirs.push(codex_home.join("packages/standalone/current/bin"));
    dirs.push(codex_home.join("packages/standalone/current"));

    #[cfg(not(windows))]
    dirs.push(home.join(".local/bin"));

    #[cfg(windows)]
    if let Some(local_app_data) = dirs_next::data_local_dir() {
        dirs.push(local_app_data.join("Programs/OpenAI/Codex/bin"));
    }

    dirs
}

fn codex_ide_extension_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in [
        home.join(".vscode/extensions"),
        home.join(".vscode-insiders/extensions"),
        home.join(".cursor/extensions"),
        home.join(".windsurf/extensions"),
    ] {
        collect_codex_cli_dirs(&root, 5, &mut dirs);
    }

    dirs.sort_by(|a, b| b.cmp(a));
    dirs.dedup();
    dirs
}

fn collect_codex_cli_dirs(root: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }

    let codex_executable = root.join(codex_binary_name());
    if is_executable_file(&codex_executable) {
        out.push(root.to_path_buf());
        let helper_path = root.join("codex-path");
        if helper_path.is_dir() {
            out.push(helper_path);
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_cli_dirs(&path, depth - 1, out);
        }
    }
}

/// Resolve the Copilot CLI path, searching beyond the (possibly limited) bundled-app PATH.
///
/// On macOS/Linux, apps launched from Finder/Dock inherit a minimal PATH that
/// excludes npm-global, nvm, volta, mise, and Homebrew directories. This
/// function probes those common locations so that prod builds can find the CLI.
fn find_cli_path(kind: FlowPilotAgentBackendKind) -> Option<std::path::PathBuf> {
    find_cli_resolution(kind, None).map(|resolution| resolution.executable)
}

fn find_cli_resolution(
    kind: FlowPilotAgentBackendKind,
    app_handle: Option<&AppHandle>,
) -> Option<CliResolution> {
    if let Ok(p) = std::env::var(kind.env_path_var()) {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return None;
        }

        let path = PathBuf::from(trimmed);
        if is_executable_file(&path) {
            return Some(CliResolution::new(path, CliResolutionSource::EnvOverride));
        }

        if path.is_dir() {
            let candidate = path.join(kind.cli_name());
            if is_executable_file(&candidate) {
                return Some(CliResolution::new(
                    candidate,
                    CliResolutionSource::EnvOverride,
                ));
            }
        }

        if path.components().count() == 1
            && let Some(found) = find_executable_in_path(trimmed, &augmented_path())
        {
            return Some(CliResolution::new(found, CliResolutionSource::EnvOverride));
        }
    }

    if kind == FlowPilotAgentBackendKind::Codex {
        if let Some(resolution) = find_bundled_codex_cli(app_handle) {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_standalone_cli() {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_npm_package_cli(app_handle) {
            return Some(resolution);
        }
    }

    if let Some(found) = find_executable_in_path(kind.cli_name(), &augmented_path()) {
        return Some(CliResolution::new(found, CliResolutionSource::Path));
    }

    if kind == FlowPilotAgentBackendKind::Codex
        && let Some(home) = dirs_next::home_dir()
    {
        for dir in codex_ide_extension_candidate_dirs(&home) {
            if let Some(candidate) = find_codex_executable_in_dir(&dir) {
                let mut path_dirs = Vec::new();
                let helper_path = dir.join("codex-path");
                if helper_path.is_dir() {
                    path_dirs.push(helper_path);
                }
                return Some(CliResolution::with_path_dirs(
                    candidate,
                    CliResolutionSource::IdeExtensionFallback,
                    path_dirs,
                ));
            }
        }
    }

    None
}

fn find_bundled_codex_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.clone(),
            resource_dir.join("codex"),
            resource_dir.join("binaries"),
            resource_dir.join("bin"),
            resource_dir.join("node_modules"),
        ]);
    }

    for root in roots {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::BundledResource)
        {
            return Some(resolution);
        }
        if let Some(candidate) = find_codex_executable_in_dir(&root) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::BundledResource,
            ));
        }
    }

    None
}

fn find_codex_standalone_cli() -> Option<CliResolution> {
    let home = dirs_next::home_dir()?;
    for dir in codex_standalone_visible_dirs(&home) {
        if let Some(candidate) = find_codex_executable_in_dir(&dir) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::CodexStandalone,
            ));
        }
    }
    None
}

fn find_codex_npm_package_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    for root in codex_npm_search_roots(app_handle) {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::CodexNpmPackage)
        {
            return Some(resolution);
        }
    }
    None
}

fn codex_npm_search_roots(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.join("node_modules"),
            resource_dir.join("codex/node_modules"),
        ]);
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let mut dir = Some(current_dir.as_path());
        while let Some(path) = dir {
            roots.push(path.join("node_modules"));
            dir = path.parent();
        }
    }

    if let Some(home) = dirs_next::home_dir() {
        roots.extend([
            home.join(".npm-global/lib/node_modules"),
            home.join(".npm-packages/lib/node_modules"),
            home.join(".bun/install/global/node_modules"),
        ]);

        let nvm_dir = std::env::var("NVM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".nvm"));
        if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("lib/node_modules"));
            }
        }

        if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("installation/lib/node_modules"));
            }
        }
    }

    #[cfg(windows)]
    if let Some(data_dir) = dirs_next::data_dir() {
        roots.push(data_dir.join("npm/node_modules"));
    }

    roots.sort();
    roots.dedup();
    roots
}

fn find_codex_packaged_cli_under_root(
    root: &Path,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let (target, platform_package) = codex_target()?;
    let package_leaf = platform_package
        .rsplit('/')
        .next()
        .unwrap_or(platform_package);
    let package_roots = [
        root.join(platform_package),
        root.join("@openai").join(package_leaf),
        root.join("@openai/codex/node_modules")
            .join(platform_package),
        root.join("@openai/codex/node_modules/@openai")
            .join(package_leaf),
        root.join("@openai/codex"),
        root.to_path_buf(),
    ];

    for package_root in package_roots {
        if let Some(resolution) = resolve_codex_native_package(&package_root, target, source) {
            return Some(resolution);
        }
    }

    None
}

fn resolve_codex_native_package(
    package_root: &Path,
    target: &str,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let target_root = package_root.join("vendor").join(target);
    let package_binary = target_root.join("bin").join(codex_binary_name());
    if is_executable_file(&package_binary) && target_root.join("codex-package.json").is_file() {
        let path_dirs = [target_root.join("codex-path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            package_binary,
            source,
            path_dirs,
        ));
    }

    let legacy_binary = target_root.join("codex").join(codex_binary_name());
    if is_executable_file(&legacy_binary) {
        let path_dirs = [target_root.join("path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            legacy_binary,
            source,
            path_dirs,
        ));
    }

    None
}

fn find_codex_executable_in_dir(dir: &Path) -> Option<PathBuf> {
    for file_name in codex_executable_file_names() {
        let candidate = dir.join(file_name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn codex_executable_file_names() -> Vec<String> {
    let mut names = vec![codex_binary_name().to_string()];
    if let Some((target, _)) = codex_target() {
        names.push(if cfg!(windows) {
            format!("codex-{target}.exe")
        } else {
            format!("codex-{target}")
        });
    }
    names
}

fn find_copilot_cli_path() -> Option<std::path::PathBuf> {
    find_cli_path(FlowPilotAgentBackendKind::GithubCopilot)
}

/// Build an augmented PATH that prepends the extra bin directories to the
/// current PATH so that the spawned copilot CLI process (a Node.js script)
/// can locate `node` and other tools even in production builds.
fn augmented_path() -> String {
    augmented_path_with_dirs(&[])
}

fn augmented_path_with_dirs(prefix_dirs: &[PathBuf]) -> String {
    let mut entries: Vec<PathBuf> = prefix_dirs
        .iter()
        .cloned()
        .chain(extra_bin_dirs())
        .filter(|d| d.exists())
        .collect();

    let current = std::env::var("PATH").unwrap_or_default();
    entries.extend(std::env::split_paths(&current));

    std::env::join_paths(entries)
        .unwrap_or_else(|_| current.into())
        .to_string_lossy()
        .into_owned()
}

fn find_executable_in_path(name: &str, path_value: &str) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path_value) {
        for file_name in executable_file_names(name) {
            let candidate = dir.join(file_name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_file_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let path = std::path::Path::new(name);
        if path.extension().is_some() {
            return vec![name.to_string()];
        }

        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![name.to_string()];
        for ext in pathext.split(';').filter(|ext| !ext.trim().is_empty()) {
            names.push(format!("{name}{}", ext.to_ascii_lowercase()));
            names.push(format!("{name}{}", ext.to_ascii_uppercase()));
        }
        names.sort();
        names.dedup();
        names
    }

    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

async fn probe_external_agent_cli(
    kind: FlowPilotAgentBackendKind,
    executable: &std::path::Path,
    path_dirs: &[PathBuf],
) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(executable)
            .arg("--version")
            .env("PATH", augmented_path_with_dirs(path_dirs))
            .output(),
    )
    .await
    .map_err(|_| format!("{} CLI probe timed out after 5s", kind.label()))?
    .map_err(|e| {
        format!(
            "Failed to run {} CLI at {}: {e}",
            kind.label(),
            executable.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!(
            "{} --version exited with status {}{}",
            kind.label(),
            output.status,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        Ok(format!("{} CLI responded to --version", kind.label()))
    } else {
        Ok(stdout)
    }
}

#[async_trait]
impl FlowPilotAgentBackend for GithubCopilotBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        FlowPilotAgentBackendKind::GithubCopilot
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let mut builder = Client::builder()
            .use_stdio(options.use_stdio)
            .log_level(LogLevel::Error);

        if let Some(url) = options.cli_url {
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

    async fn stop(&self) -> Result<(), String> {
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

    async fn is_running(&self) -> Result<bool, String> {
        let guard = COPILOT_CLIENT.lock().await;
        Ok(guard.is_some())
    }

    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String> {
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

    async fn get_auth_status(
        &self,
        _app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let guard = COPILOT_CLIENT.lock().await;
        let client = guard.as_ref().ok_or("Copilot client not started")?;
        let status = client
            .get_auth_status()
            .await
            .map_err(|e| format!("Failed to get auth status: {}", e))?;

        Ok(CopilotAuthStatus {
            authenticated: status.is_authenticated,
            login: status.login.clone(),
            message: None,
        })
    }
}

#[async_trait]
impl FlowPilotAgentBackend for ExternalCodeAgentBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        self.kind
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let cli = find_cli_resolution(self.kind, options.app_handle.as_ref()).ok_or_else(|| {
            format!(
                "{} CLI was not found. Install it or set {} to its executable path.",
                self.kind.label(),
                self.kind.env_path_var()
            )
        })?;
        let version = probe_external_agent_cli(self.kind, &cli.executable, &cli.path_dirs).await?;
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.insert(self.kind);
        tracing::info!(
            backend = self.kind.label(),
            executable = %cli.executable.display(),
            source = ?cli.source,
            version = %version,
            "enabled external FlowPilot backend"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.remove(&self.kind);
        Ok(())
    }

    async fn is_running(&self) -> Result<bool, String> {
        let guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        Ok(guard.contains(&self.kind))
    }

    async fn list_models(&self) -> Result<Vec<CopilotModelInfo>, String> {
        let mut models = Vec::new();
        match self.kind {
            FlowPilotAgentBackendKind::Codex => {
                // Codex model availability depends on whether the user is
                // authenticated with a ChatGPT account, API key, enterprise
                // policy, and the installed Codex runtime version. Hard-coded
                // Codex model ids regularly become invalid for ChatGPT-account
                // sessions, so default to the runtime/configured model unless a
                // future dynamic model source can prove a model is supported.
                models.push(CopilotModelInfo {
                    id: "default".to_string(),
                    name: "Codex configured default".to_string(),
                });
            }
            FlowPilotAgentBackendKind::ClaudeCode => {
                models.extend([
                    CopilotModelInfo {
                        id: "sonnet".to_string(),
                        name: "Claude Sonnet".to_string(),
                    },
                    CopilotModelInfo {
                        id: "opus".to_string(),
                        name: "Claude Opus".to_string(),
                    },
                    CopilotModelInfo {
                        id: "default".to_string(),
                        name: "Claude Code configured default".to_string(),
                    },
                ]);
            }
            FlowPilotAgentBackendKind::GithubCopilot => {
                models.push(CopilotModelInfo {
                    id: "default".to_string(),
                    name: "GitHub Copilot configured default".to_string(),
                });
            }
        }
        Ok(models)
    }

    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let resolution = find_cli_resolution(self.kind, app_handle);
        let executable = resolution
            .as_ref()
            .map(|resolution| resolution.executable.display().to_string());
        Ok(CopilotAuthStatus {
            authenticated: executable.is_some(),
            login: None,
            message: Some(match executable {
                Some(path) => format!(
                    "{} CLI found at {path} ({:?}). Authentication is delegated to that CLI.",
                    self.kind.label(),
                    resolution.map(|resolution| resolution.source)
                ),
                None => format!(
                    "{} CLI was not found. Set {} to its executable path.",
                    self.kind.label(),
                    self.kind.env_path_var()
                ),
            }),
        })
    }

    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let resolution = find_cli_resolution(self.kind, app_handle);
        let source = resolution.as_ref().map(|resolution| resolution.source);
        let executable = resolution
            .as_ref()
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);
        FlowPilotBackendStatus {
            backend: self.kind,
            label: self.kind.label().to_string(),
            available,
            running,
            executable,
            message: Some(if available {
                format!(
                    "{} uses FlowPilot's shared prompt/tool surface through a session-local MCP bridge ({source:?}).",
                    self.kind.label(),
                )
            } else {
                format!(
                    "{} CLI was not found. Install it or set {}.",
                    self.kind.label(),
                    self.kind.env_path_var()
                )
            }),
            transport: FlowPilotAgentTransportKind::Mcp,
            capabilities: FlowPilotAgentCapabilitySet::for_status(FlowPilotAgentTransportKind::Mcp),
        }
    }
}

fn parse_agent_backend(backend: String) -> Result<FlowPilotAgentBackendKind, String> {
    FlowPilotAgentBackendKind::parse(&backend)
        .ok_or_else(|| format!("Unsupported FlowPilot backend: {backend}"))
}

#[tauri::command]
pub async fn flowpilot_agent_backend_start(
    app_handle: AppHandle,
    backend: String,
    use_stdio: Option<bool>,
    cli_url: Option<String>,
) -> Result<(), String> {
    let backend = agent_backend(parse_agent_backend(backend)?);
    backend
        .start(FlowPilotBackendStartOptions {
            use_stdio: use_stdio.unwrap_or(true),
            cli_url,
            app_handle: Some(app_handle),
        })
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_stop(backend: String) -> Result<(), String> {
    agent_backend(parse_agent_backend(backend)?).stop().await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_is_running(backend: String) -> Result<bool, String> {
    agent_backend(parse_agent_backend(backend)?)
        .is_running()
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_list_models(
    backend: String,
) -> Result<Vec<CopilotModelInfo>, String> {
    agent_backend(parse_agent_backend(backend)?)
        .list_models()
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_get_auth_status(
    app_handle: AppHandle,
    backend: String,
) -> Result<CopilotAuthStatus, String> {
    agent_backend(parse_agent_backend(backend)?)
        .get_auth_status(Some(&app_handle))
        .await
}

#[tauri::command]
pub async fn flowpilot_agent_backend_status(
    app_handle: AppHandle,
    backend: String,
) -> Result<FlowPilotBackendStatus, String> {
    Ok(agent_backend(parse_agent_backend(backend)?)
        .status(Some(&app_handle))
        .await)
}

#[tauri::command]
pub async fn flowpilot_agent_backend_list(
    app_handle: AppHandle,
) -> Result<Vec<FlowPilotBackendStatus>, String> {
    let mut statuses = Vec::new();
    for backend in [
        FlowPilotAgentBackendKind::GithubCopilot,
        FlowPilotAgentBackendKind::Codex,
        FlowPilotAgentBackendKind::ClaudeCode,
    ] {
        statuses.push(agent_backend(backend).status(Some(&app_handle)).await);
    }
    Ok(statuses)
}

/// Start the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_start(
    app_handle: AppHandle,
    use_stdio: Option<bool>,
    cli_url: Option<String>,
) -> Result<(), String> {
    flowpilot_agent_backend_start(app_handle, "github-copilot".to_string(), use_stdio, cli_url)
        .await
}

/// Stop the GitHub Copilot SDK client
#[tauri::command]
pub async fn copilot_sdk_stop() -> Result<(), String> {
    flowpilot_agent_backend_stop("github-copilot".to_string()).await
}

/// Check if the Copilot SDK client is running
#[tauri::command]
pub async fn copilot_sdk_is_running() -> Result<bool, String> {
    flowpilot_agent_backend_is_running("github-copilot".to_string()).await
}

/// List available GitHub Copilot models
#[tauri::command]
pub async fn copilot_sdk_list_models() -> Result<Vec<CopilotModelInfo>, String> {
    flowpilot_agent_backend_list_models("github-copilot".to_string()).await
}

/// Get GitHub Copilot authentication status
#[tauri::command]
pub async fn copilot_sdk_get_auth_status(
    app_handle: AppHandle,
) -> Result<CopilotAuthStatus, String> {
    flowpilot_agent_backend_get_auth_status(app_handle, "github-copilot".to_string()).await
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
    fn workflow_edit_classifier_allows_read_only_text_answers() {
        for prompt in [
            "explain why this node is not connected to the API Call",
            "what does this FlowScript do?",
            "check if the workflow execution wiring is correct",
            "debug why the For Each loop is not working",
        ] {
            assert!(
                !is_workflow_edit_request(prompt),
                "prompt should stay read-only: {prompt}"
            );
        }
    }

    #[test]
    fn workflow_edit_classifier_still_detects_mutations() {
        for prompt in [
            "generate a workflow that fetches the Rust RSS feed",
            "connect the API Call success output to To Text",
            "fix the workflow execution wiring",
            "update this flow to store rows in the database",
        ] {
            assert!(
                is_workflow_edit_request(prompt),
                "prompt should be treated as workflow edit: {prompt}"
            );
        }
    }

    #[test]
    fn model_selection_routes_agent_backend_prefixes() {
        let github = FlowPilotModelSelection::parse(Some("github-copilot:gpt-5-mini".to_string()));
        assert_eq!(
            github.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
        );
        assert_eq!(github.model_id.as_deref(), Some("gpt-5-mini"));

        let legacy = FlowPilotModelSelection::parse(Some("copilot:claude".to_string()));
        assert_eq!(
            legacy.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
        );
        assert_eq!(legacy.model_id.as_deref(), Some("claude"));

        let codex = FlowPilotModelSelection::parse(Some("codex:default".to_string()));
        assert_eq!(
            codex.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::Codex)
        );

        let claude = FlowPilotModelSelection::parse(Some("claude-code:default".to_string()));
        assert_eq!(
            claude.backend,
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::ClaudeCode)
        );
    }

    #[test]
    fn model_selection_keeps_bits_model_ids_unprefixed() {
        let selection = FlowPilotModelSelection::parse(Some("hub:model".to_string()));
        assert_eq!(selection.backend, FlowPilotChatBackend::Bits);
        assert_eq!(selection.model_id.as_deref(), Some("hub:model"));
    }

    #[test]
    fn shared_agent_capability_set_covers_board_frontend_and_runtime_tools() {
        let capabilities = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::Both, true, true);
        for tool in [
            "get_declarations",
            "edit_flowscript",
            "validate_commands",
            "emit_commands",
            "validate_ui",
            "emit_ui",
            "internet_search",
            "database_tool",
            "storage_tool",
            "execute_event",
            "ask_user",
        ] {
            assert!(
                capabilities.tool_names.iter().any(|name| name == tool),
                "shared FlowPilot capability set must include {tool}; got {:?}",
                capabilities.tool_names
            );
        }
        assert_eq!(
            capabilities.prompt_source, "flow_like::copilot::prompts",
            "all agent backends must use the shared prompt module"
        );
    }

    #[test]
    fn codex_invocation_uses_streamable_http_mcp_server() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::Codex,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/codex"),
                CliResolutionSource::Path,
            ),
            "gpt-5-mini",
            "http://127.0.0.1:12345/mcp",
            "hello".to_string(),
            vec!["edit_flowscript".to_string()],
        )
        .expect("codex invocation should build");

        assert_eq!(invocation.backend, FlowPilotAgentBackendKind::Codex);
        assert!(invocation.args.contains(&"exec".to_string()));
        assert!(invocation.args.contains(&"--experimental-json".to_string()));
        assert!(
            invocation
                .args
                .contains(&"--skip-git-repo-check".to_string())
        );
        assert!(invocation.args.contains(&"--config".to_string()));
        assert!(
            !invocation.args.contains(&"--model".to_string()),
            "Codex should use its runtime/configured default model by default because explicit model ids can be rejected for ChatGPT-account sessions: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--sandbox", "read-only"]),
            "codex invocation should keep FlowPilot workspace edits in MCP tools, not shell writes: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg.contains("mcp_servers.flowpilot.url=")
                    && arg.contains("127.0.0.1:12345/mcp")),
            "codex args should contain MCP URL: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg == "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\""),
            "codex exec must explicitly approve the session-local FlowPilot MCP tools in headless mode: {:?}",
            invocation.args
        );
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg == "approval_policy=\"never\""),
            "codex invocation should run non-interactively through FlowPilot approvals/tools"
        );
        assert!(invocation.prompt.contains("hello"));
    }

    #[test]
    fn claude_invocation_uses_shared_mcp_config() {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::ClaudeCode,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/claude"),
                CliResolutionSource::Path,
            ),
            "sonnet",
            "http://127.0.0.1:23456/mcp",
            "hello".to_string(),
            vec![
                "get_declarations".to_string(),
                "edit_flowscript".to_string(),
            ],
        )
        .expect("claude invocation should build");

        assert_eq!(invocation.backend, FlowPilotAgentBackendKind::ClaudeCode);
        assert!(invocation.args.contains(&"--mcp-config".to_string()));
        assert!(invocation.args.contains(&"stream-json".to_string()));
        assert!(invocation.args.contains(&"--strict-mcp-config".to_string()));
        assert!(invocation.args.contains(&"--allowedTools".to_string()));
        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg.contains("mcp__flowpilot__get_declarations")
                    && arg.contains("mcp__flowpilot__edit_flowscript")),
            "claude invocation should allow only shared FlowPilot MCP tools: {:?}",
            invocation.args
        );
        assert!(invocation.args.contains(&"sonnet".to_string()));

        let config_path = invocation
            .final_output_path
            .as_ref()
            .expect("claude invocation stores temp MCP config");
        let config = std::fs::read_to_string(config_path).expect("temp MCP config is readable");
        assert!(config.contains("flowpilot"));
        assert!(config.contains("127.0.0.1:23456/mcp"));
        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn external_agent_text_extractor_handles_result_events() {
        let event = serde_json::json!({
            "type": "result",
            "message": {
                "content": [
                    { "type": "text", "text": "Created the FlowScript draft." }
                ]
            }
        });

        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
            Some("Created the FlowScript draft.")
        );
    }

    #[test]
    fn codex_event_parser_uses_agent_message_completion() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item-1",
                "type": "agent_message",
                "text": "Created the FlowScript draft."
            }
        });

        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
            Some("Created the FlowScript draft.")
        );
    }

    #[test]
    fn codex_stream_parser_ignores_mcp_tool_output_as_chat_text() {
        let event = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "list_board_nodes",
                "status": "completed",
                "result": {
                    "content": [
                        { "type": "text", "text": "Board has 37 nodes and many variables." }
                    ]
                }
            }
        });

        let mut state = ExternalAgentStreamState::default();
        assert_eq!(codex_agent_message_delta(&event, &mut state), None);

        let process_event =
            external_agent_process_event(&event).expect("mcp tool call should be framed");
        assert!(process_event.starts_with("<tool_end>"));
        assert!(process_event.contains("list_board_nodes"));
        assert_eq!(
            external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event),
            None
        );
    }

    #[test]
    fn codex_stream_parser_emits_flowscript_workspace_from_edit_tool_arguments() {
        let event = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "edit_flowscript",
                "arguments": {
                    "flowscript": "run() {\n    const db = openLocalDb({ name: \"gmail_vectors\" })\n}"
                }
            }
        });

        let workspace_event = external_agent_flowscript_workspace_event(&event)
            .expect("edit_flowscript arguments should create a workspace stream event");
        assert!(workspace_event.starts_with("<flowscript_workspace>"));
        assert!(workspace_event.contains("openLocalDb"));
        assert!(workspace_event.contains("submitted"));
    }

    #[test]
    fn codex_stream_parser_accepts_json_string_edit_tool_arguments() {
        let event = serde_json::json!({
            "type": "item.started",
            "item": {
                "id": "tool-1",
                "type": "mcp_tool_call",
                "server": "flowpilot",
                "tool": "mcp__flowpilot__edit_flowscript",
                "arguments": "{\"flowscript\":\"run() {\\n    logInfo({ message: \\\"hello\\\" })\\n}\"}"
            }
        });

        let workspace_event = external_agent_flowscript_workspace_event(&event)
            .expect("json-string edit_flowscript arguments should be parsed");
        assert!(workspace_event.starts_with("<flowscript_workspace>"));
        assert!(workspace_event.contains("logInfo"));
    }

    #[test]
    fn codex_stream_parser_emits_only_new_agent_message_suffixes() {
        let mut state = ExternalAgentStreamState::default();
        let first = serde_json::json!({
            "type": "item.updated",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello"
            }
        });
        let second = serde_json::json!({
            "type": "item.updated",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello world"
            }
        });
        let completed = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "Hello world"
            }
        });

        assert_eq!(
            codex_agent_message_delta(&first, &mut state).as_deref(),
            Some("Hello")
        );
        assert_eq!(
            codex_agent_message_delta(&second, &mut state).as_deref(),
            Some(" world")
        );
        assert_eq!(
            codex_agent_message_delta(&completed, &mut state).as_deref(),
            Some("")
        );
    }

    #[test]
    fn codex_stream_parser_separates_multiple_agent_messages() {
        let mut state = ExternalAgentStreamState::default();
        let first = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-1",
                "type": "agent_message",
                "text": "First note."
            }
        });
        let second = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "msg-2",
                "type": "agent_message",
                "text": "Second note."
            }
        });

        assert_eq!(
            codex_agent_message_delta(&first, &mut state).as_deref(),
            Some("First note.")
        );
        assert_eq!(
            codex_agent_message_delta(&second, &mut state).as_deref(),
            Some("\n\nSecond note.")
        );
    }

    #[test]
    fn codex_event_parser_surfaces_turn_failures() {
        let event = serde_json::json!({
            "type": "turn.failed",
            "error": {
                "message": "not authenticated"
            }
        });

        assert_eq!(
            external_agent_error_text(&event).as_deref(),
            Some("not authenticated")
        );
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
    fn executable_lookup_searches_supplied_path() -> std::io::Result<()> {
        let temp_dir = std::env::temp_dir().join(format!(
            "flowpilot-executable-lookup-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp_dir)?;
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = temp_dir.join(executable_name);
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let path_value = std::env::join_paths([temp_dir.as_path()])
            .expect("test path should join")
            .to_string_lossy()
            .into_owned();

        assert_eq!(
            find_executable_in_path("codex", &path_value).as_deref(),
            Some(executable.as_path())
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }

    #[test]
    fn codex_ide_extension_candidate_dirs_find_extension_bundled_binary() -> std::io::Result<()> {
        let temp_home = std::env::temp_dir().join(format!(
            "flowpilot-codex-extension-test-{}",
            uuid::Uuid::new_v4()
        ));
        let codex_dir = temp_home.join(".vscode/extensions/openai.chatgpt-test/bin/macos-aarch64");
        std::fs::create_dir_all(codex_dir.join("codex-path"))?;
        let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = codex_dir.join(executable_name);
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let dirs = codex_ide_extension_candidate_dirs(&temp_home);
        assert!(
            dirs.iter().any(|dir| dir == &codex_dir),
            "expected extension binary directory in candidates: {:?}",
            dirs
        );
        assert!(
            dirs.iter().any(|dir| dir == &codex_dir.join("codex-path")),
            "expected bundled Codex PATH helper directory in candidates: {:?}",
            dirs
        );

        let _ = std::fs::remove_dir_all(&temp_home);
        Ok(())
    }

    #[test]
    fn codex_npm_native_package_layout_resolves_like_official_sdk() -> std::io::Result<()> {
        let Some((target, platform_package)) = codex_target() else {
            return Ok(());
        };
        let temp_root = std::env::temp_dir().join(format!(
            "flowpilot-codex-npm-package-test-{}",
            uuid::Uuid::new_v4()
        ));
        let vendor_target = temp_root
            .join("node_modules")
            .join(platform_package)
            .join("vendor")
            .join(target);
        let bin_dir = vendor_target.join("bin");
        std::fs::create_dir_all(&bin_dir)?;
        std::fs::create_dir_all(vendor_target.join("codex-path"))?;
        std::fs::write(vendor_target.join("codex-package.json"), b"{}")?;
        let executable = bin_dir.join(codex_binary_name());
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
        }

        let resolution = find_codex_packaged_cli_under_root(
            &temp_root.join("node_modules"),
            CliResolutionSource::CodexNpmPackage,
        )
        .expect("official @openai/codex native package layout should resolve");
        assert_eq!(resolution.executable, executable);
        assert_eq!(resolution.source, CliResolutionSource::CodexNpmPackage);
        assert!(
            resolution
                .path_dirs
                .iter()
                .any(|dir| dir == &vendor_target.join("codex-path")),
            "expected codex-path helper dir in resolution: {:?}",
            resolution.path_dirs
        );

        let _ = std::fs::remove_dir_all(&temp_root);
        Ok(())
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
