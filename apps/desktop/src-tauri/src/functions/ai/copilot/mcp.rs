//! MCP server dispatch, tool activity, and HTTP bridge lifetime.

use super::delegated_completion::{DelegatedDrainError, DelegatedPhase, is_owned_delegation};
use super::mcp_progress::{
    FlowPilotMcpTool, McpProgressHeartbeat, McpToolCancellationGuard,
    record_delegated_run_tool_progress,
};
use super::runtime::{EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT, EXTERNAL_AGENT_SHUTDOWN_TIMEOUT};
use super::workflow_diagnostics::workflow_status_requires_repair;
use super::workflow_observation::{
    workflow_tool_abort_with_args, workflow_tool_record_with_outcome,
};
use super::workflow_preflight::{workflow_candidate_preflight, workflow_tool_preflight_with_args};
use super::workflow_results::{
    annotate_modular_fallback_result, suppress_unchanged_flowscript_source_echo,
};
use super::workflow_sdk::{
    ExternalContextPreflight, WorkflowToolDispatchObserver, is_order_sensitive_workflow_tool,
    workflow_database_setup_preflight, workflow_loop_result,
    workflow_predraft_context_preflight_with_lease,
};
use super::workflow_state::{WorkflowToolLoopSnapshot, WorkflowToolLoopState};
use flow_like::flow::copilot::{
    tool_spec::{RESEARCH_AGENT_TOOL, find_home_tool_spec},
    workflow_tool_result_succeeded,
};
use flow_like_types::tokio_util::sync::CancellationToken;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Claude Code spills MCP results above its default size to a file the agent cannot Read, which
/// would hide complete Home layouts. 500,000 characters is Claude Code's hard ceiling.
const MAX_RESULT_SIZE_META_KEY: &str = "anthropic/maxResultSizeChars";
const HOME_TOOL_MAX_RESULT_SIZE_CHARS: u64 = 500_000;

fn home_tool_result_size_meta() -> rmcp::model::Meta {
    let mut meta = rmcp::model::Meta::new();
    meta.0.insert(
        MAX_RESULT_SIZE_META_KEY.to_string(),
        HOME_TOOL_MAX_RESULT_SIZE_CHARS.into(),
    );
    meta
}

#[derive(Clone)]
struct FlowPilotMcpServer {
    tools: Arc<HashMap<String, FlowPilotMcpTool>>,
    workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    tool_activity: Arc<StdMutex<McpToolActivityState>>,
    handler_quiescence: Arc<tokio::sync::Notify>,
    workflow_operation_gate: Arc<tokio::sync::Mutex<()>>,
    dispatch_observer: WorkflowToolDispatchObserver,
    phase_closed: Arc<AtomicBool>,
    delegated_phase: Option<Arc<DelegatedPhase>>,
}

impl FlowPilotMcpServer {
    fn new(
        tools: Arc<HashMap<String, FlowPilotMcpTool>>,
        workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
        tool_activity: Arc<StdMutex<McpToolActivityState>>,
        handler_quiescence: Arc<tokio::sync::Notify>,
        workflow_operation_gate: Arc<tokio::sync::Mutex<()>>,
        phase_closed: Arc<AtomicBool>,
        delegated_phase: Option<Arc<DelegatedPhase>>,
    ) -> Self {
        let dispatch_observer = WorkflowToolDispatchObserver::new(workflow_state.as_ref(), "mcp");
        Self {
            tools,
            workflow_state,
            tool_activity,
            handler_quiescence,
            workflow_operation_gate,
            dispatch_observer,
            phase_closed,
            delegated_phase,
        }
    }

    fn to_mcp_tool(tool: &copilot_sdk::Tool) -> rmcp::model::Tool {
        let schema = match &tool.parameters_schema {
            serde_json::Value::Object(_) => tool.parameters_schema.clone(),
            _ => serde_json::json!({ "type": "object", "properties": {} }),
        };

        let mcp_tool = rmcp::model::Tool::new(
            tool.name.clone(),
            tool.description.clone(),
            rmcp::model::object(schema),
        );
        if find_home_tool_spec(&tool.name).is_some() {
            mcp_tool.with_meta(home_tool_result_size_meta())
        } else {
            mcp_tool
        }
    }
}

/// The orchestrator's instructions differ only in how it reaches the public web.
macro_rules! orchestrator_instructions {
    ($public_web:literal) => {
        concat!(
            "You are the FlowPilot platform orchestrator. Use three modes. DIRECT handles ordinary one-call, one-app, or simple two-app tasks without planning. COMPLEX SOLVE makes a dependency plan only for work likely to need at least three apps/interfaces or intrinsic multi-stage, reconciliation, approval, verification, or recovery complexity. Begin app work with list_apps. Active configured chat/page/headless Events, including REST/API and MCP, are primary. Choose the best match and exact consumer. Call data_studio_agent directly for app data work on existing apps as well as during a build; it needs no preflight. Report a failed, declined, timed-out, or approval-blocked Event as a stop. Use flowpilot_home only when the user explicitly requests work on the current profile's Home landing page. Keep it out of ordinary app builds; in a mixed request, delegate Home as a separate work item. ",
            $public_web,
            " BUILD: use project_scout for prior art, then create, fork, or acquire a base and coordinate flowpilot_widget, data_studio_agent, flowpilot_board, Events, and safe runtime verification by dependency wave. Home layout, board logic, UI, and data are strict specialist boundaries. Preserve exact returned IDs, approvals, partial work, and the user's acceptance contract. Never claim success from a requested, declined, timed-out, or unknown operation. Do not use shell or file-edit tools for FlowPilot artifacts."
        )
    };
}

