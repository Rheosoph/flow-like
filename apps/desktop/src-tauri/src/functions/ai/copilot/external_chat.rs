//! External-provider chat orchestration and recovery.

use super::agent_surface::{build_flowpilot_agent_surface, live_board_handle};
use super::backend_types::{FlowPilotAgentBackendKind, FlowPilotAgentTransportKind};
use super::cli_resolution::{external_agent_cli_resolution_failure, find_cli_resolution};
use super::client_pool::{
    acquire_nested_copilot_run_permit, nested_copilot_run_gate, nested_copilot_run_gate_key,
};
use super::external_continuation::{
    build_external_agent_prompt_body, build_external_agent_prompt_with_options,
    build_external_workflow_continuation_prompt, earn_nested_wall_clock_extension,
    external_agent_role_appendix_with_options, external_workflow_incomplete_error,
    nested_wall_clock_exhausted, nested_wall_clock_extended, nested_wall_clock_incomplete_error,
    workflow_continuation_budget,
};
use super::external_invocation::ExternalAgentInvocation;
use super::external_process::run_external_agent_invocation;
use super::external_stream::send_external_progress_event;
use super::global_chat::drain_global_chat_steering;
use super::mcp::{FlowPilotMcpBridge, McpToolActivityState, mcp_total_tool_calls};
use super::provider_errors::{
    EXTERNAL_AGENT_TOOL_CALL_ID, ExternalAgentRunOutput, actionable_external_agent_failure,
    can_resume_external_workflow_after_failure, external_agent_run_failure,
};
use super::runtime::{frontend_tool_channel, register_copilot_run};
use super::stream_events::{
    flowscript_response_workspace_envelope, render_recovered_mutation_message,
    scoped_parent_request_id, send_correlated_stream_json_event,
};
use super::tool_policy::{
    SideEffectCommandQueueCleanup, abandon_side_effect_commands, build_flowpilot_sdk_tools,
    is_flowpilot_read_only_tool, take_side_effect_delivery,
};
use super::workflow_reporting::WorkflowRunSummaryEmitter;
use super::workflow_sdk::{InitialSourceCheckpointPhase, workflow_initial_source_checkpoint_phase};
use super::workflow_state::{
    EXTERNAL_CIRCUIT_OPEN_PHASE_END_GRACE, EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET,
    EXTERNAL_STAGED_COMMIT_PREFIX_RATIO, EXTERNAL_TRANSIENT_RESTART_BACKOFF,
    MAX_EXTERNAL_TRANSPORT_RESTARTS, MAX_EXTERNAL_WORKFLOW_CONTINUATIONS,
    MAX_EXTERNAL_ZERO_ACTIVITY_RESTARTS, NESTED_RUN_WALL_CLOCK_BUDGET, TimeExtensionDecision,
    WorkflowProgressMark, WorkflowToolLoopState,
};
use crate::functions::ai::frontend_tool_bridge::FrontendToolContext;
use flow_like::{
    a2ui::SurfaceComponent,
    copilot::{ChatImage, CopilotScope, UnifiedChatMessage, UnifiedCopilotResponse},
    flow::{
        board::Board,
        copilot::{
            WebResearchCapability, flowscript_workspace_envelope, memory::AssistantMemory,
            tool_spec::RESEARCH_AGENT_TOOL, workflow_authoring_tool_allowed,
        },
        node::Node,
    },
};
use std::{
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, ipc::Channel};

