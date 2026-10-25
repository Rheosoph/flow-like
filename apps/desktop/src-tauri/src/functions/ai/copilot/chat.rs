//! Board chat dispatch, profile context, and hosted specialist execution.

use super::agent_surface::{
    pending_flowscript_redelivery_for_request, pending_flowscript_redelivery_response,
};
use super::backend_types::{
    FlowPilotAgentBackendKind, FlowPilotChatBackend, FlowPilotModelSelection,
};
use super::backends::agent_backend;
use super::catalog::{DesktopCatalogProvider, authoritative_app_catalog_nodes};
use super::client_pool::{
    acquire_nested_copilot_run_permit, nested_copilot_run_gate, nested_copilot_run_gate_key,
};
use super::external_chat::external_code_agent_chat_internal;
use super::platform_bridge::{DesktopPlatformBridge, FrontendPlatformToolSet};
use super::runtime::{claim_run_tool_owner, frontend_tool_channel, register_copilot_run};
use super::sdk_chat::copilot_sdk_chat_internal;
use super::stream_events::{
    correlate_stream_frame, request_identity_prompt_for, scoped_parent_request_id,
    send_commands_event, send_correlated_stream_json_event,
};
use super::telemetry::{AGENT_STAGE_RUN, instrumented_agent_stage};
use super::tool_policy::is_read_only_workflow_request;
use super::workflow_reporting::WorkflowRunSummaryEmitter;
use crate::{
    functions::ai::{
        copilot_sdk_tools::{
            retained_flow_ir_draft_store_for_board, schedule_flow_ir_draft_snapshot,
        },
        frontend_tool_bridge::FrontendToolContext,
    },
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    a2ui::SurfaceComponent,
    app::{App, AppVisibility},
    copilot::{
        ChatImage, CopilotScope, UIActionContext, UnifiedChatMessage, UnifiedContext,
        UnifiedCopilot, UnifiedCopilotResponse,
    },
    flow::{
        board::Board,
        copilot::{
            AttachmentManifestEntry, CatalogProvider, GlobalDataStudioContext,
            GlobalOpenBoardContext, PlatformContextInput, PlatformSpecialist, RunContext,
            build_platform_context, platform::PlatformToolBridge, run_specialist_chat_with_access,
        },
        node::Node,
    },
    models::llm::ModelUsageContext,
};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, State, ipc::Channel};

pub(super) fn resolve_copilot_app_id(
    explicit_app_id: Option<&str>,
    run_context_app_id: Option<&str>,
    action_context_app_id: Option<&str>,
) -> Result<Option<String>, String> {
    let mut resolved: Option<&str> = None;

    for candidate in [explicit_app_id, run_context_app_id, action_context_app_id]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    {
        if resolved.is_some_and(|existing| existing != candidate) {
            return Err("Conflicting app IDs in copilot request context".to_string());
        }
        resolved = Some(candidate);
    }

    Ok(resolved.map(str::to_string))
}

/// App scope for the copilot's node catalog.
///
/// A detached nested specialist carries its scope on `tool_context`, but panel runs identify the
/// app only through the request's own `app_id`/run/action context. Resolving just one of those
/// silently downgrades the catalog to builtin nodes, hiding every installed-package node from the
/// model with no error. Unlike [`resolve_copilot_app_id`] this takes the first candidate rather
/// than rejecting disagreement: catalog scope is a visibility concern, and a genuine conflict
/// still fails the request at the usage-attribution boundary.
fn resolve_catalog_app_id(
    tool_context_app_id: Option<&str>,
    explicit_app_id: Option<&str>,
    run_context_app_id: Option<&str>,
    action_context_app_id: Option<&str>,
) -> Option<String> {
    [
        tool_context_app_id,
        explicit_app_id,
        run_context_app_id,
        action_context_app_id,
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|app_id| !app_id.is_empty())
    .map(str::to_string)
}