pub(super) fn flowpilot_mcp_server_instructions<'a>(
    tool_names: impl IntoIterator<Item = &'a str>,
    workflow_mutation: bool,
) -> &'static str {
    let names = tool_names.into_iter().collect::<HashSet<_>>();
    let has_board = names.contains("get_current_flowscript")
        || names.contains("list_board_nodes")
        || names.contains("write_flowscript");
    let has_ui = names.contains("emit_ui");
    let has_data = names.contains("graph_overlay_tool") || names.contains("graph_query_tool");
    let has_home = names.contains("get_home_context") || names.contains("apply_home_layout");
    let can_apply_home = names.contains("apply_home_layout");
    let has_global = names.contains("list_apps") && names.contains("flowpilot_board");

    if has_global {
        return if names.contains(RESEARCH_AGENT_TOOL) {
            orchestrator_instructions!(
                "Call the sealed no-argument research_agent in the first wave for plainly public reference work naming no app or private data; otherwise only after the inventory has no suitable local app or useful local research candidates returned no answer."
            )
        } else {
            orchestrator_instructions!(
                "Research public facts yourself with your built-in web search and fetch; a local research app is optional, and one that errors never blocks web research. Queries carry only public subject matter, never app records, internal IDs, credentials, or personal data. Specialists have no web access: pass them the verified facts with source URLs."
            )
        };
    }

    if has_home {
        return if can_apply_home {
            "You are the FlowPilot HOME specialist. Own only the current profile's Home landing-page layout JSON. Inspect the current context and widget catalog, discover referenced apps and data sources when useful, validate the complete candidate, then stage it with apply_home_layout. Never author A2UI pages, FlowScript, app data, or another profile's layout. Do not use shell or file-edit tools for FlowPilot artifacts."
        } else {
            "You are the read-only FlowPilot HOME specialist. Inspect the current profile's Home layout and supported references, then answer without staging a change. Never author A2UI pages, FlowScript, app data, or another profile's layout. Do not use shell or file-edit tools for FlowPilot artifacts."
        };
    }

    if workflow_mutation && has_ui {
        return "This is an explicit combined root FlowPilot surface, not a widget or board specialist. Keep UI changes in emit_ui and executable workflow behavior in the FlowScript lifecycle; never let UI generation author FlowScript or let board generation emit components. For the board portion, the FlowScript render embedded in the system prompt IS the current board — do not call get_current_flowscript before authoring; re-read only after the host applies an incremental segment. Make one bounded get_declarations batch for the highest-leverage catalog calls, call plan_board_scope exactly once, then retain the accepted active segment with write_flowscript. For a focused existing-board change, omit source and pass edits: [{old_text, new_text}] with exact unique matches from the embedded board; the host retains the complete merged source. Use full source for new boards or broad rewrites. After a plan is accepted, do not call plan_board_scope again unless its tool result explicitly authorizes one revision. Do not enumerate every utility or chase omitted queries before that checkpoint. Repair the retained source with patch_flowscript and use structured compiler diagnostics for focused declaration follow-ups; once a write or patch returns zero diagnostics, use test_flowscript for an eligible isolated data transform with an exact revision, entry, payload and expected output. A failing test supplies evidence for a focused patch and retest. Unsupported boards remain untested; preserve the requested behavior. Then finish with commit_flowscript at the retained revision; commit validates inline and returns validation_errors on failure. check_flowscript is only the staged-plan growth gate or a re-validation after catalog drift or a host-applied segment. Before the first write, use at most six ancillary database/UI/storage inspections.";
    }
    if workflow_mutation {
        return "You are the FlowPilot BOARD specialist. FlowScript is the sole model-authored representation for executable workflow behavior. The FlowScript render embedded in the system prompt IS the current board — do not call get_current_flowscript before authoring; re-read only after the host applies an incremental segment. Make one bounded get_declarations batch for the highest-leverage catalog calls needed to establish the end-to-end shape, call plan_board_scope exactly once, then retain the accepted active segment with write_flowscript. For a focused existing-board change, omit source and pass edits: [{old_text, new_text}] with exact unique matches from the embedded board; the host retains the complete merged source. Use full source for new boards or broad rewrites. After a plan is accepted, do not call plan_board_scope again unless its tool result explicitly authorizes one revision. Do not enumerate every utility or chase omitted queries before that checkpoint. Repair the retained source with patch_flowscript and use structured compiler diagnostics for focused declaration follow-ups; once a write or patch returns zero diagnostics, use test_flowscript for an eligible isolated data transform with an exact revision, entry, payload and expected output. A failing test supplies evidence for a focused patch and retest. Unsupported boards remain untested; preserve the requested behavior. Then finish with commit_flowscript at the retained revision; commit validates inline and returns validation_errors on failure. check_flowscript is only the staged-plan growth gate or a re-validation after catalog drift or a host-applied segment. Before the first write, use at most six ancillary database/UI/storage inspections. Preserve every requested capability, helper, Event, and kept //@n anchor across repairs; never replace a failed production draft with a smoke test or empty Event. Cross-domain context tools are read-only: database, storage, and UI inspection. Never emit UI, mutate app data/storage directly, use public-web/ask-user tools, or use Read/shell/filesystem tools for FlowPilot artifacts. After commit_flowscript returns queued/already_queued, stop workflow tools and hand any requested UI work back to the parent for the UI specialist. Cron/schedules are app Event setup on an eventsSimple() entry, never catalog nodes.";
    }
    match (has_board, has_ui, has_data) {
        (false, true, false) => {
            "You are the FlowPilot UI specialist. Use emit_ui/get_component_schema for A2UI pages, widgets, and components. Never author FlowScript, board nodes/connections/Events, or database/storage changes. For runtime VERIFICATION of persisted work you may drive the live page with interact_app_page (set inputs, trigger buttons, read runs + screenshots), run persisted Events with execute_event, message the app's chat with call_app_chat, and read logs with query_execution_logs — never to author data. Hand workflow wiring back to the board specialist. Do not use shell or file-edit tools for FlowPilot artifacts."
        }
        (true, false, false) => {
            "You are the read-only FlowPilot BOARD specialist. Inspect the current board and its read-only context, then answer. Never edit FlowScript, execute workflows, emit UI, or mutate app data/storage. Do not use shell or file-edit tools for FlowPilot artifacts."
        }
        (false, false, true) => {
            "You are the FlowPilot DATA STUDIO specialist. Use only the provided database, graph, analytics, and ontology tools. Never author FlowScript or emit UI. Do not use shell or file-edit tools for FlowPilot artifacts."
        }
        (true, true, false) => {
            "This is an explicit combined root FlowPilot surface, not a specialist. Keep UI changes in emit_ui and board behavior in the FlowScript lifecycle; never use one role's tools to perform the other's work."
        }
        _ => {
            "Use only the reviewed FlowPilot tools exposed by this role-scoped server. Do not use shell or file-edit tools for FlowPilot artifacts."
        }
    }
}

