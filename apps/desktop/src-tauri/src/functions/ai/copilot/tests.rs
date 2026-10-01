use super::agent_surface::{
    append_flowscript_recovery_context, append_typed_ir_recovery_context,
    pending_flowscript_redelivery_response,
};
use super::backend_commands::copilot_sdk_list_models;
use super::backend_types::{
    FlowPilotAgentBackendKind, FlowPilotAgentCapabilitySet, FlowPilotAgentTransportKind,
    FlowPilotChatBackend, FlowPilotModelSelection, ReasoningEffortOption,
};
use super::backends::FlowPilotBackendStartOptions;
use super::board_commits::{
    ApplyFlowIrCommitResult, FLOW_IR_APPLIED_RECEIPTS, board_edit_job_matches_terminal_delivery,
    compact_durable_apply_receipt, exact_board_command_batch_matches, flow_ir_ack_race_diagnostic,
    flow_ir_applied_receipt_key, replay_flow_ir_applied_receipt, retain_flow_ir_applied_receipt,
    typed_commit_destructive_review_items, validate_board_edit_delivery_bounds,
};
use super::board_jobs::{
    BOARD_EDIT_JOB_MAX_ENTRIES, BOARD_EDIT_JOB_MAX_REMOTE_COMMAND_BYTES,
    BOARD_EDIT_JOB_SCHEMA_VERSION, BOARD_EDIT_JOB_TTL, BoardEditJob, BoardEditJobDeliveryLease,
    BoardEditJobPhase, BoardEditJobRecord, BoardEditJobReview, PersistedBoardEditJobEntry,
    PersistedBoardEditJobRecord, another_board_edit_job_reserves_mutation, board_command_review,
    board_edit_job_record_from_persisted, board_mutation_is_reserved, flow_ir_commit_identity,
    prune_board_edit_jobs,
};
use super::chat::{home_profile_scope_error, resolve_copilot_app_id, specialist_host_context};
use super::cli_auth::{claude_auth_probe_from_success, external_agent_auth_output_is_signed_out};
use super::cli_resolution::{
    CliResolution, CliResolutionSource, augmented_path, claude_ide_extension_binaries,
    codex_binary_name, codex_ide_extension_candidate_dirs, codex_target, extra_bin_dirs,
    find_codex_packaged_cli_under_root, find_copilot_cli_path, find_executable_in_path,
};
use super::client_pool::{
    COPILOT_START_OPTIONS, NESTED_COPILOT_POOL, NESTED_COPILOT_POOL_SIZE, NESTED_COPILOT_RUN_GATES,
    NestedCopilotPool, acquire_nested_copilot_run_permit, checkout_nested_copilot_client,
    checkout_nested_copilot_client_from, nested_copilot_run_gate, nested_copilot_run_gate_key,
    nested_copilot_start_options, quarantine_nested_copilot_client,
};
use super::external_continuation::{
    build_external_agent_prompt, build_external_agent_prompt_body,
    build_external_workflow_continuation_prompt, external_agent_role_appendix,
    external_workflow_incomplete_error, nested_wall_clock_exhausted,
    nested_wall_clock_incomplete_error,
};
use super::external_invocation::ExternalAgentInvocation;
use super::external_stream::{
    ExternalAgentStreamState, claude_agent_message_delta, claude_agent_tool_events,
    codex_agent_message_delta, external_agent_error_text,
    external_agent_flowscript_workspace_event, external_agent_mcp_connect_failure,
    external_agent_process_event, external_agent_result_text, external_result_details,
    flowpilot_stream_tag,
};
use super::global_chat::{
    GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES, GLOBAL_CHAT_RUN_MAX_CHUNKS, GlobalChatRunBuffer,
};
use super::mcp::{
    McpToolActivityState, McpToolCompletion, flowpilot_mcp_server_config,
    flowpilot_mcp_server_instructions, flowpilot_tool_result_is_error,
    flowpilot_tool_result_to_mcp, is_recoverable_platform_mutation,
    record_recoverable_platform_mutation, register_mcp_active_handler,
};
use super::mcp_progress::{
    DELEGATED_RUN_PROGRESS_FRESHNESS, LATEST_DELEGATED_RUN_TOOL_PROGRESS, McpToolCancellationGuard,
    delegated_run_heartbeat_message, is_delegated_agent_tool, mcp_progress_heartbeat_notification,
    record_delegated_run_tool_progress,
};
use super::model_catalog::{
    codex_models_with_configured_default, parse_claude_model_catalog, parse_codex_model_catalog,
};
use super::platform_bridge::{
    FrontendPlatformToolSet, frontend_platform_tool_spec, global_orchestrator_tool_scope_error,
};
use super::provider_errors::{
    ExternalAgentExitKind, ExternalAgentFailureCategory, actionable_external_agent_failure,
    can_resume_external_workflow_after_failure, classify_external_agent_failure,
    classify_external_agent_user_failure,
};
use super::runtime::{
    ACTIVE_COPILOT_RUNS, MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL, SDK_CONTROL_RPC_TIMEOUT,
    SDK_EVENT_INACTIVITY_TIMEOUT, SdkToolActivityRegistry, cancel_copilot_chat,
    register_copilot_run, sdk_tool_handler_watchdog_timeout,
};
use super::stream_events::{
    append_bounded_text, correlate_stream_frame, correlated_stream_payload,
    direct_sdk_tool_result_stream_status, flowscript_response_workspace_envelope,
    flowscript_workspace_result_payload, preview_tool_arguments, preview_tool_result,
    render_recovered_mutation_message, request_identity_prompt_for,
};
use super::telemetry::{
    AGENT_ERROR_CLASSES, AGENT_STAGE_AUTH, AGENT_STAGE_MODELS, AGENT_STAGE_RUN, AGENT_STAGE_SPAWN,
    AGENT_STAGE_STOP, agent_backend_error_props, agent_backend_lifecycle_props, backend_label,
    classify_agent_error,
};
use super::tool_policy::{
    is_flowpilot_read_only_tool, is_read_only_workflow_request, is_workflow_edit_request,
    specialist_tool_policy,
};
use super::workflow_declarations::{
    declaration_queries_are_related, declaration_repair_query_is_bounded,
    declaration_repair_query_keys, diagnostic_declaration_repair_hints, retain_declaration_result,
};
use super::workflow_diagnostics::{
    workflow_result_diagnostics, workflow_result_requires_repair,
    workflow_result_structured_diagnostics,
};
use super::workflow_observation::{
    workflow_tool_abort, workflow_tool_abort_with_args, workflow_tool_record,
};
use super::workflow_preflight::{
    workflow_candidate_preflight, workflow_tool_preflight, workflow_tool_preflight_with_args,
};
use super::workflow_reporting::{
    collect_unimplemented_stubs, workflow_run_summary_payload, workflow_run_summary_scope_plan,
};
use super::workflow_results::{
    annotate_modular_fallback_result, flowscript_source_fingerprint,
    suppress_unchanged_flowscript_source_echo,
};
use super::workflow_sdk::{
    IdleContinuationBudget, InitialSourceCheckpointPhase, guard_sdk_workflow_tools,
    is_flowscript_draft_operation_tool, is_order_sensitive_workflow_tool, is_workflow_commit_tool,
    is_workflow_loop_tool, prepare_sdk_idle_continuation_budget, scope_sdk_tool_handlers,
    typed_ir_operation_budget, workflow_database_setup_preflight,
    workflow_initial_source_checkpoint_phase, workflow_predraft_context_preflight,
    workflow_predraft_context_preflight_with_lease, workflow_state_has_retained_candidate,
};
use super::workflow_state::{
    EXTERNAL_CONTINUATION_CHECK_HEADROOM, EXTERNAL_CONTINUATION_OPERATION_HEADROOM,
    EXTERNAL_EXTENSION_CHECK_GRANT, EXTERNAL_EXTENSION_COMMIT_GRANT,
    EXTERNAL_EXTENSION_CONTINUATION_GRANT, EXTERNAL_EXTENSION_OPERATION_GRANT,
    EXTERNAL_SEGMENT_CHECK_ALLOWANCE, EXTERNAL_SEGMENT_OPERATION_ALLOWANCE,
    EXTERNAL_SEGMENT_WALL_CLOCK_ALLOWANCE, EXTERNAL_TIME_EXTENSION_SLICE,
    MAX_EXTERNAL_EARNED_WALL_CLOCK, MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS,
    MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS, MAX_EXTERNAL_PREDRAFT_CONTEXT_READS,
    MAX_EXTERNAL_SEGMENTED_FLOWSCRIPT_OPERATION_ATTEMPTS, MAX_EXTERNAL_SEGMENTED_WALL_CLOCK_BUDGET,
    MAX_EXTERNAL_SEGMENTED_WORKFLOW_EDIT_ATTEMPTS, MAX_EXTERNAL_TYPED_IR_OPERATION_BUDGET,
    MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS, MAX_EXTERNAL_WORKFLOW_CONTINUATIONS,
    MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS, MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
    MAX_INITIAL_DECLARATION_ATTEMPTS, MAX_REPAIR_DECLARATION_ATTEMPTS_PER_KEY,
    MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES, MAX_RETAINED_STRUCTURED_DIAGNOSTICS,
    MIN_EXTERNAL_TYPED_IR_OPERATION_BUDGET, NESTED_RUN_WALL_CLOCK_BUDGET, TimeExtensionDecision,
    WorkflowMutationPath, WorkflowToolLoopSnapshot, WorkflowToolLoopState, submitted_flowscript,
};
use crate::functions::ai::{
    copilot_sdk_tools::retained_flow_ir_draft_store_for_board,
    frontend_tool_bridge::FrontendToolContext,
};
use copilot_sdk::{Client, LogLevel};
use flow_like::{
    app::App,
    copilot::{ChatImage, CopilotScope, FlowIrCommitToken},
    flow::{
        board::{Board, commands::GenericCommand},
        copilot::{
            BoardCommand, BoardContextManifest, BoardScopePlan,
            FORCED_INCREMENTAL_SEGMENT_THRESHOLD, FlowScriptPendingDelivery,
            MAX_BOARD_SCOPE_SEGMENTS, ManifestAudit, ManifestAugmentations, ManifestSource,
            PlanBoardScopeArgs, ScopeStrategy, accept_scope_plan,
            default_flowscript_module_templates, flowscript_workspace_envelope,
            profile_flowscript_candidate,
            tool_spec::{MAX_DELEGATED_RUN_DISPATCH_SECS, RESEARCH_AGENT_TOOL},
        },
        variable::VariableType,
    },
};
use flow_like_types::{channel::Channel as _, tokio_util::sync::CancellationToken};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