/// The active profile, with the user's WHOLE custom-model library hydrated
/// instead of only the bits the profile activated. The model pickers offer that
/// library independent of profile membership, so an explicitly selected model
/// must resolve; automatic "best model" selection stays scoped to the profile's
/// `bits` inside `Profile`.
pub(super) async fn copilot_profile(
    app_handle: &AppHandle,
) -> Option<Arc<flow_like::profile::Profile>> {
    let mut profile = TauriSettingsState::current_profile(app_handle).await.ok()?;

    if let Ok(settings) = TauriSettingsState::construct(app_handle).await {
        let settings = settings.lock().await;
        profile.hub_profile.custom_bits = settings
            .custom_bits
            .iter()
            .cloned()
            .map(flow_like::profile::ProfileCustomBit)
            .collect();
    }

    Some(Arc::new(profile.hub_profile))
}

pub(super) fn home_profile_scope_error(
    scope: CopilotScope,
    tool_context: Option<&FrontendToolContext>,
    current_profile_id: Option<&str>,
) -> Option<String> {
    if scope != CopilotScope::Home {
        return None;
    }
    let expected = tool_context.and_then(|context| context.profile_id.as_deref())?;
    (expected.is_empty() || current_profile_id != Some(expected)).then(|| {
        serde_json::json!({
            "status": "stale",
            "code": "home_profile_changed",
            "profile_id": expected,
            "message": "The active profile changed after this Home run started. No Home changes were applied. Start a new Home request for the selected profile."
        })
        .to_string()
    })
}

/// The ids the host already knows, handed to a specialist so it defaults to the app/overlay the
/// user is actually looking at instead of asking for them or guessing.
pub(super) fn specialist_host_context(
    tool_context: Option<&FrontendToolContext>,
    host_context_guidance: Option<&str>,
) -> String {
    let context_id = |value: Option<&String>| {
        value
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    let app_id = context_id(tool_context.and_then(|context| context.app_id.as_ref()));
    let overlay_id = context_id(tool_context.and_then(|context| context.overlay_id.as_ref()));

    let mut sections: Vec<String> = Vec::new();
    if app_id.is_some() || overlay_id.is_some() {
        let mut lines = vec![
            "## HOST CONTEXT".to_string(),
            "Tool calls default to these ids when you omit them; never ask the user to repeat them."
                .to_string(),
        ];
        if let Some(app_id) = app_id {
            lines.push(format!("- app_id: {app_id}"));
        }
        if let Some(overlay_id) = overlay_id {
            lines.push(format!("- overlay_id: {overlay_id}"));
        }
        sections.push(lines.join("\n"));
    }
    if let Some(guidance) = host_context_guidance
        .map(str::trim)
        .filter(|g| !g.is_empty())
    {
        sections.push(guidance.to_string());
    }
    sections.join("\n\n")
}

/// Run one nested specialist scope on the Bits/rig backend.
///
/// The board and UI specialists are served by the `UnifiedCopilot`; Data Studio, Scout, and Home
/// run the shared platform tool loop with their own prompt and tool set through the same frontend
/// bridge every other backend uses.
#[allow(clippy::too_many_arguments)]
async fn run_bits_specialist_chat(
    app_handle: AppHandle,
    state: Arc<flow_like::state::FlowLikeState>,
    scope: CopilotScope,
    specialist: PlatformSpecialist,
    user_prompt: String,
    model_id: Option<String>,
    auth_token: Option<String>,
    tool_context: Option<FrontendToolContext>,
    host_context_guidance: Option<String>,
    nested: bool,
    read_only: bool,
    request_id: Option<String>,
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let profile = copilot_profile(&app_handle).await;
    if let Some(error) = home_profile_scope_error(
        scope,
        tool_context.as_ref(),
        profile.as_ref().map(|profile| profile.id.as_str()),
    ) {
        return Err(error);
    }
    let stream_parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let (run_cancellation, _run_registration) = register_copilot_run(
        request_id
            .as_deref()
            .or(stream_parent_request_id.as_deref()),
    );
    // Every backend uses the same mutation-lane gates. Data changes serialize per app, while Home
    // changes serialize against the process-wide active profile.
    let _nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_copilot_run_gate_key(
                    scope,
                    None,
                    tool_context.as_ref(),
                )),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };

    let context = specialist_host_context(tool_context.as_ref(), host_context_guidance.as_deref());
    let tool_channel = frontend_tool_channel(
        tool_context.as_ref(),
        request_id
            .as_deref()
            .or(stream_parent_request_id.as_deref()),
    )
    .await;
    let frontend_bridge = if nested {
        crate::functions::ai::frontend_tool_bridge::FrontendToolBridge::new_with_event(
            app_handle.clone(),
            crate::functions::ai::frontend_tool_bridge::GLOBAL_FRONTEND_TOOL_EVENT,
            tool_channel,
        )
    } else {
        crate::functions::ai::frontend_tool_bridge::FrontendToolBridge::new(
            app_handle.clone(),
            tool_channel,
        )
    }
    .with_context(tool_context);
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(DesktopPlatformBridge {
        bridge: frontend_bridge,
        tool_set: match specialist {
            PlatformSpecialist::DataStudio => FrontendPlatformToolSet::DataStudio,
            PlatformSpecialist::Scout => FrontendPlatformToolSet::Scout,
            PlatformSpecialist::Home if read_only => FrontendPlatformToolSet::HomeReadOnly,
            PlatformSpecialist::Home => FrontendPlatformToolSet::Home,
        },
        cancellation: run_cancellation.clone(),
        // A delegated specialist is not steerable; the user steers the orchestrator that called it.
        steerable: false,
    });

    let on_token = move |token: String| {
        let _ = channel.send(token);
    };
    let specialist_chat = run_specialist_chat_with_access(
        state,
        profile,
        specialist,
        read_only,
        context,
        user_prompt,
        model_id,
        auth_token,
        bridge,
        Some(on_token),
    );
    let message = tokio::select! {
        result = specialist_chat => result.map_err(|error| error.to_string())?,
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Bits run was cancelled".to_string());
        }
    };

    Ok(UnifiedCopilotResponse {
        message,
        commands: Vec::new(),
        components: Vec::new(),
        canvas_settings: None,
        root_component_id: None,
        flowscript_workspace: None,
        flow_ir_commit: None,
        suggestions: Vec::new(),
        active_scope: scope,
    })
}