pub(super) async fn external_code_agent_chat_internal(
    app_handle: AppHandle,
    backend: FlowPilotAgentBackendKind,
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
    let global_agent = global.is_some();
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
        backend.web_research(),
        tool_context
            .as_ref()
            .and_then(|context| context.board_context_manifest.as_ref()),
        read_only,
    );
    surface.live_board = live_board;
    surface.capabilities.tool_protocol = FlowPilotAgentTransportKind::Mcp;
    let _side_effect_cleanup = SideEffectCommandQueueCleanup(surface.side_effect_commands.clone());

    let cli = find_cli_resolution(backend, Some(&app_handle)).ok_or_else(|| {
        actionable_external_agent_failure(backend, &external_agent_cli_resolution_failure(backend))
    })?;

    let workflow_edit_request = surface.workflow_edit_request;
    let parent_request_id = scoped_parent_request_id(tool_context.as_ref());
    let (run_cancellation, _run_registration) =
        register_copilot_run(request_id.as_deref().or(parent_request_id.as_deref()));
    // Codex/Claude Code CLI processes are already per-invocation, so no process pool is needed
    // here. Mutation-lane gates give nested runs the same state serialization as SDK and Bits.
    // The permit is held for the entire run.
    let _nested_run_permit = if nested {
        Some(
            acquire_nested_copilot_run_permit(
                nested_copilot_run_gate(&nested_copilot_run_gate_key(
                    scope,
                    board,
                    tool_context.as_ref(),
                )),
                run_cancellation.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    // Started after the mutation-lane gate so serialized queue time does not consume the budget.
    let nested_wall_clock_deadline = nested.then(|| Instant::now() + NESTED_RUN_WALL_CLOCK_BUDGET);
    let tool_channel = frontend_tool_channel(tool_context.as_ref(), request_id.as_deref()).await;
    let mut tools = build_flowpilot_sdk_tools(
        app_handle,
        scope,
        &surface,
        global_agent,
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
        // A live FlowScript already contains the graph structure. Hiding legacy/manual discovery
        // tools removes the strongest attractors for code-agent search loops and keeps the exposed
        // MCP surface focused on one declaration batch plus iterative text edits.
        tools.retain(|(tool, _)| workflow_authoring_tool_allowed(&tool.name));
    }
    if global_agent && backend.web_research() == WebResearchCapability::Native {
        tools.retain(|(tool, _)| tool.name != RESEARCH_AGENT_TOOL);
    }
    super::workflow_benchmark::observe_tools(board, &mut tools);
    let tool_names = tools
        .iter()
        .map(|(tool, _)| tool.name.clone())
        .collect::<Vec<_>>();
    let tool_name_summary = tool_names.join(", ");

    send_correlated_stream_json_event(
        &channel,
        "tool_start",
        &serde_json::json!({
            "tool_call_id": EXTERNAL_AGENT_TOOL_CALL_ID,
            "tool": backend.cli_name(),
            "status": "running",
            "summary": format!("Starting {}", backend.label()),
        }),
        parent_request_id.as_deref(),
    );
    send_external_progress_event(
        &channel,
        EXTERNAL_AGENT_TOOL_CALL_ID,
        &format!(
            "Starting {} with shared FlowPilot MCP tools: {}",
            backend.label(),
            tool_name_summary
        ),
        parent_request_id.as_deref(),
    );
    let workflow_state = workflow_edit_request.then(|| {
        let mut state =
            WorkflowToolLoopState::from_flowscript_recovery(surface.flowscript_recovery.as_ref());
        state.attach_shared_session(surface.workflow_manifest.clone());
        Arc::new(StdMutex::new(state))
    });
    let tool_activity = Arc::new(StdMutex::new(McpToolActivityState::default()));
    let mut run_summary = WorkflowRunSummaryEmitter::new(
        channel.clone(),
        parent_request_id.clone(),
        backend.cli_name(),
        model_id,
        run_cancellation.clone(),
    );
    run_summary.attach_workflow_state(workflow_state.clone());
    let mut final_workflow_snapshot = None;
    let mut last_successful_mutation = None;
    let mut continuation = 0u8;
    let mut phases_run = 0u32;
    let mut zero_activity_restarts = 0u8;
    let mut previous_exhausted_budget: Option<String> = None;
    let mut previous_exhausted_progress: Option<WorkflowProgressMark> = None;
    // Claude Code receives the bounded role/lifecycle appendix through --append-system-prompt so
    // it lands in the real system prompt; other backends keep it inline in the stdin prompt.
    let prompt_profile = surface.prompt_profile;
    let prompt_mode = surface.prompt_mode;
    let claude_role_appendix =
        matches!(backend, FlowPilotAgentBackendKind::ClaudeCode).then(|| {
            external_agent_role_appendix_with_options(
                scope,
                workflow_edit_request,
                global_agent,
                prompt_profile,
                prompt_mode,
            )
        });
    // The latest Claude session id observed on a finished phase; every later phase in this run
    // (continuation or transport restart) resumes it so the model keeps its own transcript
    // instead of a lossy host reconstruction. Phases with no captured id fall back to the full
    // re-wrapped prompt in a fresh session.
    let mut resume_session_id: Option<String> = None;
    let mut next_phase_resume: Option<String> = None;
    let mut prompt = if claude_role_appendix.is_some() {
        build_external_agent_prompt_body(&surface.system_content, &user_prompt)
    } else {
        build_external_agent_prompt_with_options(
            &surface.system_content,
            &user_prompt,
            scope,
            workflow_edit_request,
            global_agent,
            prompt_profile,
            prompt_mode,
        )
    };
    let agent_result = loop {
        if nested_wall_clock_exhausted(
            nested_wall_clock_deadline
                .map(|deadline| nested_wall_clock_extended(deadline, workflow_state.as_ref())),
        ) && !earn_nested_wall_clock_extension(workflow_state.as_ref())
        {
            run_summary.mark_budget_incomplete();
            break Err(nested_wall_clock_incomplete_error(
                final_workflow_snapshot.as_ref(),
                continuation,
            ));
        }
        run_summary.record_phase();
        let phase_start_tool_calls = mcp_total_tool_calls(&tool_activity);
        // A fresh MCP server per provider phase is deliberate. It makes the phase URL an epoch:
        // delayed requests from a killed CLI cannot register as work owned by the next repair.
        let mcp_bridge = match FlowPilotMcpBridge::start(
            tools.clone(),
            workflow_state.clone(),
            tool_activity.clone(),
        )
        .await
        {
            Ok(bridge) => bridge,
            Err(error) => break Err(error),
        };
        let mcp_url = mcp_bridge.url.clone();
        let mut invocation = match ExternalAgentInvocation::new(
            backend,
            cli.clone(),
            model_id,
            reasoning_effort,
            &mcp_url,
            prompt,
            tool_names.clone(),
            global_agent,
            current_images.as_deref().unwrap_or_default(),
            next_phase_resume.take().as_deref(),
            claude_role_appendix.as_deref(),
        ) {
            Ok(invocation) => invocation,
            Err(error) => {
                let _ = mcp_bridge.finish_phase().await;
                break Err(error);
            }
        };
        invocation.continues_streamed_text = phases_run > 0;
        send_external_progress_event(
            &channel,
            EXTERNAL_AGENT_TOOL_CALL_ID,
            &format!("Using {} via {}", backend.label(), mcp_url),
            parent_request_id.as_deref(),
        );
        // A nested run's wall-clock deadline cancels only this invocation's child token: the CLI
        // process is killed through the existing forceful-cancellation machinery, while the run
        // itself stays alive to report a graceful, terminal incomplete result below.
        let invocation_cancellation = run_cancellation.child_token();
        // The deadline is armed before the model plans, so a segmented build earns its extra wall
        // clock mid-phase. Poll instead of sleeping to a fixed instant so the extension applies to
        // the very phase that declared the plan.
        let wall_clock_watchdog = nested_wall_clock_deadline.map(|deadline| {
            let cancel_invocation = invocation_cancellation.clone();
            let state = workflow_state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if Instant::now() < nested_wall_clock_extended(deadline, state.as_ref()) {
                        continue;
                    }
                    // Try to EARN more time before killing the phase. A run that is still moving
                    // forward keeps its in-flight work instead of losing it at an arbitrary
                    // boundary; one that is circling produces an unchanged progress mark, is
                    // refused here, and is cancelled exactly as before.
                    let granted = state.as_ref().is_some_and(|state| {
                        state.lock().is_ok_and(|mut state| {
                            matches!(
                                state.try_grant_time_extension(),
                                TimeExtensionDecision::Granted { .. }
                            )
                        })
                    });
                    if !granted {
                        cancel_invocation.cancel();
                        return;
                    }
                }
            })
        });
        // A staged plan grows one draft toward a single commit, so running out of wall clock loses
        // every segment. Past this ratio the host asks it to commit the coherent prefix it already
        // validated; the remaining segments continue in a fresh run against the applied board.
        let staged_prefix_watchdog =
            workflow_state
                .as_ref()
                .zip(nested_wall_clock_deadline)
                .map(|(state, deadline)| {
                    let state = state.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            let Ok(mut state) = state.lock() else {
                                return;
                            };
                            if state.queued || state.staged_prefix_commit_requested {
                                return;
                            }
                            let total = NESTED_RUN_WALL_CLOCK_BUDGET
                                .saturating_add(state.wall_clock_extension());
                            let started = deadline
                                .checked_sub(NESTED_RUN_WALL_CLOCK_BUDGET)
                                .unwrap_or(deadline);
                            let threshold =
                                started + total.mul_f64(EXTERNAL_STAGED_COMMIT_PREFIX_RATIO);
                            if Instant::now() >= threshold && state.request_staged_prefix_commit() {
                                return;
                            }
                        }
                    })
                });
        let predraft_checkpoint_fired = Arc::new(AtomicBool::new(false));
        let predraft_checkpoint_watchdog = workflow_state.as_ref().map(|state| {
            let state = state.clone();
            let cancel_invocation = invocation_cancellation.clone();
            let fired = predraft_checkpoint_fired.clone();
            tokio::spawn(async move {
                let mut ready_since: Option<Instant> = None;
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let checkpoint = match state.lock() {
                        Ok(state) => workflow_initial_source_checkpoint_phase(&state),
                        Err(_) => return,
                    };
                    match checkpoint {
                        InitialSourceCheckpointPhase::Complete => {
                            // A source operation started or a draft was retained; the soft
                            // checkpoint did its job and must not interfere with validation.
                            return;
                        }
                        InitialSourceCheckpointPhase::AwaitingPrerequisites
                        | InitialSourceCheckpointPhase::AncillaryContextInFlight => {
                            // Planning is a prerequisite, not part of the source-writing budget.
                            // Likewise, a context read admitted by the shared workflow session may
                            // legitimately run longer than this checkpoint. Give the model a fresh
                            // source-writing window after either prerequisite settles.
                            ready_since = None;
                            continue;
                        }
                        InitialSourceCheckpointPhase::AwaitingInitialSource => {}
                    }
                    let started = ready_since.get_or_insert_with(Instant::now);
                    if started.elapsed() >= EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET {
                        fired.store(true, AtomicOrdering::Relaxed);
                        cancel_invocation.cancel();
                        return;
                    }
                }
            })
        });
        // Once the shared circuit opens, every further mutation tool is refused pre-dispatch and
        // nothing dispatched can close it again within this phase. Give the CLI a short grace
        // window to stop on its own, then end the phase so the continuation (which resets the
        // circuit) starts instead of letting refused tool calls idle out the whole budget.
        let circuit_open_fired = Arc::new(AtomicBool::new(false));
        let circuit_open_watchdog = workflow_state.as_ref().map(|state| {
            let state = state.clone();
            let cancel_invocation = invocation_cancellation.clone();
            let fired = circuit_open_fired.clone();
            tokio::spawn(async move {
                let mut open_since: Option<Instant> = None;
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let open = match state.lock() {
                        Ok(state) => {
                            let elapsed_ms = state.shared_session_elapsed_ms();
                            state
                                .shared_session
                                .as_ref()
                                .map(|session| session.snapshot(elapsed_ms))
                                .and_then(|snapshot| snapshot.circuit)
                                .is_some()
                        }
                        Err(_) => return,
                    };
                    if !open {
                        open_since = None;
                        continue;
                    }
                    let started = open_since.get_or_insert_with(Instant::now);
                    if started.elapsed() >= EXTERNAL_CIRCUIT_OPEN_PHASE_END_GRACE {
                        fired.store(true, AtomicOrdering::Relaxed);
                        cancel_invocation.cancel();
                        return;
                    }
                }
            })
        });
        let mut run_result = run_external_agent_invocation(
            invocation,
            channel.clone(),
            parent_request_id.clone(),
            invocation_cancellation,
            board.map(|board| board.id.clone()),
        )
        .await;
        phases_run = phases_run.saturating_add(1);
        if let Ok(output) = run_result.as_ref()
            && let Some(session) = output.session_id.as_deref()
        {
            resume_session_id = Some(session.to_string());
        }
        if let Some(watchdog) = wall_clock_watchdog {
            watchdog.abort();
        }
        if let Some(watchdog) = predraft_checkpoint_watchdog {
            watchdog.abort();
        }
        if let Some(watchdog) = staged_prefix_watchdog {
            watchdog.abort();
        }
        if let Some(watchdog) = circuit_open_watchdog {
            watchdog.abort();
        }
        if predraft_checkpoint_fired.load(AtomicOrdering::Relaxed) {
            if let Some(state) = workflow_state.as_ref()
                && let Ok(mut state) = state.lock()
                && !state.flowscript_draft_retained
            {
                state.last_status = Some("declarations_ready_no_source".to_string());
            }
            run_result = Err(format!(
                "FlowPilot pre-draft source checkpoint timed out after {} seconds with usable declarations but no source operation; continue in a fresh bounded phase, reuse the accepted scope plan or call plan_board_scope exactly once, then call write_flowscript for its active segment",
                EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET.as_secs()
            ));
        } else if circuit_open_fired.load(AtomicOrdering::Relaxed) {
            run_result = Err(
                "FlowPilot shared zero-progress circuit opened mid-phase; the host ended the provider phase so a bounded continuation with a reset circuit can retry a materially different strategy"
                    .to_string(),
            );
        }

        let phase_outcome = match mcp_bridge.finish_phase().await {
            Ok(outcome) => outcome,
            Err(error) => break Err(error),
        };
        final_workflow_snapshot = phase_outcome.workflow_snapshot;
        last_successful_mutation = phase_outcome.last_successful_mutation;
        let queued = final_workflow_snapshot
            .as_ref()
            .is_some_and(|state| state.queued);
        let run_failure = external_agent_run_failure(&run_result).map(str::to_string);
        // A phase that managed to queue its batch before the deadline still returns normally; an
        // externally cancelled run keeps its own terminal reporting.
        if nested_wall_clock_exhausted(
            nested_wall_clock_deadline
                .map(|deadline| nested_wall_clock_extended(deadline, workflow_state.as_ref())),
        ) && !queued
            && !run_cancellation.is_cancelled()
            && !earn_nested_wall_clock_extension(workflow_state.as_ref())
        {
            run_summary.mark_budget_incomplete();
            break Err(nested_wall_clock_incomplete_error(
                final_workflow_snapshot.as_ref(),
                continuation,
            ));
        }
        if !workflow_edit_request || queued || run_cancellation.is_cancelled() {
            break run_result;
        }
        if run_failure.as_deref().is_some_and(|error| {
            !can_resume_external_workflow_after_failure(
                final_workflow_snapshot.as_ref(),
                error,
                run_cancellation.is_cancelled(),
            )
        }) {
            break run_result;
        }

        let exhausted_budget = final_workflow_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.exhausted_budget.clone());
        if exhausted_budget.is_some() && exhausted_budget == previous_exhausted_budget {
            // The previous continuation already received a fresh bounded slice for this exact
            // budget and burned it again. Terminal only when the progress ledger ALSO failed to
            // advance across that slice: a run that spent the slice moving forward earns another,
            // a circling run stops honestly here.
            let current_progress = workflow_state
                .as_ref()
                .and_then(|state| state.lock().ok().map(|state| state.progress_mark()));
            let progressed = matches!(
                (current_progress.as_ref(), previous_exhausted_progress.as_ref()),
                (Some(mark), Some(previous)) if mark.advanced_beyond(previous)
            );
            if !progressed {
                run_summary.mark_budget_incomplete();
                break Err(external_workflow_incomplete_error(
                    final_workflow_snapshot.as_ref(),
                    continuation,
                ));
            }
        }

        let phase_tool_calls =
            mcp_total_tool_calls(&tool_activity).saturating_sub(phase_start_tool_calls);
        // Failures reach this point only when already classified transient. Host-initiated phase
        // ends (pre-draft checkpoint, circuit-open cancellation) must still consume a
        // continuation — the continuation grant is what resets the circuit and re-slices the
        // budget. Pure provider/transport failures (stream disconnects, resets) are not workflow
        // work and retry on their own bounded counter even mid-phase; burning a continuation per
        // dropped stream ended runs with no FlowScript after two unlucky disconnects.
        let host_initiated_phase_end = run_failure.as_deref().is_some_and(|error| {
            error.contains("pre-draft source checkpoint") || error.contains("zero-progress circuit")
        });
        if run_failure.is_some() && !host_initiated_phase_end {
            let restart_cap = if phase_tool_calls == 0 {
                MAX_EXTERNAL_ZERO_ACTIVITY_RESTARTS
            } else {
                MAX_EXTERNAL_TRANSPORT_RESTARTS
            };
            if zero_activity_restarts >= restart_cap {
                break run_result;
            }
            zero_activity_restarts = zero_activity_restarts.saturating_add(1);
        } else {
            // Phases end for many reasons across an hours-long build, so the continuation cap grows
            // with earned time. The repeat-exhausted-budget rule above is what still stops circling.
            if continuation >= workflow_continuation_budget(workflow_state.as_ref()) {
                run_summary.mark_budget_incomplete();
                break Err(external_workflow_incomplete_error(
                    final_workflow_snapshot.as_ref(),
                    continuation,
                ));
            }
            continuation = continuation.saturating_add(1);
            run_summary.record_continuation();
            if let Some(workflow_state) = workflow_state.as_ref()
                && let Ok(mut state) = workflow_state.lock()
            {
                state.grant_continuation_slice();
            }
            previous_exhausted_budget = exhausted_budget;
            previous_exhausted_progress = workflow_state
                .as_ref()
                .and_then(|state| state.lock().ok().map(|state| state.progress_mark()));
        }

        if run_failure.is_some() {
            // Give a transient provider/transport failure a moment to clear before restarting the
            // phase, without ignoring an end-to-end cancellation while waiting.
            let cancelled_during_backoff = tokio::select! {
                _ = tokio::time::sleep(EXTERNAL_TRANSIENT_RESTART_BACKOFF) => false,
                _ = run_cancellation.cancelled() => true,
            };
            if cancelled_during_backoff {
                break run_result;
            }
        }

        let mut repair_request = build_external_workflow_continuation_prompt(
            &raw_user_prompt,
            final_workflow_snapshot.as_ref(),
            continuation.max(1),
        );
        // The codex/claude-code CLIs receive their whole prompt on stdin and then see EOF, so a
        // phase restart is the only point where a mid-run instruction can reach them. Anything
        // still queued when the run ends is handed back to the frontend and re-sent as its own
        // turn, so a steer is never silently dropped on a single-phase run.
        if let Some(run_id) = request_id.as_deref() {
            let steering = drain_global_chat_steering(run_id).await;
            if !steering.is_empty() {
                repair_request.push_str(&format!(
                    "\n\nThe user sent this while you were working. Treat it as part of the current request and adjust course now:\n{}",
                    steering.join("\n")
                ));
            }
        }
        if let Some(error) = run_failure.as_deref() {
            let recovery_action = if final_workflow_snapshot.as_ref().is_some_and(|state| {
                state.flowscript_draft_retained && state.last_flowscript.is_some()
            }) {
                "The host retained an exact FlowScript source revision. Continue that draft/revision and do not duplicate a queued commit."
            } else if final_workflow_snapshot
                .as_ref()
                .is_some_and(|state| state.typed_draft_retained)
            {
                "The host retained an exact typed draft revision. Continue that draft/revision and do not start a second mutation path."
            } else if final_workflow_snapshot
                .as_ref()
                .is_some_and(|state| state.scope_plan.is_some())
            {
                "The host retained the accepted scope plan. Do not call plan_board_scope again; create the first draft for its active segment with write_flowscript."
            } else {
                "No draft revision was retained. Resume the bounded pre-draft loop from host-retained declaration/read state: obtain usable declarations if needed, call plan_board_scope exactly once, then create the first draft with write_flowscript."
            };
            repair_request.push_str(&format!(
                "\n\nINTERNAL TRANSIENT RECOVERY: the previous provider/transport phase ended with `{}`. The host opened a fresh bounded phase. {recovery_action}",
                flow_like::flow::copilot::stream::safe_text_preview(error, 600),
            ));
        }
        prompt = match (
            claude_role_appendix.as_deref(),
            resume_session_id.as_deref(),
        ) {
            // Resumed continuation: the session already holds the platform prompt and the
            // model's own transcript — send only the compact continuation payload.
            (Some(_), Some(session)) => {
                next_phase_resume = Some(session.to_string());
                repair_request
            }
            (Some(_), None) => {
                build_external_agent_prompt_body(&surface.system_content, &repair_request)
            }
            (None, _) => build_external_agent_prompt_with_options(
                &surface.system_content,
                &repair_request,
                scope,
                true,
                global_agent,
                prompt_profile,
                prompt_mode,
            ),
        };
        send_external_progress_event(
            &channel,
            EXTERNAL_AGENT_TOOL_CALL_ID,
            &format!(
                "{} ended before queueing changes; continuing the bounded workflow run ({continuation}/{MAX_EXTERNAL_WORKFLOW_CONTINUATIONS})",
                backend.label()
            ),
            parent_request_id.as_deref(),
        );
    };

    if run_cancellation.is_cancelled() {
        abandon_side_effect_commands(&surface.side_effect_commands);
    }

    let raw_error_note = match &agent_result {
        Ok(output) => output.error.clone(),
        Err(error) => Some(error.clone()),
    };
    let error_note = raw_error_note
        .as_deref()
        .map(|error| actionable_external_agent_failure(backend, error));
    let debug_error_note = error_note
        .as_deref()
        .map(|error| flow_like::flow::copilot::stream::safe_text_preview(error, 1_200));
    send_correlated_stream_json_event(
        &channel,
        "tool_end",
        &serde_json::json!({
            "tool_call_id": EXTERNAL_AGENT_TOOL_CALL_ID,
            "tool": backend.cli_name(),
            "status": if error_note.is_some() { "error" } else { "done" },
            "result_summary": debug_error_note
                .clone()
                .unwrap_or_else(|| format!("{} finished", backend.label())),
            "error": debug_error_note,
        }),
        parent_request_id.as_deref(),
    );
    run_summary.resolve_outcome(error_note.is_some(), workflow_edit_request);

    let has_retained_candidate = final_workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.last_flowscript.as_ref())
        .is_some();
    let agent_output = match agent_result {
        Ok(output) => output,
        Err(error) if last_successful_mutation.is_some() => ExternalAgentRunOutput {
            text: render_recovered_mutation_message(
                last_successful_mutation
                    .as_ref()
                    .expect("guarded by is_some"),
            ),
            error: Some(error),
            session_id: None,
        },
        Err(error) if workflow_edit_request && has_retained_candidate => ExternalAgentRunOutput {
            text: String::new(),
            error: Some(error),
            session_id: None,
        },
        Err(error) => return Err(actionable_external_agent_failure(backend, &error)),
    };
    let text = agent_output.text.trim().to_string();
    let display_error = agent_output
        .error
        .map(|error| actionable_external_agent_failure(backend, &error));
    let message = match (display_error, text.is_empty()) {
        (Some(error), true) if has_retained_candidate => format!(
            "{} retained the most complete FlowScript draft for repair, but did not queue it because validation is still failing: {error}",
            backend.label()
        ),
        (Some(error), true) => return Err(error),
        (Some(error), false) => format!(
            "{text}\n\n> Note: {} ended with an error after this partial response: {error}",
            backend.label()
        ),
        (None, true) => format!(
            "{} completed without a final text response.",
            backend.label()
        ),
        (None, false) => text,
    };
    let message = if final_workflow_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.modular_fallback.as_ref())
        .is_some()
    {
        "Queued an independently runnable partial working slice for review. The requested application is still incomplete; the fuller failed FlowScript remains retained for another repair pass. Do not treat this as full completion."
            .to_string()
    } else {
        message
    };

    // `emit_ui` results are invisible to the MCP transport, so rendered surfaces are drained from
    // the shared store — the LAST successful emit wins, matching the SDK path's extraction.
    let emitted_surface = surface
        .emitted_surfaces
        .lock()
        .ok()
        .and_then(|mut surfaces| surfaces.drain(..).next_back());
    let (components, canvas_settings, root_component_id) = match emitted_surface {
        Some(emitted) => (
            serde_json::from_value::<Vec<SurfaceComponent>>(emitted.components).unwrap_or_default(),
            Some(emitted.canvas_settings),
            Some(emitted.root_component_id),
        ),
        None => (Vec::new(), None, None),
    };
    if !components.is_empty() {
        let comp_event = format!(
            "<components>{}</components>",
            serde_json::to_string(&components).unwrap_or_default()
        );
        let _ = channel.send(comp_event);
        if let Some(canvas) = &canvas_settings {
            let canvas_event = format!(
                "<canvas_settings>{}</canvas_settings>",
                serde_json::to_string(canvas).unwrap_or_default()
            );
            let _ = channel.send(canvas_event);
        }
    }

    let queued_workspace = surface
        .queued_flowscript
        .lock()
        .ok()
        .and_then(|workspace| workspace.clone());
    let flowscript_workspace = queued_workspace
        .as_deref()
        .map(|source| {
            flowscript_response_workspace_envelope(
                source,
                "queued",
                final_workflow_snapshot.as_ref(),
            )
        })
        .or_else(|| {
            final_workflow_snapshot.as_ref().and_then(|snapshot| {
                snapshot.last_flowscript.as_deref().map(|source| {
                    flowscript_workspace_envelope(
                        source,
                        snapshot
                            .last_status
                            .as_deref()
                            .unwrap_or("validation_errors"),
                    )
                })
            })
        });

    let (commands, flow_ir_commit) = take_side_effect_delivery(&surface.side_effect_commands);
    run_summary.set_applied_commands(commands.len());
    Ok(UnifiedCopilotResponse {
        message,
        commands,
        suggestions: Vec::new(),
        components,
        canvas_settings,
        root_component_id,
        flowscript_workspace,
        flow_ir_commit,
        active_scope: scope,
    })
}