impl rmcp::ServerHandler for FlowPilotMcpServer {
    fn on_initialized(
        &self,
        _context: rmcp::service::NotificationContext<rmcp::RoleServer>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.dispatch_observer
            .record("mcp", "initialized", None, None);
        tracing::info!("client initialized");
        std::future::ready(())
    }

    fn get_info(&self) -> rmcp::model::ServerInfo {
        let instructions = flowpilot_mcp_server_instructions(
            self.tools.keys().map(String::as_str),
            self.workflow_state.is_some(),
        );
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(instructions)
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        self.dispatch_observer
            .record("mcp", "list_tools", None, None);
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
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        let tool_name = request.name.to_string();
        let tool = self.tools.get(tool_name.as_str()).cloned();
        let telemetry_tool = tool
            .as_ref()
            .map(|tool| tool.definition.name.clone())
            .unwrap_or_else(|| "unknown_tool".to_string());
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());

        async move {
            if self.phase_closed.load(Ordering::Acquire) {
                return Err(rmcp::ErrorData::invalid_request(
                    "This provider phase has ended; no new tools may run.",
                    None,
                ));
            }
            self.dispatch_observer
                .record(&telemetry_tool, "arrival", None, None);
            if let Ok(mut activity) = self.tool_activity.lock() {
                activity.total_tool_calls = activity.total_tool_calls.saturating_add(1);
            }
            let workflow_operation_guard = if self.workflow_state.is_some()
                && is_order_sensitive_workflow_tool(&tool_name)
            {
                match self.workflow_operation_gate.clone().try_lock_owned() {
                    Ok(guard) => Some(guard),
                    Err(_) => {
                        self.dispatch_observer.record(
                            &telemetry_tool,
                            "preflight_short_circuit",
                            None,
                            Some("edit_in_flight"),
                        );
                        return Ok(workflow_loop_result(
                            serde_json::json!({
                                "status": "edit_in_flight",
                                "next_action": "wait",
                                "message": "Another order-sensitive workflow operation is still running. Wait for its retained revision/status before issuing the next mutation."
                            }),
                            true,
                        ));
                    }
                }
            } else {
                None
            };
            // Register before workflow preflight so a phase boundary cannot observe quiescence in
            // the small window between `edit_in_flight = true` and spawning its blocking handler.
            // A global specialist belongs to the chat run, including when code mode drops its
            // HTTP waiter while producing a final answer. Ordinary calls remain request-owned.
            let mut delegated_call = self
                .delegated_phase
                .as_ref()
                .filter(|_| tool.is_some() && is_owned_delegation(&tool_name, &args))
                .map(|phase| phase.begin(&tool_name, &args))
                .transpose()
                .map_err(|message| rmcp::ErrorData::invalid_request(message, None))?;
            let delegated_id = delegated_call.as_ref().map(|call| call.id);
            let delegated_deadline = delegated_call.as_ref().map(|call| call.deadline);
            let cancellation = delegated_call
                .as_ref()
                .map(|call| call.cancellation.clone())
                .unwrap_or_else(|| context.ct.child_token());
            let handler_cancellation = cancellation.clone();
            let active_handler = register_mcp_active_handler(
                &self.tool_activity,
                &self.handler_quiescence,
                cancellation.clone(),
            )
            .map_err(|message| {
                self.dispatch_observer.record(
                    &telemetry_tool,
                    "preflight_short_circuit",
                    Some("handler_registry_unavailable"),
                    Some("internal_state_unavailable"),
                );
                rmcp::ErrorData::internal_error(message, None)
            })?;
            let mut cancellation_guard = delegated_call
                .is_none()
                .then(|| McpToolCancellationGuard::new(cancellation.clone()));
            if self.phase_closed.load(Ordering::Acquire) {
                cancellation.cancel();
                return Err(rmcp::ErrorData::invalid_request(
                    "This provider phase has ended; no new tools may run.",
                    None,
                ));
            }

            let mut context_preflight = ExternalContextPreflight::default();
            if let Some(state) = &self.workflow_state {
                if let Some(result) = workflow_database_setup_preflight(state, &tool_name, &args) {
                    self.dispatch_observer
                        .short_circuit_mcp(&telemetry_tool, &result);
                    return Ok(result);
                }
                context_preflight =
                    workflow_predraft_context_preflight_with_lease(state, &tool_name, &args);
                if let Some(result) = context_preflight
                    .result
                    .take()
                    .or_else(|| workflow_tool_preflight_with_args(state, &tool_name, &args))
                    .or_else(|| workflow_candidate_preflight(state, &tool_name, &args))
                {
                    if context_preflight.lease.is_some() {
                        workflow_tool_abort_with_args(
                            state,
                            context_preflight.lease.as_ref(),
                            &tool_name,
                            &args,
                            "A later host preflight short-circuited the reserved context read",
                        );
                    }
                    self.dispatch_observer
                        .short_circuit_mcp(&telemetry_tool, &result);
                    return Ok(result);
                }
            }

            let Some(tool) = tool else {
                self.dispatch_observer.record(
                    &telemetry_tool,
                    "preflight_short_circuit",
                    Some("unknown_tool"),
                    Some("invalid_params"),
                );
                if let Some(state) = &self.workflow_state {
                    workflow_tool_abort_with_args(
                        state,
                        context_preflight.lease.as_ref(),
                        &tool_name,
                        &args,
                        "Unknown FlowPilot tool after workflow preflight",
                    );
                }
                return Err(rmcp::ErrorData::invalid_params(
                    format!("Unknown FlowPilot tool: {tool_name}"),
                    None,
                ));
            };

            let _progress_heartbeat =
                McpProgressHeartbeat::start(&context, cancellation.clone(), &tool_name);
            let definition_name = tool.definition.name.clone();
            let handler = tool.handler.clone();
            let recorded_args = args.clone();
            let abort_args = args.clone();
            let workflow_lease = context_preflight.lease;
            let abort_lease = workflow_lease.clone();
            let recorded_tool_name = tool_name.clone();
            let workflow_state = self.workflow_state.clone();
            let tool_activity = self.tool_activity.clone();
            let dispatch_observer = self.dispatch_observer.clone();
            flowpilot_debug_trace!(tool = %definition_name, "FlowPilot MCP tool call started");

            // Ordinary calls inherit protocol cancellation and HTTP future drops. Delegated
            // workers keep the root run's token so a lost provider waiter cannot stop them.
            let task_result = tokio::task::spawn_blocking(move || {
                let _workflow_operation_guard = workflow_operation_guard;
                let _active_handler = active_handler;
                let mut result =
                    crate::functions::ai::frontend_tool_bridge::with_frontend_tool_execution_scope(
                        handler_cancellation,
                        delegated_deadline,
                        || {
                            dispatch_observer.record(&telemetry_tool, "dispatched", None, None);
                            (handler)(&definition_name, &args)
                        },
                    );

                // Record and annotate on the blocking worker itself. If the MCP HTTP future is
                // dropped, its JoinHandle is detached; doing this only after `.await` left
                // `edit_in_flight` stuck and let a late result overwrite the next repair phase.
                if let Some(state) = &workflow_state {
                    let succeeded = result.result_type != "error"
                        && result.error.is_none()
                        && workflow_tool_result_succeeded(&result.text_result_for_llm);
                    workflow_tool_record_with_outcome(
                        state,
                        workflow_lease.as_ref(),
                        &recorded_tool_name,
                        &recorded_args,
                        &result.text_result_for_llm,
                        succeeded,
                    );
                    annotate_modular_fallback_result(state, &recorded_tool_name, &mut result);
                    suppress_unchanged_flowscript_source_echo(
                        &recorded_tool_name,
                        &recorded_args,
                        &mut result,
                    );
                }

                record_delegated_run_tool_progress(
                    &recorded_tool_name,
                    mcp_total_tool_calls(&tool_activity),
                    workflow_state.as_ref(),
                );

                record_recoverable_platform_mutation(&tool_activity, &recorded_tool_name, &result);

                if let Some(call) = delegated_call.as_mut() {
                    call.complete(&result);
                }

                result
            })
            .await;
            // Once the synchronous handler has settled there is no orphan left to cancel. If this
            // request future is dropped while awaiting the JoinHandle, Drop keeps the guard armed.
            if let Some(guard) = cancellation_guard.as_mut() {
                guard.disarm();
            }
            if let Some(id) = delegated_id
                && let Some(phase) = self.delegated_phase.as_ref()
            {
                phase.delivered(id);
            }
            let result = match task_result {
                Ok(result) => result,
                Err(error) => {
                    let message = format!("FlowPilot MCP tool task failed: {error}");
                    if let Some(state) = &self.workflow_state {
                        workflow_tool_abort_with_args(
                            state,
                            abort_lease.as_ref(),
                            &tool_name,
                            &abort_args,
                            &message,
                        );
                    }
                    return Err(rmcp::ErrorData::internal_error(message, None));
                }
            };

            if result.result_type == "error" || result.error.is_some() {
                tracing::warn!(
                    tool = %tool_name,
                    error = ?result.error,
                    "FlowPilot MCP tool call returned an error"
                );
            } else {
                flowpilot_debug_trace!(tool = %tool_name, "FlowPilot MCP tool call completed");
            }

            Ok(flowpilot_tool_result_to_mcp(result))
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct McpToolCompletion {
    pub(super) tool_name: String,
    pub(super) result_text: String,
}

#[derive(Debug, Default)]
pub(super) struct McpToolActivityState {
    pub(super) last_successful_mutation: Option<McpToolCompletion>,
    /// Total tool-call arrivals across every provider phase of this run. A phase whose delta is
    /// zero proves the CLI failed before doing any work, so its restart is accounted separately
    /// from the bounded workflow continuations.
    pub(super) total_tool_calls: u64,
    pub(super) next_handler_id: u64,
    pub(super) active_handlers: HashMap<u64, CancellationToken>,
}

pub(super) fn mcp_total_tool_calls(activity: &Arc<StdMutex<McpToolActivityState>>) -> u64 {
    activity
        .lock()
        .map(|activity| activity.total_tool_calls)
        .unwrap_or_default()
}

/// Membership guard for synchronous MCP handlers that may outlive their HTTP request future.
/// `spawn_blocking` cannot be force-aborted, so a provider phase is not allowed to hand off to a
/// repair process until every registered handler has observed cancellation and left this set.
pub(super) struct McpActiveHandlerGuard {
    id: u64,
    activity: Arc<StdMutex<McpToolActivityState>>,
    quiescence: Arc<tokio::sync::Notify>,
}

impl Drop for McpActiveHandlerGuard {
    fn drop(&mut self) {
        if let Ok(mut activity) = self.activity.lock() {
            activity.active_handlers.remove(&self.id);
        }
        self.quiescence.notify_waiters();
    }
}

pub(super) fn register_mcp_active_handler(
    activity: &Arc<StdMutex<McpToolActivityState>>,
    quiescence: &Arc<tokio::sync::Notify>,
    cancellation: CancellationToken,
) -> Result<McpActiveHandlerGuard, String> {
    let id = {
        let mut activity = activity
            .lock()
            .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?;
        activity.next_handler_id = activity.next_handler_id.wrapping_add(1).max(1);
        let id = activity.next_handler_id;
        activity.active_handlers.insert(id, cancellation);
        id
    };
    Ok(McpActiveHandlerGuard {
        id,
        activity: activity.clone(),
        quiescence: quiescence.clone(),
    })
}

pub(super) fn is_recoverable_platform_mutation(tool_name: &str) -> bool {
    use flow_like::flow::copilot::tool_spec::{ToolApprovalSpec, find_global_tool_spec};

    find_global_tool_spec(tool_name)
        .or_else(|| find_home_tool_spec(tool_name))
        .is_some_and(|spec| !matches!(spec.approval, ToolApprovalSpec::None))
}

pub(super) fn record_recoverable_platform_mutation(
    tool_activity: &Arc<StdMutex<McpToolActivityState>>,
    tool_name: &str,
    result: &copilot_sdk::ToolResultObject,
) {
    if is_recoverable_platform_mutation(tool_name)
        && !flowpilot_tool_result_is_error(result)
        && let Ok(mut activity) = tool_activity.lock()
    {
        activity.last_successful_mutation = Some(McpToolCompletion {
            tool_name: tool_name.to_string(),
            result_text: result.text_result_for_llm.clone(),
        });
    }
}

pub(super) fn flowpilot_tool_result_is_error(result: &copilot_sdk::ToolResultObject) -> bool {
    let semantic_error = serde_json::from_str::<serde_json::Value>(&result.text_result_for_llm)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|status| workflow_status_requires_repair(&status));

    // Frontend approval denials use a successful SDK text envelope. Reuse the shared semantic
    // result check so a denied draft can never become provider-exit recovery evidence.
    result.result_type == "error"
        || result.error.is_some()
        || !workflow_tool_result_succeeded(&result.text_result_for_llm)
        || semantic_error
}

pub(super) fn flowpilot_tool_result_to_mcp(
    result: copilot_sdk::ToolResultObject,
) -> rmcp::model::CallToolResult {
    if flowpilot_tool_result_is_error(&result) {
        rmcp::model::CallToolResult::error(vec![rmcp::model::Content::text(
            result
                .error
                .unwrap_or_else(|| result.text_result_for_llm.clone()),
        )])
    } else {
        let mut contents = vec![rmcp::model::Content::text(result.text_result_for_llm)];
        if let Some(images) = result.binary_results_for_llm {
            contents.extend(images.into_iter().filter_map(|image| {
                image
                    .mime_type
                    .starts_with("image/")
                    .then(|| rmcp::model::Content::image(image.data, image.mime_type))
            }));
        }
        rmcp::model::CallToolResult::success(contents)
    }
}

pub(super) struct FlowPilotMcpBridge {
    pub(super) url: String,
    pub(super) cancellation_token:
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig,
    pub(super) server_task: Option<tokio::task::JoinHandle<()>>,
    pub(super) workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
    pub(super) tool_activity: Arc<StdMutex<McpToolActivityState>>,
    pub(super) handler_quiescence: Arc<tokio::sync::Notify>,
    phase_closed: Arc<AtomicBool>,
    delegated_phase: Option<Arc<DelegatedPhase>>,
}

const FLOWPILOT_MCP_SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);