/// Unified copilot chat command that handles both board and UI generation
#[tauri::command]
pub async fn copilot_chat(
    app_handle: AppHandle,
    webview: tauri::Webview,
    state: State<'_, TauriFlowLikeState>,
    // Scope selection
    scope: CopilotScope,
    // Board context (optional for Frontend scope)
    board: Option<Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: Option<Vec<String>>,
    // UI context (optional for Board scope)
    current_surface: Option<Vec<SurfaceComponent>>,
    // The surface's persisted canvasSettings, customCss included. The UI specialist edits an
    // existing stylesheet only if it can see it.
    current_canvas_settings: Option<serde_json::Value>,
    selected_component_ids: Option<Vec<String>>,
    // Common parameters
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    token: Option<String>,
    // Extended context
    run_context: Option<RunContext>,
    action_context: Option<UIActionContext>,
    // Sub-agent run spawned while another Copilot session is mid-turn (needs its own CLI)
    nested: Option<bool>,
    // Read-only specialist sub-run: inspect and answer without mutation tools.
    read_only: Option<bool>,
    // App scope for hosted-model usage attribution. Omit for genuine global chat.
    app_id: Option<String>,
    // Runtime tools in a detached nested specialist are scoped by the frontend, not model input.
    tool_context: Option<FrontendToolContext>,
    // Stable frontend request id used for end-to-end cancellation of detached agent runs.
    request_id: Option<String>,
    // Immutable user-authored request, separate from host-added mode/run-context guidance.
    raw_user_prompt: Option<String>,
    // Streaming channel
    channel: Channel<String>,
) -> Result<UnifiedCopilotResponse, String> {
    let nested = nested.unwrap_or(false);
    if matches!(scope, CopilotScope::Home)
        && tool_context
            .as_ref()
            .is_some_and(|context| context.profile_id.is_some())
    {
        let current_profile = TauriSettingsState::current_profile(&app_handle).await.ok();
        if let Some(error) = home_profile_scope_error(
            scope,
            tool_context.as_ref(),
            current_profile
                .as_ref()
                .map(|profile| profile.hub_profile.id.as_str()),
        ) {
            return Err(error);
        }
    }
    let raw_user_prompt = raw_user_prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .or_else(|| {
            tool_context
                .as_ref()
                .and_then(|context| context.source_user_prompt.clone())
                .filter(|prompt| !prompt.trim().is_empty())
        })
        .unwrap_or_else(|| user_prompt.clone());
    // `Some` is an explicit tool/UI mode; `None` is Auto for the direct panel. Resolve this once
    // before choosing Bits, GitHub Copilot, Codex, or Claude so every backend receives the same
    // authoring surface and watchdog policy.
    let read_only = match read_only {
        Some(explicit) => explicit,
        None => is_read_only_workflow_request(&raw_user_prompt.to_lowercase()),
    };
    // The retained-draft identity and acceptance contract must survive across nested runs spawned
    // from one user turn. Delegated specialist instructions differ per nested run, so identity
    // binds to the outer chat's immutable source prompt whenever the tool context carries it,
    // scoped by the owning conversation id so identical prompt text from another conversation
    // never shares a draft lease; a genuinely different user request still produces a different
    // identity.
    let request_identity_prompt =
        request_identity_prompt_for(tool_context.as_ref(), &raw_user_prompt);
    let host_context_guidance = run_context.as_ref().map(|context| {
        format!(
            "## HOST RUN CONTEXT\nThe user is asking about execution run `{}` for app `{}` and board `{}`. Use the run/log query tools and ground the answer in that run.",
            context.run_id, context.app_id, context.board_id
        )
    });
    if !read_only
        && matches!(scope, CopilotScope::Board | CopilotScope::Both)
        && let Some(board) = board.as_ref()
        && let Some(delivery) =
            pending_flowscript_redelivery_for_request(&app_handle, board, &request_identity_prompt)
                .await
    {
        let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
        let workspace_status = if delivery.stale_board {
            "stale"
        } else {
            "queued"
        };
        send_correlated_stream_json_event(
            &channel,
            "flowscript_workspace",
            &serde_json::json!({
                "source": &delivery.source,
                "status": workspace_status,
            }),
            parent_request_id.as_deref(),
        );
        send_commands_event(&channel, &delivery.commands);
        return Ok(pending_flowscript_redelivery_response(scope, delivery));
    }
    // Full Node/WASM definitions received over IPC are display data, not an authority boundary.
    // Resolve the live app package catalog from the native registry for every board agent path.
    let _renderer_catalog_nodes = catalog_nodes;
    let catalog_nodes = if matches!(scope, CopilotScope::Board | CopilotScope::Both) {
        let catalog_app_id = resolve_catalog_app_id(
            tool_context
                .as_ref()
                .and_then(|context| context.app_id.as_deref()),
            app_id.as_deref(),
            run_context.as_ref().map(|context| context.app_id.as_str()),
            action_context
                .as_ref()
                .map(|context| context.app_id.as_str()),
        );
        authoritative_app_catalog_nodes(&app_handle, catalog_app_id.as_deref()).await
    } else {
        None
    };
    let _tool_owner = claim_run_tool_owner(
        webview.label(),
        tool_context.as_ref(),
        request_id.as_deref(),
    )
    .await;
    let model_selection = FlowPilotModelSelection::parse(model_id);
    if let FlowPilotChatBackend::Agent(agent_backend) = model_selection.backend {
        return match agent_backend {
            FlowPilotAgentBackendKind::GithubCopilot => {
                let model_id = model_selection
                    .model_id
                    .as_deref()
                    .filter(|model_id| !model_id.trim().is_empty())
                    .ok_or_else(|| "GitHub Copilot backend requires a model id".to_string())?;

                instrumented_agent_stage(
                    &app_handle,
                    agent_backend,
                    AGENT_STAGE_RUN,
                    copilot_sdk_chat_internal(
                        app_handle.clone(),
                        model_id,
                        reasoning_effort.as_deref(),
                        scope,
                        board.as_ref(),
                        catalog_nodes,
                        selected_node_ids.as_deref().unwrap_or(&[]),
                        current_surface.as_ref(),
                        current_canvas_settings.as_ref(),
                        user_prompt,
                        raw_user_prompt,
                        request_identity_prompt,
                        host_context_guidance,
                        current_images,
                        history.unwrap_or_default(),
                        channel,
                        None,
                        None,
                        tool_context,
                        request_id,
                        nested,
                        read_only,
                    ),
                )
                .await
            }
            FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
                let model_id = model_selection
                    .model_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string());

                instrumented_agent_stage(
                    &app_handle,
                    agent_backend,
                    AGENT_STAGE_RUN,
                    external_code_agent_chat_internal(
                        app_handle.clone(),
                        agent_backend,
                        &model_id,
                        reasoning_effort.as_deref(),
                        scope,
                        board.as_ref(),
                        catalog_nodes,
                        selected_node_ids.as_deref().unwrap_or(&[]),
                        current_surface.as_ref(),
                        current_canvas_settings.as_ref(),
                        user_prompt,
                        raw_user_prompt,
                        request_identity_prompt,
                        host_context_guidance,
                        current_images,
                        history.unwrap_or_default(),
                        channel,
                        None,
                        None,
                        tool_context,
                        request_id,
                        nested,
                        read_only,
                    ),
                )
                .await
            }
        };
    }

    // Data Studio, Scout, and Home are tool-loop specialists. The board/UI copilots cannot author
    // their artifacts, so on the Bits backend they run the shared platform loop with their own
    // prompt and tool set. Their availability is a property of the host, independent of the
    // selected model. Every FlowPilot backend advertises the same specialist tools.
    if let Some(specialist) = match scope {
        CopilotScope::DataStudio => Some(PlatformSpecialist::DataStudio),
        CopilotScope::Scout => Some(PlatformSpecialist::Scout),
        CopilotScope::Home => Some(PlatformSpecialist::Home),
        _ => None,
    } {
        return run_bits_specialist_chat(
            app_handle,
            state.0.clone(),
            scope,
            specialist,
            user_prompt,
            model_selection.model_id,
            token,
            tool_context,
            host_context_guidance,
            nested,
            read_only,
            request_id,
            channel,
        )
        .await;
    }

    // The Bits orchestrator runs the sealed researcher in-process (`research_agent` never leaves
    // the loop), so a delegated Research scope only arrives here from a backend mismatch.
    if matches!(scope, CopilotScope::Research) {
        let message = "The delegated research agent runs inside the orchestrator's own loop on this backend; call `research_agent` from the top-level assistant instead of delegating a Research scope.".to_string();
        // A nested run is a delegated specialist call from the frontend tool bridge, which reports
        // a resolved promise as `status: "ok"`. Returning Ok here would hand the orchestrator a
        // capability notice shaped exactly like specialist findings, which it then relays — or
        // rationalizes — as if the work had been done. Fail the call so the bridge reports an error,
        // and say the failure is permanent so the model does not spend its remaining rounds
        // retrying a delegation this host will never serve.
        if nested {
            return Err(format!(
                "{message} This is a permanent property of the selected backend, not a transient failure — do not retry this tool in this run."
            ));
        }
        let _ = channel.send(message.clone());
        return Ok(UnifiedCopilotResponse {
            message,
            commands: Vec::new(),
            components: Vec::new(),
            canvas_settings: None,
            root_component_id: None,
            flowscript_workspace: None,
            flow_ir_commit: None,
            suggestions: Vec::new(),
            active_scope: scope,
        });
    }

    flowpilot_debug_log!(
        "[copilot_chat] Called with scope: {:?}, run_context: {:?}",
        scope,
        run_context
    );

    let selected_node_ids = selected_node_ids.unwrap_or_default();
    let selected_component_ids = selected_component_ids.unwrap_or_default();
    let history = history.unwrap_or_default();

    let state_clone = state.0.clone();

    let profile = copilot_profile(&app_handle).await;

    let attribution_app_id = resolve_copilot_app_id(
        app_id.as_deref(),
        run_context.as_ref().map(|context| context.app_id.as_str()),
        action_context
            .as_ref()
            .map(|context| context.app_id.as_str()),
    )?;
    let usage_context = match attribution_app_id.as_deref() {
        Some(app_id) => {
            let app = App::load(app_id.to_string(), state_clone.clone())
                .await
                .map_err(|error| {
                    format!("Failed to resolve app for copilot usage attribution: {error}")
                })?;
            Some(ModelUsageContext {
                app_id: if matches!(app.visibility, AppVisibility::Offline) {
                    None
                } else {
                    Some(app_id.to_string())
                },
                run_id: run_context.as_ref().map(|context| context.run_id.clone()),
                api_base_url: None,
            })
        }
        None => None,
    };

    // Only create catalog provider if we might need it (Board or Both scope)
    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend | CopilotScope::Home => None,
        _ => Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes))),
    };

    // Profile/Bits board runs use the core rig loop rather than the SDK/MCP adapters. Attach the
    // same frontend execution bridge explicitly so provider choice does not remove runtime
    // verification tools. Detached nested board specialists must use the global bridge listener.
    let stream_parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let board_context_augmentation = tool_context
        .as_ref()
        .and_then(|context| context.board_context_manifest.clone());
    let (run_cancellation, _run_registration) = register_copilot_run(
        request_id
            .as_deref()
            .or(stream_parent_request_id.as_deref()),
    );
    // The in-process Bits/rig loop shares mutable editor state with the agent backends, so its
    // nested runs take the same mutation-lane gate. The permit is held for the entire run.
    let _nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_copilot_run_gate_key(
                    scope,
                    board.as_ref(),
                    tool_context.as_ref(),
                )),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    let tool_channel = frontend_tool_channel(
        tool_context.as_ref(),
        request_id
            .as_deref()
            .or(stream_parent_request_id.as_deref()),
    )
    .await;
    let runtime_frontend_bridge = if nested {
        crate::functions::ai::frontend_tool_bridge::FrontendToolBridge::new_with_event(
            app_handle.clone(),
            crate::functions::ai::frontend_tool_bridge::GLOBAL_FRONTEND_TOOL_EVENT,
            tool_channel,
        )
    } else {
        crate::functions::ai::frontend_tool_bridge::FrontendToolBridge::new(
            app_handle.clone(),
            tool_channel,
        )
    }
    .with_context(tool_context);
    let runtime_bridge: Arc<dyn PlatformToolBridge> = Arc::new(DesktopPlatformBridge {
        bridge: runtime_frontend_bridge,
        tool_set: FrontendPlatformToolSet::BoardRuntime,
        cancellation: run_cancellation.clone(),
        // Board runtime tools belong to a board/widget run, which is not steerable.
        steerable: false,
    });

    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        stream_parent_request_id.clone(),
        "bits",
        model_selection.model_id.as_deref().unwrap_or("default"),
        run_cancellation.clone(),
    );
    run_summary.record_phase();
    let workflow_session_snapshot = Arc::new(StdMutex::new(None));
    run_summary.attach_shared_session_snapshot(workflow_session_snapshot.clone());
    let workflow_edit_request =
        !read_only && matches!(scope, CopilotScope::Board | CopilotScope::Both) && board.is_some();

    let copilot_init =
        UnifiedCopilot::new(state_clone, catalog_provider, profile, None, usage_context);
    let copilot = tokio::select! {
        result = copilot_init => result.map_err(|error| error.to_string())?,
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Bits run was cancelled during initialization".to_string());
        }
    }
    // Bind draft/acceptance identity to the same conversation-scoped request identity the SDK and
    // external agent backends use, while `raw_user_prompt` keeps serving routing/classification.
    .with_request_identity_prompt(Some(request_identity_prompt));
    let mut copilot = copilot
        .with_board_context_augmentation(board_context_augmentation)
        .with_read_only(read_only)
        .with_workflow_session_snapshot_sink(workflow_session_snapshot);

    // Core exposes database/UI/storage inspection in every mode and withholds execute tools when
    // read-only. Attach one bridge for both rather than maintaining a provider-specific surface.
    copilot = copilot.with_runtime_bridge(runtime_bridge);

    let mut bits_draft_snapshot = None;
    if !read_only
        && !matches!(scope, CopilotScope::Frontend | CopilotScope::Home)
        && let Some(board) = board.as_ref()
    {
        let flow_ir_drafts = retained_flow_ir_draft_store_for_board(board)?;
        let board_key = board.id.clone();
        let snapshot_store = flow_ir_drafts.clone();
        let snapshot_board_key = board_key.clone();
        copilot = copilot
            .with_flow_ir_draft_store(flow_ir_drafts.clone())
            .with_flow_ir_draft_mutation_hook(Arc::new(move || {
                schedule_flow_ir_draft_snapshot(&snapshot_board_key, &snapshot_store);
            }));
        bits_draft_snapshot = Some((board_key, flow_ir_drafts));
    }

    let on_token = Some(move |token: String| {
        let token = correlate_stream_frame(&token, stream_parent_request_id.as_deref());
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

    let chat = copilot.chat_with_raw_user_prompt(
        scope,
        board.as_ref(),
        &selected_node_ids,
        current_surface.as_ref(),
        current_canvas_settings.as_ref(),
        &selected_component_ids,
        user_prompt,
        Some(raw_user_prompt),
        current_images,
        history,
        model_selection.model_id,
        token,
        context,
        on_token,
    );
    let chat_result = tokio::select! {
        result = chat => result.map_err(|error| error.to_string()),
        _ = run_cancellation.cancelled() => {
            Err("FlowPilot Bits run was cancelled".to_string())
        }
    };
    if let Some((board_key, store)) = bits_draft_snapshot.as_ref() {
        // Flush/supersede the debounce generation on every terminal path as a final safety net;
        // per-tool hooks above already cover long-running repair loops.
        schedule_flow_ir_draft_snapshot(board_key, store);
    }
    if let Ok(response) = &chat_result {
        run_summary.set_applied_commands(response.commands.len());
        if !response.commands.is_empty() || response.flow_ir_commit.is_some() {
            run_summary.set_outcome("committed");
        } else {
            run_summary.resolve_outcome(false, workflow_edit_request);
        }
    } else {
        run_summary.resolve_outcome(true, workflow_edit_request);
    }
    chat_result
}

/// Collects self-awareness context for the global assistant: the signed-in user (supplied by the
/// frontend), the active profile, the names of the user's other profiles, and — when the user has a
/// board open — that board's identity. Gathers the Tauri-owned data (profiles, active profile) and
/// delegates the shared rendering to [`build_platform_context`] so desktop and server produce the
/// same context wording.
pub(super) async fn build_global_agent_context(
    app_handle: &AppHandle,
    active_profile: Option<&flow_like::profile::Profile>,
    user_context: Option<&str>,
    open_board: Option<&GlobalOpenBoardContext>,
    open_data_studio: Option<&GlobalDataStudioContext>,
    attachments: &[AttachmentManifestEntry],
) -> String {
    let switchable: Vec<String> =
        match crate::functions::settings::profiles::get_profiles(app_handle.clone()).await {
            Ok(profiles) => profiles
                .values()
                .map(|profile| {
                    let name = profile.hub_profile.name.trim();
                    if name.is_empty() {
                        profile.hub_profile.id.clone()
                    } else {
                        name.to_string()
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };

    build_platform_context(PlatformContextInput {
        user_context,
        active_profile: active_profile.map(|profile| (profile.name.as_str(), profile.id.as_str())),
        switchable_profiles: &switchable,
        open_board,
        open_data_studio,
        attachments,
    })
}
