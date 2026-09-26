//! GitHub Copilot SDK chat sessions and event handling.

use super::agent_surface::{build_flowpilot_agent_surface, live_board_handle};
use super::attachments::build_copilot_attachments;
use super::backend_types::FlowPilotAgentBackendKind;
use super::client_pool::{
    COPILOT_CLIENT, COPILOT_SINGLETON_CLIENT_GATE, NestedCopilotClientLease,
    acquire_nested_copilot_run_permit, checkout_nested_copilot_client,
    checkout_top_level_copilot_client, nested_copilot_run_gate, nested_copilot_run_gate_key,
    quarantine_nested_copilot_client,
};
use super::external_continuation::workflow_edit_continuation_prompt;
use super::external_invocation::explicit_reasoning_effort;
use super::global_chat::drain_global_chat_steering;
use super::runtime::{
    SDK_CHAT_ABORT_TIMEOUT, SDK_CONTROL_RPC_TIMEOUT, SDK_RESPONSE_MAX_BYTES,
    SDK_USAGE_CALLS_MAX_ENTRIES, SdkToolActivityRegistry, frontend_tool_channel,
    register_copilot_run,
};
use super::stream_events::{
    announce_tool_start, append_bounded_text, close_pending_tool_steps,
    direct_sdk_tool_result_stream_status, extract_json_status,
    flowscript_response_workspace_envelope, flowscript_workspace_result_payload,
    preview_tool_result, scoped_parent_request_id, send_commands_event,
    send_correlated_stream_json_event, send_stream_json_event, summarize_tool_result,
};
use super::tool_policy::{
    SideEffectCommandQueueCleanup, abandon_side_effect_commands, build_flowpilot_sdk_tools,
    drain_streamable_side_effect_commands, is_flowpilot_read_only_tool, take_side_effect_delivery,
};
use super::workflow_diagnostics::{
    workflow_result_clears_repair, workflow_result_diagnostics, workflow_result_fallback_message,
    workflow_result_requires_repair,
};
use super::workflow_reporting::WorkflowRunSummaryEmitter;
use super::workflow_sdk::{
    IdleContinuationBudget, full_redacted_tool_result, guard_sdk_workflow_tools,
    is_flowscript_draft_operation_tool, prepare_sdk_idle_continuation_budget,
    scope_sdk_tool_handlers, workflow_state_has_retained_candidate,
};
use super::workflow_state::WorkflowToolLoopState;
use crate::functions::ai::frontend_tool_bridge::FrontendToolContext;
use copilot_sdk::{Client, MessageOptions};
use flow_like::{
    a2ui::SurfaceComponent,
    copilot::{ChatImage, CopilotScope, UnifiedChatMessage, UnifiedCopilotResponse},
    flow::{
        board::Board,
        copilot::{BoardCommand, memory::AssistantMemory, workflow_authoring_tool_allowed},
        node::Node,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};
use tauri::{AppHandle, ipc::Channel};

/// Internal function to handle Copilot SDK chat
pub(super) async fn copilot_sdk_chat_internal(
    app_handle: AppHandle,
    model_id: &str,
    reasoning_effort: Option<&str>,
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    current_canvas_settings: Option<&serde_json::Value>,
    user_prompt: String,
    raw_user_prompt: String,
    request_identity_prompt: String,
    host_context_guidance: Option<String>,
    current_images: Option<Vec<ChatImage>>,
    history: Vec<UnifiedChatMessage>,
    channel: Channel<String>,
    global: Option<String>,
    memory: Option<Arc<AssistantMemory>>,
    tool_context: Option<FrontendToolContext>,
    request_id: Option<String>,
    nested: bool,
    read_only: bool,
) -> Result<UnifiedCopilotResponse, String> {
    use copilot_sdk::SessionEventData;

    const MAX_WORKFLOW_IDLE_CONTINUATIONS: u8 = 2;

    let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let nested_gate_key = nested_copilot_run_gate_key(scope, board, tool_context.as_ref());
    let (run_cancellation, _run_registration) =
        register_copilot_run(request_id.as_deref().or(parent_request_id.as_deref()));

    let live_board = live_board_handle(&app_handle, board);
    let live_board_snapshot = live_board.as_ref().map(|live_board| live_board.snapshot());
    let authoritative_board = live_board_snapshot.as_deref().or(board);
    let public_user_prompt = tool_context
        .as_ref()
        .and_then(|context| context.source_user_prompt.as_deref())
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or(&raw_user_prompt);
    let mut surface = build_flowpilot_agent_surface(
        scope,
        authoritative_board,
        catalog_nodes,
        selected_node_ids,
        current_surface,
        current_canvas_settings,
        current_images
            .as_ref()
            .is_some_and(|images| !images.is_empty()),
        &history,
        &raw_user_prompt,
        public_user_prompt,
        &request_identity_prompt,
        host_context_guidance.as_deref(),
        global.as_deref(),
        FlowPilotAgentBackendKind::GithubCopilot.web_research(),
        tool_context
            .as_ref()
            .and_then(|context| context.board_context_manifest.as_ref()),
        read_only,
    );
    surface.live_board = live_board;
    let side_effect_commands = surface.side_effect_commands.clone();
    let _side_effect_cleanup = SideEffectCommandQueueCleanup(side_effect_commands.clone());
    let queued_flowscript = surface.queued_flowscript.clone();
    let emitted_surfaces = surface.emitted_surfaces.clone();
    let workflow_edit_request = surface.workflow_edit_request;
    let workflow_state = workflow_edit_request.then(|| {
        let mut state =
            WorkflowToolLoopState::from_flowscript_recovery(surface.flowscript_recovery.as_ref());
        state.attach_shared_session(surface.workflow_manifest.clone());
        Arc::new(StdMutex::new(state))
    });
    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        parent_request_id.clone(),
        "github-copilot",
        model_id,
        run_cancellation.clone(),
    );
    run_summary.set_continuation_limit(u32::from(MAX_WORKFLOW_IDLE_CONTINUATIONS));
    run_summary.attach_workflow_state(workflow_state.clone());
    run_summary.record_phase();

    let tool_channel = frontend_tool_channel(tool_context.as_ref(), request_id.as_deref()).await;
    let mut tools = build_flowpilot_sdk_tools(
        app_handle,
        scope,
        &surface,
        global.is_some(),
        nested,
        tool_context,
        memory,
        &raw_user_prompt,
        tool_channel,
    );
    if read_only && matches!(scope, CopilotScope::DataStudio) {
        tools.clear();
    } else if read_only {
        tools.retain(|(tool, _)| is_flowpilot_read_only_tool(&tool.name));
    } else if workflow_edit_request {
        tools.retain(|(tool, _)| workflow_authoring_tool_allowed(&tool.name));
        tools = guard_sdk_workflow_tools(
            tools,
            workflow_state
                .as_ref()
                .expect("workflow state exists for mutation sessions")
                .clone(),
        );
    }
    super::workflow_benchmark::observe_tools(board, &mut tools);
    let sdk_tool_activity = Arc::new(SdkToolActivityRegistry::default());
    let mut sdk_tool_activity_rx = sdk_tool_activity.subscribe();
    tools = scope_sdk_tool_handlers(
        tools,
        run_cancellation.clone(),
        Some(sdk_tool_activity.clone()),
    );

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
        reasoning_effort: explicit_reasoning_effort(reasoning_effort).map(str::to_string),
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

    flowpilot_debug_log!(
        "[copilot_sdk_chat] start (model: {model_id}, global: {}, nested: {nested}, tools: {})",
        global.is_some(),
        allowed_tool_names.len()
    );

    // Same-board nested runs must not interleave (retained draft base-fingerprint integrity).
    // Keep the per-board permit for the entire run. Queueing behind the current owner has no
    // arbitrary timeout, but explicit cancellation wins.
    let nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_gate_key),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };

    // Every run needs a CLI process it owns exclusively for its duration — the process serializes
    // requests, so two live sessions on one process deadlock. Nested runs always take a pooled
    // process. Top-level runs claim the shared client first (a single turn spawns nothing extra)
    // and fall back to their own pool once another turn already holds it.
    //
    // The client slot is cloned before awaiting any RPC: a wedged create_session must not hold the
    // global mutex and block stop/status/recovery calls.
    // Only global-chat runs are registered as steerable; for any other run the drain is a no-op,
    // so this can be taken from the request id unconditionally.
    let global_run_id = request_id.clone();
    let mut singleton_client_permit = None;
    let nested_client_lease = if nested {
        Some(checkout_nested_copilot_client(run_cancellation.clone()).await?)
    } else {
        match COPILOT_SINGLETON_CLIENT_GATE.clone().try_acquire_owned() {
            Ok(permit) => {
                singleton_client_permit = Some(permit);
                None
            }
            Err(_) => Some(checkout_top_level_copilot_client(run_cancellation.clone()).await?),
        }
    };
    let client = match nested_client_lease.as_ref() {
        Some(lease) => lease.client(),
        None => COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot SDK not running. Please start it first.")?,
    };

    let create_session = client.create_session(config);
    let session_result = tokio::select! {
        result = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, create_session) => {
            let result = result.map_err(|_| format!(
                "{} Copilot session creation exceeded {} seconds",
                if nested { "Nested" } else { "GitHub" },
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
            result.and_then(|result| result.map_err(|error| {
                if nested {
                    format!("Failed to create nested session: {error}")
                } else {
                    format!("Failed to create session: {error}")
                }
            }))
        }
        _ = run_cancellation.cancelled() => {
            Err("FlowPilot Copilot run was cancelled during session creation".to_string())
        }
    };
    let session = match session_result {
        Ok(session) => session,
        Err(error) => {
            if nested {
                quarantine_nested_copilot_client(&client).await;
            }
            return Err(error);
        }
    };
    struct CopilotSessionCleanup {
        client: Arc<Client>,
        session: Arc<copilot_sdk::Session>,
        session_id: String,
        nested_run_permit: Option<tokio::sync::OwnedSemaphorePermit>,
        nested_client_lease: Option<NestedCopilotClientLease>,
        /// Claim on the shared CLI process, held for exactly as long as the pooled lease would be:
        /// until this run's session is deleted. Releasing it earlier would let the next top-level
        /// turn create a session on a process that still has one.
        singleton_client_permit: Option<tokio::sync::OwnedSemaphorePermit>,
    }
    impl Drop for CopilotSessionCleanup {
        fn drop(&mut self) {
            let client = self.client.clone();
            let session = self.session.clone();
            let session_id = self.session_id.clone();
            let nested_run_permit = self.nested_run_permit.take();
            let nested_client_lease = self.nested_client_lease.take();
            let singleton_client_permit = self.singleton_client_permit.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _nested_run_permit = nested_run_permit;
                    let _singleton_client_permit = singleton_client_permit;
                    // Held past session deletion: a pooled client may only rejoin the idle pool
                    // once its previous session is gone, or the next checkout could deadlock the
                    // CLI process with a second concurrent session.
                    let _nested_client_lease = nested_client_lease;
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                    // Client::delete_session also evicts the SDK's local Arc<Session> cache.
                    let _ = tokio::time::timeout(
                        SDK_CHAT_ABORT_TIMEOUT,
                        client.delete_session(&session_id),
                    )
                    .await;
                });
            } else if let Some(lease) = nested_client_lease {
                // Without a runtime the pending session cannot be cleaned up; drop the process
                // from the pool instead of re-pooling a client with an undeleted session.
                lease.deregister();
            }
        }
    }
    // The SDK client keeps every session in its internal map until destroy succeeds. Ensure every
    // return path (including cancellation and parser errors) gets bounded best-effort cleanup.
    let _session_cleanup = CopilotSessionCleanup {
        client: client.clone(),
        session: session.clone(),
        session_id: session.session_id().to_string(),
        nested_run_permit,
        nested_client_lease,
        singleton_client_permit,
    };
    if nested {
        flowpilot_debug_log!("[copilot_sdk_chat] creating session on the nested CLI");
    } else {
        flowpilot_debug_log!("[copilot_sdk_chat] client lock acquired; creating session");
    }
    flowpilot_debug_log!(
        "[copilot_sdk_chat] session {} created",
        session.session_id()
    );
    // Register tool handlers
    for (tool, handler) in tools {
        tokio::select! {
            _ = session.register_tool_with_handler(tool, Some(handler)) => {}
            _ = run_cancellation.cancelled() => {
                return Err("FlowPilot Copilot run was cancelled while registering tools".to_string());
            }
            _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
                return Err(format!(
                    "FlowPilot Copilot tool registration exceeded {} seconds",
                    SDK_CONTROL_RPC_TIMEOUT.as_secs(),
                ));
            }
        }
    }

    // FlowPilot only exposes reviewed custom tools. Approve permission requests for those
    // tools (the CLI surfaces one before invoking them) and deny anything else so built-in
    // file/shell tools cannot run.
    let register_permission = session.register_permission_handler(move |req| {
        let tool_name = req.extension_data.get("toolName").and_then(|v| v.as_str());
        match tool_name {
            Some(name) if permission_allowed_tool_names.contains(name) => {
                copilot_sdk::PermissionRequestResult::approved()
            }
            _ => copilot_sdk::PermissionRequestResult::denied(),
        }
    });
    tokio::select! {
        _ = register_permission => {}
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Copilot run was cancelled while configuring permissions".to_string());
        }
        _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
            return Err(format!(
                "FlowPilot Copilot permission registration exceeded {} seconds",
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
        }
    }

    let mut events = session.subscribe();
    let attachments = current_images
        .as_ref()
        .filter(|images| !images.is_empty())
        .map(|images| build_copilot_attachments(images))
        .transpose()?;

    let send_message = session.send(MessageOptions {
        prompt: user_prompt,
        attachments,
        mode: None,
    });
    tokio::select! {
        result = send_message => result.map_err(|e| format!("Failed to send message: {e}"))?,
        _ = run_cancellation.cancelled() => {
            return Err("FlowPilot Copilot run was cancelled while sending its prompt".to_string());
        }
        _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
            return Err(format!(
                "FlowPilot Copilot prompt delivery exceeded {} seconds",
                SDK_CONTROL_RPC_TIMEOUT.as_secs(),
            ));
        }
    };
    flowpilot_debug_log!(
        "[copilot_sdk_chat] prompt sent on session {}; streaming events",
        session.session_id()
    );

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
    // Budget name that already received a bounded continuation slice, mirroring the external
    // phase loop: granting the same exhausted budget a second slice would only loop.
    let mut previous_idle_exhausted_budget: Option<String> = None;
    let mut tool_names_by_call_id: HashMap<String, String> = HashMap::new();
    let mut open_tool_call_ids: HashSet<String> = HashSet::new();
    let mut session_error_note: Option<String> = None;
    // Most recent mutating tool call that failed validation: (tool name, errors). Cleared when a
    // later call queues/renders. Feeds the idle-continuation nudge so a model that stops after a
    // failed edit gets told exactly what to fix instead of a generic "try again".
    let mut last_validation_errors: Option<(String, Vec<String>)> = None;
    // Token usage the SDK reports per turn (assistant.usage) — accumulated into one usage_stat frame
    // so the chat shows the agent's own model usage (mirrors the Bits/rig path in platform.rs).
    let mut usage_prompt_tokens: u64 = 0;
    let mut usage_completion_tokens: u64 = 0;
    let mut usage_cost: f64 = 0.0;
    let mut usage_has_cost = false;
    let mut usage_model: Option<String> = None;
    let mut usage_calls: Vec<serde_json::Value> = Vec::new();
    let mut last_sdk_event_at = tokio::time::Instant::now();

    loop {
        let event_inactivity_deadline = sdk_tool_activity.inactivity_deadline(last_sdk_event_at);
        let next_event = tokio::select! {
            result = events.recv() => {
                match &result {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        last_sdk_event_at = tokio::time::Instant::now();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
                result
            }
            _ = sdk_tool_activity_rx.changed() => {
                continue;
            }
            _ = run_cancellation.cancelled() => {
                let note = "the FlowPilot Copilot run was cancelled";
                let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                if nested {
                    quarantine_nested_copilot_client(&client).await;
                }
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && extracted_flowscript_workspace.is_none()
                {
                    return Err("FlowPilot Copilot run was cancelled".to_string());
                }
                session_error_note = Some(note.to_string());
                break;
            }
            _ = tokio::time::sleep_until(event_inactivity_deadline) => {
                let now = tokio::time::Instant::now();
                if sdk_tool_activity.inactivity_deadline(last_sdk_event_at) > now {
                    // A protocol-v3 handler began after this sleep was armed. Activity leases carry
                    // absolute tool deadlines even though the SDK has not broadcast the request
                    // events to this subscriber yet.
                    continue;
                }
                let inactive_seconds = now.duration_since(last_sdk_event_at).as_secs();
                let note = format!(
                    "the Copilot SDK event stream produced no activity for {} seconds",
                    inactive_seconds
                );
                let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, session.abort()).await;
                if nested {
                    quarantine_nested_copilot_client(&client).await;
                }
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(&note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && extracted_flowscript_workspace.is_none()
                    && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                {
                    return Err(format!("FlowPilot Copilot session timed out: {note}"));
                }
                // Preserve any exact retained draft/diagnostics so the bounded outer workflow
                // continuation can resume them instead of pretending the timed-out phase queued.
                session_error_note = Some(note);
                break;
            }
        };
        match next_event {
            Ok(event) => match &event.data {
                SessionEventData::AssistantMessageDelta(delta) => {
                    append_bounded_text(
                        &mut full_response,
                        &delta.delta_content,
                        SDK_RESPONSE_MAX_BYTES,
                    );
                    if !workflow_edit_request {
                        let _ = channel.send(delta.delta_content.clone());
                    }
                }
                SessionEventData::AssistantMessage(msg) => {
                    // Don't overwrite accumulated content unless it's truly final
                    if full_response.is_empty() {
                        append_bounded_text(
                            &mut full_response,
                            &msg.content,
                            SDK_RESPONSE_MAX_BYTES,
                        );
                    }
                }
                SessionEventData::AssistantUsage(data) => {
                    super::workflow_benchmark::observe_usage(
                        board,
                        data.input_tokens,
                        data.output_tokens,
                        None,
                    );
                    let input = data.input_tokens.unwrap_or(0.0).max(0.0).round() as u64;
                    let output = data.output_tokens.unwrap_or(0.0).max(0.0).round() as u64;
                    if input > 0 || output > 0 {
                        usage_prompt_tokens += input;
                        usage_completion_tokens += output;
                        if let Some(cost) = data.cost {
                            usage_cost += cost;
                            usage_has_cost = true;
                        }
                        if data.model.is_some() {
                            usage_model = data.model.clone();
                        }
                        if usage_calls.len() < SDK_USAGE_CALLS_MAX_ENTRIES {
                            usage_calls.push(serde_json::json!({
                                "model": data.model.clone().unwrap_or_default(),
                                "usage": {
                                    "prompt_tokens": input,
                                    "completion_tokens": output,
                                    "total_tokens": input + output,
                                    "cost": data.cost,
                                },
                            }));
                        }
                    }
                }
                SessionEventData::ToolExecutionStart(tool_event) => {
                    let newly_announced = tool_names_by_call_id
                        .insert(
                            tool_event.tool_call_id.clone(),
                            tool_event.tool_name.clone(),
                        )
                        .is_none();
                    open_tool_call_ids.insert(tool_event.tool_call_id.clone());
                    // The same call may already have been announced via the protocol v3
                    // external_tool.requested broadcast — don't emit a second tool_start.
                    if newly_announced {
                        announce_tool_start(
                            &channel,
                            &tool_event.tool_call_id,
                            &tool_event.tool_name,
                            tool_event.arguments.as_ref(),
                            &mut extracted_flowscript_workspace,
                            parent_request_id.as_deref(),
                        );
                    }
                }
                SessionEventData::ExternalToolRequested(request) => {
                    // Protocol v3 broadcasts custom tool calls as external_tool.requested and may
                    // never emit tool.execution_start for them — announce the step here so custom
                    // FlowPilot tools stream reliably.
                    let Some(tool_call_id) =
                        request.tool_call_id.clone().filter(|id| !id.is_empty())
                    else {
                        continue;
                    };
                    if tool_names_by_call_id.contains_key(&tool_call_id) {
                        continue;
                    }
                    let tool_name = request
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| "tool".to_string());
                    tool_names_by_call_id.insert(tool_call_id.clone(), tool_name.clone());
                    open_tool_call_ids.insert(tool_call_id.clone());
                    announce_tool_start(
                        &channel,
                        &tool_call_id,
                        &tool_name,
                        request.arguments.as_ref(),
                        &mut extracted_flowscript_workspace,
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionProgress(progress) => {
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": progress.tool_call_id,
                            "tool": tool_names_by_call_id.get(&progress.tool_call_id),
                            "message": flow_like::flow::copilot::stream::safe_text_preview(&progress.progress_message, 1_200),
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionPartialResult(partial) => {
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_progress",
                        &serde_json::json!({
                            "tool_call_id": partial.tool_call_id,
                            "tool": tool_names_by_call_id.get(&partial.tool_call_id),
                            "message": flow_like::flow::copilot::stream::safe_text_preview(&partial.partial_output, 1_200),
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::ToolExecutionComplete(tool_complete) => {
                    open_tool_call_ids.remove(&tool_complete.tool_call_id);
                    let completed_tool_name = tool_names_by_call_id
                        .remove(&tool_complete.tool_call_id)
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

                        if let Some(mut payload) = flowscript_workspace_result_payload(
                            &completed_tool_name,
                            &parsed,
                            extracted_flowscript_workspace.as_deref(),
                        ) {
                            if let Some(object) = payload.as_object_mut() {
                                object.insert(
                                    "tool_call_id".to_string(),
                                    serde_json::Value::String(
                                        tool_complete.tool_call_id.to_string(),
                                    ),
                                );
                            }
                            if let Some(workspace) =
                                payload.get("source").and_then(serde_json::Value::as_str)
                            {
                                extracted_flowscript_workspace = Some(workspace.to_string());
                            }
                            send_stream_json_event(&channel, "flowscript_workspace", &payload);
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

                        // Track raw and typed validation outcomes so idle handling can nudge with
                        // exact structured diagnostics instead of a representation-specific retry.
                        let diagnostics = workflow_result_diagnostics(Some(&parsed));
                        if workflow_result_requires_repair(&parsed, &diagnostics) {
                            let errors = if diagnostics.is_empty() {
                                workflow_result_fallback_message(&parsed)
                                    .into_iter()
                                    .collect()
                            } else {
                                diagnostics
                            };
                            last_validation_errors = Some((completed_tool_name.clone(), errors));
                        } else if workflow_result_clears_repair(&parsed) {
                            last_validation_errors = None;
                        }

                        // Queued board commands travel via the side-effect store. Direct/legacy
                        // batches can stream immediately; retained FlowScript batches stay in the
                        // queue until commands and their exact review token can be taken together.
                        if status == Some("queued") {
                            let commands =
                                drain_streamable_side_effect_commands(&side_effect_commands);
                            if !commands.is_empty() {
                                send_commands_event(&channel, &commands);
                                extracted_commands.extend(commands);
                                last_validated_commands = None;
                            }
                        }
                        // Rendered UI travels via the emitted-surfaces store (tool results no
                        // longer echo the tree). Drain the newest surface; keep the legacy
                        // result-echo parse as a fallback.
                        if status == Some("rendered") {
                            let emitted = emitted_surfaces
                                .lock()
                                .ok()
                                .and_then(|mut surfaces| surfaces.drain(..).next_back());
                            let (components, canvas, root_id) = match emitted {
                                Some(surface) => (
                                    serde_json::from_value::<Vec<SurfaceComponent>>(
                                        surface.components,
                                    )
                                    .unwrap_or_default(),
                                    Some(surface.canvas_settings),
                                    Some(surface.root_component_id),
                                ),
                                None => (
                                    parsed
                                        .get("components")
                                        .cloned()
                                        .and_then(|comps| {
                                            serde_json::from_value::<Vec<SurfaceComponent>>(comps)
                                                .ok()
                                        })
                                        .unwrap_or_default(),
                                    parsed.get("canvasSettings").cloned(),
                                    parsed
                                        .get("rootComponentId")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string),
                                ),
                            };
                            if let Some(canvas) = canvas {
                                extracted_canvas_settings = Some(canvas);
                            }
                            if let Some(root_id) = root_id {
                                extracted_root_component_id = Some(root_id);
                            }
                            if !components.is_empty() {
                                let comp_event = format!(
                                    "<components>{}</components>",
                                    serde_json::to_string(&components).unwrap_or_default()
                                );
                                let _ = channel.send(comp_event);
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
                    let terminal_status = result_content.and_then(extract_json_status);
                    let status = if !tool_complete.success {
                        "error"
                    } else {
                        result_content
                            .map(direct_sdk_tool_result_stream_status)
                            .unwrap_or("done")
                    };
                    let error_message = tool_complete.error.as_ref().map(|error| {
                        if error.message.is_empty() {
                            "Tool failed".to_string()
                        } else {
                            flow_like::flow::copilot::stream::safe_text_preview(&error.message, 600)
                        }
                    });
                    send_correlated_stream_json_event(
                        &channel,
                        "tool_end",
                        &serde_json::json!({
                            "tool_call_id": tool_complete.tool_call_id,
                            "tool": completed_tool_name,
                            "status": status,
                            "terminal_status": terminal_status,
                            // Kept for older clients while the detailed report uses terminal_status.
                            "result_status": terminal_status,
                            "result_summary": summarize_tool_result(result_content, error_message.as_deref()),
                            "result_preview": result_content.map(preview_tool_result),
                            // Full text for compiler-receipt evidence; previews truncate large
                            // commit results and corrupt the captured authored source. Redaction
                            // still applies — only truncation is lifted.
                            "result": is_flowscript_draft_operation_tool(&completed_tool_name)
                                .then(|| result_content.map(full_redacted_tool_result))
                                .flatten(),
                            "error": error_message,
                        }),
                        parent_request_id.as_deref(),
                    );
                }
                SessionEventData::SessionIdle(_) => {
                    // Idle is the CLI's turn boundary and the only point where a second
                    // `session.send` is safe — mid-turn it races the pending tool call and the
                    // process never answers. Anything the user typed while this turn ran gets
                    // folded in here, before any host-generated continuation is considered.
                    if let Some(run_id) = global_run_id.as_deref() {
                        let steering = drain_global_chat_steering(run_id).await;
                        if !steering.is_empty() {
                            let prompt = format!(
                                "The user sent this while you were working. Treat it as part of the current request and continue accordingly:\n{}",
                                steering.join("\n")
                            );
                            let steer_send = session.send(MessageOptions {
                                prompt,
                                attachments: None,
                                mode: None,
                            });
                            let steer_result = tokio::select! {
                                result = steer_send => result.map_err(|error| error.to_string()),
                                _ = run_cancellation.cancelled() => {
                                    Err("run cancelled before the steering message was sent".to_string())
                                }
                                _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
                                    Err("sending the steering message timed out".to_string())
                                }
                            };
                            match steer_result {
                                Ok(_) => continue,
                                Err(error) => {
                                    session_error_note =
                                        Some(format!("steering message not delivered: {error}"));
                                }
                            }
                        }
                    }

                    // v3 external tool calls may never get a tool.execution_complete event —
                    // close any still-open steps so the frontend doesn't keep spinners alive.
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "done",
                        None,
                        parent_request_id.as_deref(),
                    );

                    if extracted_commands.is_empty()
                        && let Some(commands) = last_validated_commands.take()
                    {
                        send_commands_event(&channel, &commands);
                        extracted_commands.extend(commands);
                    }

                    if extracted_commands.is_empty() {
                        let commands = drain_streamable_side_effect_commands(&side_effect_commands);
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

                    // Nudge the model to finish when it stalled mid-task: either a workflow-edit
                    // request that queued nothing, or ANY scope whose last mutating call failed
                    // validation and produced no successful follow-up (models often stop right
                    // after a failed edit_flowscript/emit_ui instead of fixing and retrying).
                    let failed_attempt_pending = last_validation_errors.is_some()
                        && extracted_commands.is_empty()
                        && extracted_components.is_empty();
                    let workflow_mutation_is_terminal = workflow_state
                        .as_ref()
                        .and_then(|state| state.lock().ok())
                        .is_some_and(|state| state.queued);
                    if ((workflow_edit_request
                        && extracted_commands.is_empty()
                        && !workflow_mutation_is_terminal)
                        || failed_attempt_pending)
                        && workflow_idle_continuations < MAX_WORKFLOW_IDLE_CONTINUATIONS
                    {
                        // A continuation that demands more edits must be executable on arrival:
                        // grant an exhausted loop budget the same bounded slice the external
                        // phase loop grants, and stop honestly when that exact budget was already
                        // granted one and burned it again.
                        match prepare_sdk_idle_continuation_budget(
                            workflow_state.as_ref(),
                            previous_idle_exhausted_budget.as_deref(),
                        ) {
                            IdleContinuationBudget::Terminal(reason) => {
                                session_error_note =
                                    Some(format!("stopped without queueing changes: {reason}"));
                                break;
                            }
                            IdleContinuationBudget::SliceGranted(budget) => {
                                previous_idle_exhausted_budget = Some(budget);
                            }
                            IdleContinuationBudget::Executable => {
                                previous_idle_exhausted_budget = None;
                            }
                        }
                        workflow_idle_continuations = workflow_idle_continuations.saturating_add(1);
                        run_summary.record_continuation();
                        run_summary.record_phase();
                        full_response.clear();
                        let workflow_snapshot = workflow_state
                            .as_ref()
                            .and_then(|state| state.lock().ok().map(|state| state.snapshot()));
                        let prompt = workflow_edit_continuation_prompt(
                            &raw_user_prompt,
                            extracted_flowscript_workspace.as_deref(),
                            workflow_idle_continuations,
                            last_validation_errors.as_ref(),
                            workflow_snapshot.as_ref(),
                        );
                        let continuation_send = session.send(MessageOptions {
                            prompt,
                            attachments: None,
                            mode: None,
                        });
                        let continuation_result = tokio::select! {
                            result = continuation_send => result.map_err(|error| error.to_string()),
                            _ = run_cancellation.cancelled() => {
                                Err("run cancelled before the continuation was sent".to_string())
                            }
                            _ = tokio::time::sleep(SDK_CONTROL_RPC_TIMEOUT) => {
                                Err(format!(
                                    "workflow continuation delivery exceeded {} seconds",
                                    SDK_CONTROL_RPC_TIMEOUT.as_secs(),
                                ))
                            }
                        };
                        match continuation_result {
                            Ok(_) => continue,
                            Err(e) => {
                                // Degrade instead of aborting: keep whatever the session already
                                // produced and surface the continuation failure as a note.
                                session_error_note = Some(format!(
                                    "failed to continue the workflow edit session: {e}"
                                ));
                                break;
                            }
                        }
                    }

                    break;
                }
                SessionEventData::SessionError(err) => {
                    let error_text = if err.message.trim().is_empty() {
                        err.error_type.clone()
                    } else {
                        format!("{}: {}", err.error_type, err.message)
                    };
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "error",
                        Some(&error_text),
                        parent_request_id.as_deref(),
                    );
                    let has_partial_output = !full_response.trim().is_empty()
                        || !extracted_commands.is_empty()
                        || !extracted_components.is_empty()
                        || extracted_flowscript_workspace.is_some();
                    if !has_partial_output
                        && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                    {
                        return Err(format!("Session error: {error_text}"));
                    }
                    session_error_note = Some(error_text);
                    break;
                }
                SessionEventData::SessionShutdown(_) | SessionEventData::Abort(_) => {
                    let note = "the Copilot session ended before the response completed";
                    close_pending_tool_steps(
                        &channel,
                        &mut open_tool_call_ids,
                        &tool_names_by_call_id,
                        "error",
                        Some(note),
                        parent_request_id.as_deref(),
                    );
                    if full_response.trim().is_empty()
                        && extracted_commands.is_empty()
                        && extracted_components.is_empty()
                        && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                    {
                        return Err(
                            "GitHub Copilot session ended before producing a response.".to_string()
                        );
                    }
                    session_error_note = Some(note.to_string());
                    break;
                }
                _ => {}
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                // The event buffer overflowed; skipping events is recoverable — terminating here
                // would silently kill the run mid-stream.
                eprintln!(
                    "[copilot_sdk_chat] Event stream lagged, skipped {skipped} events; continuing"
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let note = "the Copilot event stream closed before the session finished";
                flowpilot_debug_log!("[copilot_sdk_chat] Event stream closed before session idle");
                close_pending_tool_steps(
                    &channel,
                    &mut open_tool_call_ids,
                    &tool_names_by_call_id,
                    "error",
                    Some(note),
                    parent_request_id.as_deref(),
                );
                if full_response.trim().is_empty()
                    && extracted_commands.is_empty()
                    && extracted_components.is_empty()
                    && !workflow_state_has_retained_candidate(workflow_state.as_ref())
                {
                    return Err(
                        "GitHub Copilot stopped before producing a response (event stream closed)."
                            .to_string(),
                    );
                }
                session_error_note = Some(note.to_string());
                break;
            }
        }
    }

    if run_cancellation.is_cancelled() {
        // A retained commit is held in the queue with its commands until final delivery.
        // Cancellation is not a successful response handoff: abandon both atomically so the exact
        // revision is reopened instead of returning an orphaned command batch or token.
        abandon_side_effect_commands(&side_effect_commands);
    }

    // Collect the final native tail and its exact review token under one queue lock. Retained
    // batches are never eligible for the streaming drains above, so a poisoned/failing final lock
    // cannot expose their commands without the matching token.
    let (commands, flow_ir_commit) = take_side_effect_delivery(&side_effect_commands);
    if !commands.is_empty() {
        send_commands_event(&channel, &commands);
        extracted_commands.extend(commands);
    }

    // Same fallback for rendered UI: if the session ended before the "rendered" tool event was
    // observed, the emitted-surfaces store still holds the tree.
    if extracted_components.is_empty()
        && let Some(surface) = emitted_surfaces
            .lock()
            .ok()
            .and_then(|mut surfaces| surfaces.drain(..).next_back())
    {
        let components =
            serde_json::from_value::<Vec<SurfaceComponent>>(surface.components).unwrap_or_default();
        if !components.is_empty() {
            let comp_event = format!(
                "<components>{}</components>",
                serde_json::to_string(&components).unwrap_or_default()
            );
            let _ = channel.send(comp_event);
            let canvas_event = format!(
                "<canvas_settings>{}</canvas_settings>",
                serde_json::to_string(&surface.canvas_settings).unwrap_or_default()
            );
            let _ = channel.send(canvas_event);
            extracted_canvas_settings = Some(surface.canvas_settings);
            extracted_root_component_id = Some(surface.root_component_id);
            extracted_components = components;
        }
    }

    // Publish the session's own token usage as a usage_stat frame (once, after the loop so
    // workflow-edit continuations aggregate into a single stat). Labeled by role so nested board/UI
    // sub-runs are distinguishable from the top-level assistant in the stats sheet.
    if !usage_calls.is_empty() {
        let step_name = if global.is_some() {
            "Assistant"
        } else {
            match scope {
                CopilotScope::Board => "Board copilot",
                CopilotScope::Frontend => "UI copilot",
                CopilotScope::Both => "Copilot",
                CopilotScope::DataStudio => "Data Studio agent",
                CopilotScope::Scout => "Project scout",
                CopilotScope::Home => "Home designer",
                CopilotScope::Research => "Researcher",
            }
        };
        send_correlated_stream_json_event(
            &channel,
            "usage_stat",
            &serde_json::json!({
                "step_name": step_name,
                "stats": {
                    "usage": {
                        "prompt_tokens": usage_prompt_tokens,
                        "completion_tokens": usage_completion_tokens,
                        "total_tokens": usage_prompt_tokens + usage_completion_tokens,
                        "cost": usage_has_cost.then_some(usage_cost),
                    },
                    "model": usage_model,
                    "iterations": usage_calls.len(),
                    "calls": usage_calls,
                },
            }),
            parent_request_id.as_deref(),
        );
    }

    // ── Fallback: if the model didn't call emit_ui but dumped JSON in the
    // response text, extract components from there so they still show up.
    if extracted_components.is_empty()
        && matches!(scope, CopilotScope::Frontend | CopilotScope::Both)
    {
        let surface = flow_like::a2ui::copilot::extract_surface_from_response(&full_response);
        if !surface.components.is_empty() {
            flowpilot_debug_log!(
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

    let workflow_snapshot = workflow_state
        .as_ref()
        .and_then(|state| state.lock().ok().map(|state| state.snapshot()));
    let has_retained_workflow_candidate = workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.last_flowscript.as_ref())
        .is_some();

    let modular_fallback_queued = workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.modular_fallback.as_ref())
        .is_some();
    let final_message = if workflow_edit_request {
        if !extracted_commands.is_empty() && modular_fallback_queued {
            "Queued an independently runnable partial working slice for review. The requested application is still incomplete; the fuller failed FlowScript remains retained for another repair pass. Do not treat this as full completion."
                .to_string()
        } else if !extracted_commands.is_empty() {
            "Queued workflow changes for review. Fill placeholder secrets before running."
                .to_string()
        } else if workflow_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.queued)
        {
            "This workflow draft revision was already queued earlier, so FlowPilot did not enqueue duplicate board commands."
                .to_string()
        } else if extracted_flowscript_workspace.is_some() || has_retained_workflow_candidate {
            "Workflow draft needs attention: no board commands were queued. Check the latest FlowScript compiler diagnostics, repair the retained source revision, and commit that same draft."
                .to_string()
        } else {
            "FlowPilot could not produce board commands or retain a workflow draft for this request."
                .to_string()
        }
    } else {
        full_response
    };

    let final_message = match &session_error_note {
        Some(note) if final_message.trim().is_empty() => {
            format!("The run ended early: {note}")
        }
        Some(note) => format!("{final_message}\n\n> Note: the run ended early ({note})."),
        None => final_message,
    };

    run_summary.set_applied_commands(extracted_commands.len());
    run_summary.resolve_outcome(session_error_note.is_some(), workflow_edit_request);

    // Preserve the best failed candidate for another repair turn, but pair source and status in one
    // envelope. The frontend applies only explicit `queued`; validation candidates remain visible
    // without becoming board mutations.
    let queued_workspace = queued_flowscript
        .lock()
        .ok()
        .and_then(|workspace| workspace.clone());
    let validated_flowscript_workspace = queued_workspace
        .as_deref()
        .map(|source| {
            flowscript_response_workspace_envelope(source, "queued", workflow_snapshot.as_ref())
        })
        .or_else(|| {
            workflow_snapshot.as_ref().and_then(|snapshot| {
                snapshot.last_flowscript.as_deref().map(|source| {
                    flowscript_response_workspace_envelope(
                        source,
                        snapshot
                            .last_status
                            .as_deref()
                            .unwrap_or("validation_errors"),
                        Some(snapshot),
                    )
                })
            })
        });

    Ok(UnifiedCopilotResponse {
        message: final_message,
        commands: extracted_commands,
        suggestions: vec![],
        components: extracted_components,
        canvas_settings: extracted_canvas_settings,
        root_component_id: extracted_root_component_id,
        flowscript_workspace: validated_flowscript_workspace,
        flow_ir_commit,
        active_scope: scope,
    })
}