pub(super) fn flowpilot_mcp_server_config()
-> rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
    use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;

    let mut config = StreamableHttpServerConfig::default();
    config.stateful_mode = true;
    // Keep long-running POST/SSE tool calls active through proxies and Claude Code's HTTP client.
    // Setting this to `None` made a quiet flowpilot_board request look dead and its transport was
    // dropped while the frontend/nested agent continued mutating in the background.
    config.sse_keep_alive = Some(FLOWPILOT_MCP_SSE_KEEP_ALIVE);
    config
}

impl FlowPilotMcpBridge {
    pub(super) async fn start(
        tools: Vec<(copilot_sdk::Tool, copilot_sdk::ToolHandler)>,
        workflow_state: Option<Arc<StdMutex<WorkflowToolLoopState>>>,
        tool_activity: Arc<StdMutex<McpToolActivityState>>,
        delegation_owner: Option<CancellationToken>,
    ) -> Result<Self, String> {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpService, session::local::LocalSessionManager,
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

        let config = flowpilot_mcp_server_config();
        let cancellation_token = config.clone();
        let service_tools = tools.clone();
        let service_workflow_state = workflow_state.clone();
        let service_tool_activity = tool_activity.clone();
        let handler_quiescence = Arc::new(tokio::sync::Notify::new());
        let service_handler_quiescence = handler_quiescence.clone();
        let workflow_operation_gate = Arc::new(tokio::sync::Mutex::new(()));
        let service_workflow_operation_gate = workflow_operation_gate.clone();
        let phase_closed = Arc::new(AtomicBool::new(false));
        let service_phase_closed = phase_closed.clone();
        let delegated_phase = delegation_owner.map(DelegatedPhase::new);
        let service_delegated_phase = delegated_phase.clone();
        let service: StreamableHttpService<FlowPilotMcpServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || {
                    Ok(FlowPilotMcpServer::new(
                        service_tools.clone(),
                        service_workflow_state.clone(),
                        service_tool_activity.clone(),
                        service_handler_quiescence.clone(),
                        service_workflow_operation_gate.clone(),
                        service_phase_closed.clone(),
                        service_delegated_phase.clone(),
                    ))
                },
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
            server_task: Some(server_task),
            workflow_state,
            tool_activity,
            handler_quiescence,
            phase_closed,
            delegated_phase,
        })
    }

    pub(super) async fn drain_delegated_tools(
        &self,
    ) -> Result<Vec<serde_json::Value>, DelegatedDrainError> {
        self.phase_closed.store(true, Ordering::Release);
        match self.delegated_phase.as_ref() {
            Some(phase) => phase.drain().await,
            None => Ok(Vec::new()),
        }
    }

    pub(super) async fn finish_provider_phase(
        self,
        provider_succeeded: bool,
    ) -> Result<FlowPilotMcpPhaseOutcome, String> {
        let delegated_phase = self.delegated_phase.clone();
        let completions = if provider_succeeded {
            self.drain_delegated_tools().await
        } else {
            Ok(Vec::new())
        };
        // Quiesce even when a completion deadline or cancellation failed the drain. A new
        // provider must never overlap the previous phase's remaining workers.
        let mut outcome = self.finish_phase().await?;
        outcome.delegated_results = match completions {
            Ok(results) => results,
            Err(DelegatedDrainError::DeadlineExceeded) => delegated_phase
                .ok_or_else(|| "Missing delegated phase after its deadline".to_string())?
                .collect_after_deadline()
                .map_err(|error| error.to_string())?,
            Err(error) => return Err(error.to_string()),
        };
        Ok(outcome)
    }

    pub(super) fn cancel_active_handlers(&self) -> Result<(), String> {
        let cancellations = self
            .tool_activity
            .lock()
            .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?
            .active_handlers
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for cancellation in cancellations {
            cancellation.cancel();
        }
        Ok(())
    }

    pub(super) async fn wait_for_handler_quiescence(&self) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT;
        loop {
            // Register the notification future before inspecting the count so a handler cannot
            // leave between the check and the await and strand us until the timeout.
            let notified = self.handler_quiescence.notified();
            let active = self
                .tool_activity
                .lock()
                .map_err(|_| "FlowPilot MCP handler registry is unavailable".to_string())?
                .active_handlers
                .len();
            if active == 0 {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(format!(
                    "FlowPilot cancelled a provider phase, but {active} synchronous MCP handler(s) did not quiesce within {} seconds. The repair continuation was stopped to prevent stale commands from overlapping a newer phase.",
                    EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT.as_secs()
                ));
            }
        }
    }

    /// Close one phase-local MCP server and wait for every detached-capable handler before the
    /// next repair process is allowed to start. Each provider phase gets a fresh URL, so a late
    /// HTTP request from the old CLI cannot be mistaken for work belonging to the new phase.
    pub(super) async fn finish_phase(mut self) -> Result<FlowPilotMcpPhaseOutcome, String> {
        self.phase_closed.store(true, Ordering::Release);
        if let Some(phase) = self.delegated_phase.as_ref() {
            phase.cancel();
        }
        self.cancellation_token.cancellation_token.cancel();
        let cancellation_result = self.cancel_active_handlers();
        let quiescence_result = self.wait_for_handler_quiescence().await;

        if let Some(mut server_task) = self.server_task.take()
            && tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, &mut server_task)
                .await
                .is_err()
        {
            server_task.abort();
            let _ = server_task.await;
            flowpilot_debug_log!(
                "[flowpilot-mcp] graceful shutdown exceeded {:?}; server task aborted",
                EXTERNAL_AGENT_SHUTDOWN_TIMEOUT
            );
        }

        cancellation_result?;
        quiescence_result?;

        let workflow_snapshot = self.workflow_state.as_ref().and_then(|state| {
            state.lock().ok().map(|mut state| {
                // A handler that panicked or lost its HTTP future before recording a result can
                // still leave the logical owner set. Quiescence proves no old worker can race this
                // repair-state transition now.
                state.finish_interrupted_phase();
                state.snapshot()
            })
        });
        let last_successful_mutation = self
            .tool_activity
            .lock()
            .ok()
            .and_then(|activity| activity.last_successful_mutation.clone());
        Ok(FlowPilotMcpPhaseOutcome {
            workflow_snapshot,
            last_successful_mutation,
            delegated_results: Vec::new(),
        })
    }
}