mod backend_telemetry;
mod board_edit_jobs;
mod candidate_regression;
mod cli_discovery;
mod declaration_preflight;
mod declaration_repair;
mod nested_runs;
mod provider_invocations;
mod provider_streaming;
mod request_recovery;
mod run_lifecycle;
mod scope_planning;
mod sdk_integration;
mod source_recovery;
mod tool_policy;
mod typed_ir;
mod workflow_preflight;
mod workflow_reporting;

fn sorted_prop_keys(props: &serde_json::Value) -> Vec<&str> {
    let mut keys: Vec<&str> = props
        .as_object()
        .expect("agent backend props are an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn flowscript_recovery_test_board() -> Board {
    let mut board = Board::new_detached(
        Some(format!("flowscript-recovery-{}", uuid::Uuid::new_v4())),
        flow_like::flow_like_storage::Path::from("/test"),
    );
    board.name = "Recovery".to_string();
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board
}

fn retained_recovery_context(
    draft_id: &str,
    revision: u64,
) -> flow_like::flow::copilot::FlowIrEditableDraftContext {
    flow_like::flow::copilot::FlowIrEditableDraftContext {
        board_id: "board".to_string(),
        draft_id: draft_id.to_string(),
        revision,
        status: "editing".to_string(),
        base_fingerprint: "base".to_string(),
        missing_modules: vec!["send_reply".to_string()],
        remaining_capabilities: vec!["smtp_send".to_string()],
        diagnostics: Vec::new(),
    }
}

fn workflow_call_result_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = result
        .content
        .iter()
        .find_map(|content| match &content.raw {
            rmcp::model::RawContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .expect("workflow guard result contains JSON text");
    serde_json::from_str(text).expect("workflow guard result is valid JSON")
}

fn board_edit_job_test_record(
    job_id: impl Into<String>,
    phase: BoardEditJobPhase,
    touched_at: Instant,
) -> BoardEditJobRecord {
    let job_id = job_id.into();
    BoardEditJobRecord {
        job: BoardEditJob {
            schema_version: BOARD_EDIT_JOB_SCHEMA_VERSION.to_string(),
            job_id: job_id.clone(),
            app_id: "review-app".to_string(),
            board_id: "review-board".to_string(),
            request_id: Some(format!("request-{job_id}")),
            remote_profile_id: None,
            remote_principal_id: None,
            remote_hub: None,
            phase,
            created_at_ms: 1,
            updated_at_ms: 1,
            expires_at_ms: 2,
            token: FlowIrCommitToken {
                board_id: "review-board".to_string(),
                draft_id: format!("draft-{job_id}"),
                revision: 3,
                base_fingerprint: "base".to_string(),
                claim_id: format!("claim-{job_id}"),
                requires_destructive_approval: false,
            },
            approval: flow_like::flow::copilot::tool_spec::ResolvedToolApproval::none(),
            review: BoardEditJobReview {
                command_count: 0,
                command_counts: BTreeMap::new(),
                command_summaries: Vec::new(),
                replacement_mode: false,
                destructive_effects: Vec::new(),
            },
            flowscript_source: None,
            persisted_board_fingerprint: None,
            result: None,
            error: None,
        },
        board_commands: vec![BoardCommand::AddNode {
            node_type: "events_generic".to_string(),
            ref_id: Some(format!("node-{job_id}")),
            position: None,
            friendly_name: None,
            additional_pins: None,
            target_layer: None,
            summary: None,
        }],
        replacement_mode: false,
        touched_at,
        resolution_lock: Arc::new(tokio::sync::Mutex::new(())),
        delivery_lease: None,
    }
}

fn unstarted_pool_client() -> Arc<Client> {
    Arc::new(Client::builder().build().expect("unstarted pool client"))
}

fn leaked_test_pool(size: usize) -> &'static NestedCopilotPool {
    Box::leak(Box::new(NestedCopilotPool::new(size)))
}

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

fn accept_plan(
    state: &Arc<StdMutex<WorkflowToolLoopState>>,
    args: serde_json::Value,
) -> serde_json::Value {
    workflow_call_result_json(
        &workflow_tool_preflight_with_args(state, "plan_board_scope", &args)
            .expect("plan_board_scope is answered by the host loop"),
    )
}

fn staged_plan_args(segments: usize) -> serde_json::Value {
    let segments = (1..=segments)
        .map(|index| {
            serde_json::json!({
                "id": format!("s{index}"),
                "title": format!("Segment {index}"),
                "behavior": format!("Build and fully wire the nodes belonging to slice {index}."),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "strategy": "staged", "segments": segments })
}

/// Move the ledger forward the way a real run does: one more revision checked clean and a
/// larger retained document.
fn record_forward_progress(state: &mut WorkflowToolLoopState, source: &str) {
    state.valid_checks = state.valid_checks.saturating_add(1);
    state.last_flowscript = Some(source.to_string());
}

/// The one-segment plan an ordinary edit declares, for tests that build loop state directly.
fn single_segment_plan() -> BoardScopePlan {
    use flow_like::flow::copilot::{CURRENT_BOARD_REF, PlannedSegment};

    accept_scope_plan(PlanBoardScopeArgs {
        strategy: ScopeStrategy::Single,
        segments: vec![PlannedSegment {
            id: "s1".to_string(),
            title: "Whole request".to_string(),
            behavior: "Build the complete requested workflow as one document.".to_string(),
            depends_on: Vec::new(),
            board_ref: CURRENT_BOARD_REF.to_string(),
        }],
        rationale: String::new(),
    })
    .expect("a one-segment single-strategy plan is valid")
}

/// Preflight requires an accepted scope plan before the first source write. Lifecycle tests
/// that are about what happens AFTER that take the one-segment plan an ordinary edit declares.
fn accept_single_segment_plan(state: &Arc<StdMutex<WorkflowToolLoopState>>) {
    let result = workflow_tool_preflight_with_args(
        state,
        "plan_board_scope",
        &serde_json::json!({
            "strategy": "single",
            "segments": [{
                "id": "s1",
                "title": "Whole request",
                "behavior": "Build the complete requested workflow as one document."
            }]
        }),
    )
    .expect("plan_board_scope is answered by the host loop");
    assert_eq!(result.is_error, Some(false));
}

fn rich_support_flowscript() -> &'static str {
    r#"@secret
const IMAP_HOST: string = ""

function pollSupportInbox() {
    const connection = emailImapConnect({ host: IMAP_HOST })
    const inbox = mailImapInbox({ connection: connection.connection })
    const refs = mailImapList({ inbox: inbox.inbox })
    const first = arrayGet({ array: refs.refs, index: 0 })
    const mail = emailImapInboxFetchMail({ emailRef: first.value })
    logInfo({ message: mail.subject })
}

function requestApproval() {
    smtpSendEmail({ to: "example@example.com", subject: "Review" })
}

eventsSimple() {
    pollSupportInbox()
    requestApproval()
}

eventsGeneric(payload: Struct) {
    structGet({ struct: payload, field: "ticket_id" })
}
"#
}

fn seed_failed_rich_candidate(state: &Arc<StdMutex<WorkflowToolLoopState>>) {
    workflow_tool_record(
        state,
        "edit_flowscript",
        &serde_json::json!({ "flowscript": rich_support_flowscript() }),
        &serde_json::json!({
            "status": "validation_errors",
            "errors": ["one connection still needs repair"]
        })
        .to_string(),
    );
}

// 1x1 transparent PNG — enough for arg/stdin plumbing assertions (real
// snapshots must be larger for the model to perceive them).
fn test_chat_image() -> ChatImage {
    ChatImage {
            data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string(),
            media_type: "image/png".to_string(),
        }
}