pub(super) struct FlowPilotMcpPhaseOutcome {
    pub(super) workflow_snapshot: Option<WorkflowToolLoopSnapshot>,
    pub(super) last_successful_mutation: Option<McpToolCompletion>,
    pub(super) delegated_results: Vec<serde_json::Value>,
}

impl Drop for FlowPilotMcpBridge {
    fn drop(&mut self) {
        // `external_code_agent_chat_internal` can itself be cancelled by Tauri/the caller. Do not
        // leave the session-local listener alive merely because the async shutdown path was skipped.
        self.cancellation_token.cancellation_token.cancel();
        self.phase_closed.store(true, Ordering::Release);
        if let Some(phase) = self.delegated_phase.as_ref() {
            phase.cancel();
        }
        let _ = self.cancel_active_handlers();
        if let Some(server_task) = self.server_task.take() {
            server_task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FlowPilotMcpServer, HOME_TOOL_MAX_RESULT_SIZE_CHARS, MAX_RESULT_SIZE_META_KEY};

    use super::{FlowPilotMcpBridge, McpToolActivityState};
    use flow_like_types::tokio_util::sync::CancellationToken;
    use rmcp::{
        ServerHandler, ServiceExt,
        model::{CallToolRequestParams, ClientRequest, Request},
        service::PeerRequestOptions,
        transport::StreamableHttpClientTransport,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    fn pending_tool_handler() -> (
        copilot_sdk::ToolHandler,
        tokio::sync::oneshot::Receiver<CancellationToken>,
        Arc<tokio::sync::Notify>,
    ) {
        let (started, receiver) = tokio::sync::oneshot::channel();
        let started = Arc::new(Mutex::new(Some(started)));
        let release = Arc::new(tokio::sync::Notify::new());
        let handler_release = release.clone();
        let handler: copilot_sdk::ToolHandler = Arc::new(move |_, _| {
            let (cancellation, _) =
                crate::functions::ai::frontend_tool_bridge::current_tool_execution_for_test()
                    .unwrap();
            if let Some(started) = started.lock().unwrap().take() {
                let _ = started.send(cancellation.clone());
            }
            let completed = tokio::runtime::Handle::current().block_on(async {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => false,
                    _ = handler_release.notified() => true,
                }
            });
            copilot_sdk::ToolResultObject::text(if completed {
                r#"{"status":"applied","board_id":"test-board","command_count":6228}"#
            } else {
                r#"{"status":"cancelled"}"#
            })
        });
        (handler, receiver, release)
    }

    async fn pending_tool_bridge(
        name: &'static str,
        owner: CancellationToken,
    ) -> (
        FlowPilotMcpBridge,
        tokio::sync::oneshot::Receiver<CancellationToken>,
        Arc<tokio::sync::Notify>,
    ) {
        let (handler, receiver, release) = pending_tool_handler();
        let bridge = FlowPilotMcpBridge::start(
            vec![(copilot_sdk::Tool::new(name), handler)],
            None,
            Arc::new(Mutex::new(McpToolActivityState::default())),
            Some(owner),
        )
        .await
        .unwrap();
        (bridge, receiver, release)
    }

    #[tokio::test]
    async fn dropping_server_request_future_preserves_only_owned_delegations() {
        struct PeerServer;
        impl ServerHandler for PeerServer {}

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_service =
            tokio::spawn(async { PeerServer.serve(server_transport).await.unwrap() });
        let client = ().serve(client_transport).await.unwrap();
        let server_service = server_service.await.unwrap();

        for (name, owned) in [("flowpilot_board", true), ("execute_event", false)] {
            let (handler, started, release) = pending_tool_handler();
            let phase = super::DelegatedPhase::new(CancellationToken::new());
            let activity = Arc::new(Mutex::new(McpToolActivityState::default()));
            let server = FlowPilotMcpServer::new(
                Arc::new(std::collections::HashMap::from([(
                    name.to_owned(),
                    super::FlowPilotMcpTool {
                        definition: copilot_sdk::Tool::new(name),
                        handler,
                    },
                )])),
                None,
                activity.clone(),
                Arc::new(tokio::sync::Notify::new()),
                Arc::new(tokio::sync::Mutex::new(())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                Some(phase.clone()),
            );
            let context = rmcp::service::RequestContext::new(
                rmcp::model::NumberOrString::Number(1),
                server_service.peer().clone(),
            );
            let request = tokio::spawn(async move {
                server
                    .call_tool(CallToolRequestParams::new(name), context)
                    .await
            });
            let cancellation = tokio::time::timeout(Duration::from_secs(3), started)
                .await
                .unwrap()
                .unwrap();
            request.abort();
            let _ = request.await;
            assert_eq!(cancellation.is_cancelled(), !owned);
            release.notify_one();
            let results = tokio::time::timeout(Duration::from_secs(3), phase.drain())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(results.len(), usize::from(owned));
            tokio::time::timeout(Duration::from_secs(3), async {
                while !activity.lock().unwrap().active_handlers.is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        let _ = client.cancel().await;
        let _ = server_service.cancel().await;
    }

    #[tokio::test]
    async fn successful_parent_exit_drains_a_dropped_delegation_and_fences_late_calls() {
        let (bridge, started, release) =
            pending_tool_bridge("flowpilot_board", CancellationToken::new()).await;
        let url = bridge.url.clone();
        let client = ().serve(StreamableHttpClientTransport::from_uri(url.clone())).await.unwrap();
        let peer = client.peer().clone();
        let request = tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new("flowpilot_board"))
                .await
        });
        let child_cancel = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        request.abort();
        let _ = request.await;
        let _ = client.cancel().await;
        assert!(
            !child_cancel.is_cancelled(),
            "dropping the provider HTTP waiter must not cancel run-owned work"
        );
        let finish = tokio::spawn(async move { bridge.finish_provider_phase(true).await });
        tokio::task::yield_now().await;
        assert!(
            !finish.is_finished(),
            "the root must wait for the specialist before continuing"
        );
        let late = ().serve(StreamableHttpClientTransport::from_uri(url)).await.unwrap();
        assert!(
            late.call_tool(CallToolRequestParams::new("flowpilot_board"))
                .await
                .is_err(),
            "the old phase must not admit new work while draining"
        );
        let _ = late.cancel().await;
        release.notify_one();
        let outcome = tokio::time::timeout(Duration::from_secs(10), finish)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let results = outcome.delegated_results;
        assert_eq!(results[0]["result"]["output"]["command_count"], 6228);
        let continuation =
            super::super::delegated_completion::delegated_completion_prompt(&results);
        assert!(continuation.contains("6228"));
        assert!(continuation.contains("Do not repeat a completed mutation"));
    }

    #[tokio::test]
    async fn provider_failure_cancels_delegation_before_the_next_phase() {
        let (bridge, started, _release) =
            pending_tool_bridge("flowpilot_board", CancellationToken::new()).await;
        let client =
            ().serve(StreamableHttpClientTransport::from_uri(bridge.url.clone()))
                .await
                .unwrap();
        let peer = client.peer().clone();
        let request = tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new("flowpilot_board"))
                .await
        });
        let child_cancel = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        let outcome =
            tokio::time::timeout(Duration::from_secs(10), bridge.finish_provider_phase(false))
                .await
                .unwrap()
                .unwrap();
        assert!(child_cancel.is_cancelled());
        assert!(outcome.delegated_results.is_empty());
        request.abort();
        let _ = request.await;
        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn deadline_preserves_queued_review_and_quiesces_pending_sibling() {
        let (handler, started, _release) = pending_tool_handler();
        let completed: copilot_sdk::ToolHandler = Arc::new(|_, _| {
            copilot_sdk::ToolResultObject::text(r#"{"status":"queued","job_id":"retained-review"}"#)
        });
        let activity = Arc::new(Mutex::new(McpToolActivityState::default()));
        let bridge = FlowPilotMcpBridge::start(
            vec![
                (copilot_sdk::Tool::new("flowpilot_board"), completed),
                (copilot_sdk::Tool::new("data_studio_agent"), handler),
            ],
            None,
            activity.clone(),
            Some(CancellationToken::new()),
        )
        .await
        .unwrap();
        let client =
            ().serve(StreamableHttpClientTransport::from_uri(bridge.url.clone()))
                .await
                .unwrap();
        client
            .call_tool(CallToolRequestParams::new("flowpilot_board"))
            .await
            .unwrap();
        let peer = client.peer().clone();
        let request = tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new("data_studio_agent"))
                .await
        });
        let cancellation = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        bridge
            .delegated_phase
            .as_ref()
            .unwrap()
            .expire_pending_calls_for_test();
        let outcome =
            tokio::time::timeout(Duration::from_secs(10), bridge.finish_provider_phase(true))
                .await
                .unwrap()
                .unwrap();
        assert!(cancellation.is_cancelled());
        assert!(activity.lock().unwrap().active_handlers.is_empty());
        let results = outcome.delegated_results;
        assert_eq!(results[0]["result"]["output"]["job_id"], "retained-review");
        assert!(results[0].get("host_interruption").is_none());
        assert_eq!(results[1]["result"]["output"]["status"], "cancelled");
        assert_eq!(
            results[1]["host_interruption"]["own_deadline_expired"],
            true
        );
        assert_eq!(results[2]["status"], "timeout");
        assert_eq!(results[2]["handlers_quiesced"], true);
        let prompt = super::super::delegated_completion::delegated_completion_prompt(&results);
        assert!(prompt.contains("retained-review"));
        assert!(prompt.contains("timeout"));
        request.abort();
        let _ = request.await;
        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn user_stop_during_deadline_quiescence_prevents_completion_resume() {
        let owner = CancellationToken::new();
        let (handler, started, _release) = pending_tool_handler();
        let stop_during_cleanup = owner.clone();
        let handler: copilot_sdk::ToolHandler = Arc::new(move |name, args| {
            let result = handler(name, args);
            stop_during_cleanup.cancel();
            result
        });
        let bridge = FlowPilotMcpBridge::start(
            vec![(copilot_sdk::Tool::new("flowpilot_board"), handler)],
            None,
            Arc::new(Mutex::new(McpToolActivityState::default())),
            Some(owner),
        )
        .await
        .unwrap();
        let client =
            ().serve(StreamableHttpClientTransport::from_uri(bridge.url.clone()))
                .await
                .unwrap();
        let peer = client.peer().clone();
        let request = tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new("flowpilot_board"))
                .await
        });
        let _ = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        bridge
            .delegated_phase
            .as_ref()
            .unwrap()
            .expire_pending_calls_for_test();
        let error =
            tokio::time::timeout(Duration::from_secs(10), bridge.finish_provider_phase(true))
                .await
                .unwrap()
                .err()
                .expect("user cancellation must prevent resume");
        assert!(error.contains("cancelled"));
        request.abort();
        let _ = request.await;
        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn explicit_run_cancellation_interrupts_a_successful_parent_drain() {
        let owner = CancellationToken::new();
        let (bridge, started, _release) =
            pending_tool_bridge("data_studio_agent", owner.clone()).await;
        let client =
            ().serve(StreamableHttpClientTransport::from_uri(bridge.url.clone()))
                .await
                .unwrap();
        let peer = client.peer().clone();
        let request = tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new("data_studio_agent"))
                .await
        });
        let child_cancel = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        let finish = tokio::spawn(async move { bridge.finish_provider_phase(true).await });
        tokio::task::yield_now().await;
        owner.cancel();
        let error = tokio::time::timeout(Duration::from_secs(10), finish)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("explicit cancellation must fail the completion check");
        assert!(error.contains("cancelled"));
        assert!(child_cancel.is_cancelled());
        request.abort();
        let _ = request.await;
        let _ = client.cancel().await;
    }

    #[tokio::test]
    async fn cancelled_non_delegated_request_still_cancels_its_handler() {
        let (bridge, started, _release) =
            pending_tool_bridge("execute_event", CancellationToken::new()).await;
        let client =
            ().serve(StreamableHttpClientTransport::from_uri(bridge.url.clone()))
                .await
                .unwrap();
        let request = client
            .send_cancellable_request(
                ClientRequest::CallToolRequest(Request::new(CallToolRequestParams::new(
                    "execute_event",
                ))),
                PeerRequestOptions::no_options(),
            )
            .await
            .unwrap();
        let cancellation = tokio::time::timeout(Duration::from_secs(3), started)
            .await
            .unwrap()
            .unwrap();
        request
            .cancel(Some("test request cancellation".into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), cancellation.cancelled())
            .await
            .unwrap();
        let _ = client.cancel().await;
        assert!(bridge.drain_delegated_tools().await.unwrap().is_empty());
        bridge.finish_phase().await.unwrap();
    }

    fn advertised_max_result_size(name: &str) -> Option<u64> {
        let tool = FlowPilotMcpServer::to_mcp_tool(&copilot_sdk::Tool::new(name));
        serde_json::to_value(&tool)
            .expect("serialize MCP tool")
            .get("_meta")
            .and_then(|meta| meta.get(MAX_RESULT_SIZE_META_KEY))
            .and_then(serde_json::Value::as_u64)
    }

    #[test]
    fn home_tools_advertise_the_claude_code_result_size_ceiling() {
        for name in [
            "get_home_context",
            "validate_home_layout",
            "apply_home_layout",
        ] {
            assert_eq!(
                advertised_max_result_size(name),
                Some(HOME_TOOL_MAX_RESULT_SIZE_CHARS),
                "{name} must declare {MAX_RESULT_SIZE_META_KEY}"
            );
        }
    }

    #[test]
    fn non_home_tools_keep_the_default_result_size() {
        let tool = FlowPilotMcpServer::to_mcp_tool(&copilot_sdk::Tool::new("emit_ui"));
        assert!(tool.meta.is_none());
        assert_eq!(advertised_max_result_size("emit_ui"), None);
    }
}
