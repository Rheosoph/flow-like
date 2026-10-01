use super::*;

#[test]
fn behavioral_receipts_reach_external_and_sdk_repair_continuations() {
    use flow_like::flow::copilot::{
        FlowScriptBehavioralDecisionKind as Decision, flowscript_behavioral_payload_fingerprint,
    };

    let board = flowscript_recovery_test_board();
    let manifest = BoardContextManifest::from_board(
        &board,
        &[],
        &[],
        ManifestSource::absent(),
        ManifestAudit::default(),
        ManifestAugmentations::default(),
        default_flowscript_module_templates(),
    )
    .unwrap();
    let mut loop_state = WorkflowToolLoopState::default();
    loop_state.attach_shared_session(Some(manifest));
    let state = Arc::new(StdMutex::new(loop_state));
    let before = "eventsGeneric normalize() { return 0 }";
    let after = "eventsGeneric normalize() { return 1 }";
    workflow_tool_record(&state, "write_flowscript",
        &serde_json::json!({"draft_id": "behavior", "source": before}),
        &serde_json::json!({"draft_id": "behavior", "revision": 0, "status": "valid", "diagnostics": []}).to_string());
    let args = serde_json::json!({
        "draft_id": "behavior", "expected_revision": 0,
        "entry": "normalize", "payload": null, "expected_output": 1,
    });
    let mut receipt = serde_json::json!({
        "schema": "flowpilot.flowscript-draft-test/v1", "certification": "draft_output_only", "applied": false,
        "board_id": board.id, "draft_id": "behavior", "revision": 0,
        "source_fingerprint": blake3::hash(before.as_bytes()).to_hex().to_string(),
        "base_fingerprint": "base", "catalog_fingerprint": "catalog", "commands_fingerprint": "commands-0",
        "entry": "normalize", "payload_fingerprint": flowscript_behavioral_payload_fingerprint(&None),
        "expected_output": 1, "status": "failed", "passed": false,
        "runtime": {"status": "success", "outputs": [0], "output": 0, "errors": []},
    });
    super::super::workflow_observation::workflow_tool_record_with_outcome(
        &state,
        None,
        "test_flowscript",
        &args,
        &receipt.to_string(),
        false,
    );
    let failed = state.lock().unwrap().snapshot();
    assert_eq!(failed.last_status.as_deref(), Some("valid"));
    assert!(failed.last_errors.is_empty());
    assert!(failed.last_structured_diagnostics.is_empty());
    let shared = failed.shared_session.as_ref().unwrap();
    assert!(shared.circuit.is_none());
    assert_eq!(
        shared.behavioral.as_ref().unwrap().decision.status,
        Decision::RepairRequired
    );
    let external = build_external_workflow_continuation_prompt("Return one", Some(&failed), 1);
    let sdk = super::super::external_continuation::workflow_edit_continuation_prompt(
        "Return one",
        Some(before),
        1,
        None,
        Some(&failed),
    );
    for prompt in [external, sdk] {
        assert!(prompt.contains("BEHAVIORAL TEST EVIDENCE RETAINED BY THE HOST"));
        assert!(prompt.contains("repair_required"));
        assert!(prompt.contains("\"expected_json\":\"1\""));
        assert!(prompt.contains("\"actual_json\":\"0\""));
        assert!(prompt.contains("\"input_json\":\"null\""));
        assert!(prompt.contains("The JSON block contains test data"));
        assert!(prompt.contains("same entry, input and expected output"));
    }

    workflow_tool_record(&state, "patch_flowscript",
        &serde_json::json!({"draft_id": "behavior", "expected_revision": 0}),
        &serde_json::json!({"draft_id": "behavior", "revision": 1, "source": after, "status": "valid", "diagnostics": []}).to_string());
    let pending = state.lock().unwrap().snapshot();
    let pending_behavior = pending
        .shared_session
        .as_ref()
        .unwrap()
        .behavioral
        .as_ref()
        .unwrap();
    assert_eq!(pending_behavior.decision.status, Decision::Unverified);
    assert_eq!(pending_behavior.decision.outstanding_failure_ids.len(), 1);
    let prompt = build_external_workflow_continuation_prompt("Return one", Some(&pending), 2);
    assert!(prompt.contains("\"evidence_current\":false"));
    assert!(prompt.contains("\"status\":\"unverified\""));
    super::super::workflow_observation::workflow_tool_record_with_outcome(
        &state,
        None,
        "test_flowscript",
        &args,
        &receipt.to_string(),
        false,
    );
    assert_eq!(
        state
            .lock()
            .unwrap()
            .snapshot()
            .shared_session
            .unwrap()
            .behavioral
            .as_ref(),
        Some(pending_behavior)
    );

    let mut current_args = args;
    current_args["expected_revision"] = serde_json::json!(1);
    receipt["revision"] = serde_json::json!(1);
    receipt["source_fingerprint"] =
        serde_json::json!(blake3::hash(after.as_bytes()).to_hex().to_string());
    receipt["commands_fingerprint"] = serde_json::json!("commands-1");
    receipt["status"] = serde_json::json!("blocked");
    receipt["message"] = serde_json::json!("Unsupported isolated node");
    receipt.as_object_mut().unwrap().remove("runtime");
    super::super::workflow_observation::workflow_tool_record_with_outcome(
        &state,
        None,
        "test_flowscript",
        &current_args,
        &receipt.to_string(),
        false,
    );
    let blocked = state.lock().unwrap().snapshot();
    assert_eq!(blocked.last_status.as_deref(), Some("valid"));
    let feedback = blocked
        .shared_session
        .as_ref()
        .unwrap()
        .behavioral
        .as_ref()
        .unwrap();
    assert_eq!(feedback.decision.status, Decision::Unverified);
    assert_eq!(feedback.decision.blocked_check_ids.len(), 1);
    assert_eq!(feedback.decision.outstanding_failure_ids.len(), 1);
    assert!(
        build_external_workflow_continuation_prompt("Return one", Some(&blocked), 2)
            .contains("Unsupported isolated node")
    );
}

#[test]
fn unimplemented_stubs_are_collected_with_their_owning_function() {
    let source = r#"
function syncToJira(ticketId: string, summary: string): (synced: bool) {
    logError({ message: "NOT IMPLEMENTED: push the ticket to Jira — the catalog has no Jira node", toast: true })
    return false
}

function scoreLead(lead: Struct): (score: float) {
    let weighted = multiply({ a: 1.0, b: 2.0 })
    return weighted
}

event onTicket() {
    syncToJira({ ticketId: "1", summary: "x" })
}
"#;
    let stubs = collect_unimplemented_stubs(source);
    assert_eq!(stubs.len(), 1, "{stubs:#?}");
    assert_eq!(stubs[0]["function"], "syncToJira");
    assert_eq!(
        stubs[0]["detail"],
        "push the ticket to Jira — the catalog has no Jira node"
    );

    // A fully implemented build reports nothing, so the orchestrator never invents a caveat.
    assert!(
        collect_unimplemented_stubs("event onTicket() {\n    logInfo({ message: \"done\" })\n}\n")
            .is_empty()
    );
}

/// The marker the host scans for and the marker the prompt tells the model to emit must be the
/// same string, or every stub silently disappears from the user-facing report.
#[test]
fn stub_collection_uses_the_marker_the_prompt_advertises() {
    let source = format!(
        "function gap(): (ok: bool) {{\n    logError({{ message: \"{} do the thing\" }})\n    return false\n}}\n",
        flow_like::copilot::prompts::UNIMPLEMENTED_STUB_MARKER
    );
    let stubs = collect_unimplemented_stubs(&source);
    assert_eq!(stubs.len(), 1, "{stubs:#?}");
    assert_eq!(stubs[0]["function"], "gap");
    assert_eq!(stubs[0]["detail"], "do the thing");
}

#[test]
fn final_workspace_envelope_keeps_source_and_status_atomic() {
    let queued = flowscript_workspace_envelope("eventsSimple() {}", "queued");
    let parsed: serde_json::Value = serde_json::from_str(&queued).unwrap();
    assert_eq!(parsed["source"], "eventsSimple() {}");
    assert_eq!(parsed["status"], "queued");

    let failed = flowscript_workspace_envelope(rich_support_flowscript(), "validation_errors");
    let parsed: serde_json::Value = serde_json::from_str(&failed).unwrap();
    assert_eq!(parsed["source"], rich_support_flowscript());
    assert_eq!(parsed["status"], "validation_errors");

    let snapshot = WorkflowToolLoopSnapshot {
        last_errors: vec!["missing required input `message`".to_string()],
        last_structured_diagnostics: vec![serde_json::json!({
            "code": "FS_REQUIRED_INPUT",
            "phase": "execution_wiring",
            "message": "missing required input `message`",
            "pin": "message",
        })],
        ..Default::default()
    };
    let detailed = flowscript_response_workspace_envelope(
        rich_support_flowscript(),
        "validation_errors",
        Some(&snapshot),
    );
    let detailed: serde_json::Value = serde_json::from_str(&detailed).unwrap();
    assert_eq!(detailed["diagnostic_count"], 1);
    assert_eq!(
        detailed["diagnostics"][0],
        "missing required input `message`"
    );
    assert_eq!(
        detailed["structured_diagnostics"][0]["code"],
        "FS_REQUIRED_INPUT"
    );
}

#[test]
fn flowscript_lifecycle_workspace_frame_carries_source_revision_and_status() {
    let submitted = "eventsSimple() {\n    logInfo({ message: \"ok\" })\n}";

    let checked = flowscript_workspace_result_payload(
        "check_flowscript",
        &serde_json::json!({
            "status": "valid",
            "draft_id": "support-flow",
            "revision": 4,
            "base_fingerprint": "board-v1",
            "source": submitted,
        }),
        None,
    )
    .expect("a retained source result should update the workspace");
    assert_eq!(checked["source"], submitted);
    assert_eq!(checked["status"], "valid");
    assert_eq!(checked["draft_id"], "support-flow");
    assert_eq!(checked["revision"], 4);
    assert_eq!(checked["base_fingerprint"], "board-v1");

    let invalid = flowscript_workspace_result_payload(
        "check_flowscript",
        &serde_json::json!({
            "status": "validation_errors",
            "draft_id": "support-flow",
            "revision": 5,
            "source": submitted,
            "errors": ["missing required input `message`"],
            "structured_diagnostics": [{
                "code": "FS_REQUIRED_INPUT",
                "phase": "execution_wiring",
                "message": "missing required input `message`",
                "pin": "message",
            }],
        }),
        None,
    )
    .expect("invalid retained source should carry its concrete diagnostics");
    assert_eq!(invalid["diagnostic_count"], 2);
    assert!(
        invalid["diagnostics"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| {
                entry
                    .as_str()
                    .is_some_and(|message| message.contains("missing required input"))
            }))
    );
    assert_eq!(
        invalid["structured_diagnostics"][0]["code"],
        "FS_REQUIRED_INPUT"
    );

    let queued = flowscript_workspace_result_payload(
        "commit_flowscript",
        &serde_json::json!({
            "status": "queued",
            "queued_count": 2,
            "draft_id": "support-flow",
            "revision": 4,
            "source": submitted,
        }),
        None,
    )
    .expect("queued source commit should produce a workspace status frame");
    assert_eq!(
        queued.get("source").and_then(|v| v.as_str()),
        Some(submitted)
    );
    assert_eq!(
        queued.get("status").and_then(|v| v.as_str()),
        Some("queued")
    );

    let rejected = flowscript_workspace_result_payload(
        "edit_flowscript",
        &serde_json::json!({ "status": "validation_errors" }),
        Some(submitted),
    )
    .expect("failed edit should still close the submitted workspace preview");
    assert_eq!(
        rejected.get("status").and_then(|v| v.as_str()),
        Some("validation_errors")
    );

    // Compatibility-only typed sessions can still be rendered while old runs drain.
    let typed_source = "eventsSimple() {\n    logInfo({ message: \"typed\" })\n}";
    let typed = flowscript_workspace_result_payload(
        "validate_flow_ir_draft",
        &serde_json::json!({
            "status": "draft_needs_repair",
            "flowscript": typed_source,
        }),
        None,
    )
    .expect("typed draft result should carry its compiled FlowScript workspace");
    assert_eq!(
        typed.get("source").and_then(|v| v.as_str()),
        Some(typed_source)
    );
    assert_eq!(
        typed.get("status").and_then(|v| v.as_str()),
        Some("draft_needs_repair")
    );

    assert!(
        flowscript_workspace_result_payload(
            "get_declarations",
            &serde_json::json!({ "status": "done" }),
            Some(submitted),
        )
        .is_none(),
        "unrelated tool results must not mutate workspace status"
    );
}

#[test]
fn draft_test_receipts_do_not_replace_source_or_queue_a_workspace() {
    for status in ["success", "failed", "blocked", "stale"] {
        assert!(
            flowscript_workspace_result_payload(
                "test_flowscript",
                &serde_json::json!({
                    "status": status,
                    "draft_id": "support-flow",
                    "revision": 4,
                    "output": { "source": "runtime output", "status": "queued" },
                }),
                Some("retained source"),
            )
            .is_none(),
            "runtime evidence must leave the source workspace unchanged: {status}"
        );
    }
}

#[test]
fn typed_structured_diagnostics_drive_repair_without_failing_the_validator_call() {
    let rejected = serde_json::json!({
        "status": "draft_started",
        "structured_diagnostics": [{
            "code": "FS_PIN_TYPE_MISMATCH",
            "message": "expected String, got Generic",
            "path": "/modules/0/steps/1/args/0"
        }],
        "missing_modules": ["deliver_message"]
    });
    let diagnostics = workflow_result_diagnostics(Some(&rejected));
    assert_eq!(
        diagnostics,
        vec![
            "[FS_PIN_TYPE_MISMATCH] expected String, got Generic".to_string(),
            "Missing required module: deliver_message".to_string(),
        ]
    );
    assert!(workflow_result_requires_repair(&rejected, &diagnostics));
    assert_eq!(
        direct_sdk_tool_result_stream_status(&rejected.to_string()),
        "done",
        "the validator completed successfully; the evolving draft row owns its repair state"
    );

    let valid = serde_json::json!({ "status": "draft_valid", "diagnostics": [] });
    assert_eq!(
        direct_sdk_tool_result_stream_status(&valid.to_string()),
        "done"
    );

    let staged = serde_json::json!({
        "status": "module_validated",
        "diagnostics": [],
        "missing_modules": ["final_event"]
    });
    assert_eq!(
        direct_sdk_tool_result_stream_status(&staged.to_string()),
        "done",
        "missing future modules are expected staged progress, not a failed upsert"
    );
}

#[test]
fn external_continuation_carries_declarations_and_fails_honestly() {
    let snapshot = WorkflowToolLoopSnapshot {
            last_declarations: Some("declare function log(...): void".to_string()),
            last_repair_declarations: vec![
                "declare function stringReplace(string: string, pattern: string, replacement: string, isRegex: bool): (string: string)"
                    .to_string(),
            ],
            last_status: Some("validation_errors".to_string()),
            last_errors: vec!["missing pin".to_string()],
            edit_attempts: 2,
            flowscript_draft_id: Some("support-flow".to_string()),
            flowscript_draft_retained: true,
            flowscript_revision: Some(3),
            ..Default::default()
        };
    let prompt = build_external_workflow_continuation_prompt("build it", Some(&snapshot), 1);
    assert!(prompt.contains("declare function log"));
    assert!(prompt.contains("EXACT LIVE-CATALOG REPAIR DECLARATIONS"));
    assert!(prompt.contains("isRegex: bool"));
    assert!(prompt.contains("missing pin"));
    assert!(prompt.contains("Validation diagnostics (1 total)"));
    assert!(prompt.contains("draft_id=support-flow, revision=3"));

    let error =
        external_workflow_incomplete_error(Some(&snapshot), MAX_EXTERNAL_WORKFLOW_CONTINUATIONS);
    assert!(error.contains("without queueing changes"));
    assert!(error.contains("missing pin"));
    assert!(error.contains("draft_id=support-flow, revision=3"));
}

#[test]
fn incomplete_error_reports_every_budget_and_all_retained_diagnostics() {
    let snapshot = WorkflowToolLoopSnapshot {
        last_status: Some("validation_errors".to_string()),
        last_errors: (0..25)
            .map(|index| format!("diagnostic number {index}"))
            .collect(),
        last_structured_diagnostics: vec![serde_json::json!({
            "code": "FS_UNKNOWN_INPUT_PIN",
            "phase": "type_check",
            "message": "unknown pin `mail_subject`"
        })],
        edit_attempts: 2,
        flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
        stalled_edit_attempts: 1,
        flowscript_commit_attempts: 0,
        exhausted_budget: Some(format!(
            "FlowScript source operation budget ({}/{})",
            MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS, MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
        )),
        flowscript_draft_id: Some("mail-agent".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(16),
        ..Default::default()
    };

    let error = external_workflow_incomplete_error(Some(&snapshot), 2);
    assert!(error.contains("FlowScript source operation budget (24/24)"));
    assert!(error.contains("provider continuations 2/2"));
    assert!(error.contains("checks 2/12"));
    assert!(error.contains("source operations 24/24"));
    assert!(error.contains("stalled repeats 1/3"));
    assert!(error.contains("commit attempts 0/3"));
    assert!(error.contains("Remaining diagnostics (25 total)"));
    assert!(error.contains("diagnostic number 0"));
    assert!(error.contains("diagnostic number 19"));
    assert!(!error.contains("diagnostic number 20"));
    assert!(error.contains("(+5 more)"));
    assert!(error.contains("Structured diagnostics (1 retained)"));
    assert!(error.contains("FS_UNKNOWN_INPUT_PIN"));
    assert!(error.contains("draft_id=mail-agent, revision=16"));
    assert!(error.contains("same user request"));
}

#[test]
fn nested_wall_clock_budget_terminates_gracefully_through_the_incomplete_path() {
    assert!(!nested_wall_clock_exhausted(None));
    assert!(!nested_wall_clock_exhausted(Some(
        Instant::now() + Duration::from_secs(60)
    )));
    assert!(nested_wall_clock_exhausted(Some(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("test instant supports subtraction")
    )));

    let snapshot = WorkflowToolLoopSnapshot {
        last_status: Some("validation_errors".to_string()),
        last_errors: vec!["missing pin".to_string()],
        edit_attempts: 3,
        flowscript_draft_id: Some("uptime-monitor".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(11),
        ..Default::default()
    };
    let error = nested_wall_clock_incomplete_error(Some(&snapshot), 1);
    assert!(error.contains("NESTED_RUN_WALL_CLOCK_BUDGET_EXHAUSTED"));
    assert!(error.contains("wall-clock budget"));
    assert!(!error.contains("provider continuation budget"));
    assert!(error.contains("stopped gracefully"));
    assert!(error.contains("terminal for this run"));
    // The shared incomplete path keeps the retained draft coordinates and diagnostics so the
    // outer agent can resume the exact candidate instead of rebuilding.
    assert!(error.contains("draft_id=uptime-monitor, revision=11"));
    assert!(error.contains("missing pin"));
    assert!(error.contains("checks 3/12"));
    assert!(error.contains("12-minute wall-clock budget"));
}

#[test]
fn reports_use_effective_budgets_after_segmentation_and_earned_time() {
    let mut state = WorkflowToolLoopState {
        board_node_count: 404,
        granted_time_extensions: 2,
        edit_attempts: 8,
        flowscript_operation_attempts: 17,
        flowscript_commit_attempts: 2,
        last_status: Some("validation_errors".to_string()),
        last_errors: vec!["missing pin".to_string()],
        flowscript_draft_id: Some("character-repair".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(7),
        ..Default::default()
    };
    let mut plan = staged_plan_args(4);
    plan["strategy"] = serde_json::json!("incremental");
    state
        .accept_scope_plan_args(serde_json::from_value(plan).expect("valid scope plan"))
        .expect("accepted scope plan");
    let snapshot = state.snapshot();

    let error = nested_wall_clock_incomplete_error(Some(&snapshot), 3);
    assert!(error.contains("81-minute wall-clock budget"), "{error}");
    for expected in [
        "provider continuations 3/4",
        "checks 8/33",
        "source operations 17/66",
        "commit attempts 2/9",
        "draft_id=character-repair, revision=7",
        "missing pin",
    ] {
        assert!(error.contains(expected), "missing {expected}: {error}");
    }
    let continuation = build_external_workflow_continuation_prompt(
        "Repair the character workflow",
        Some(&snapshot),
        3,
    );
    assert!(continuation.contains("source operations: 17/66"));

    let summary = workflow_run_summary_payload(
        "incomplete",
        "codex",
        "test-model",
        81 * 60 * 1000,
        4,
        3,
        2,
        Some(&snapshot),
        0,
    );
    assert_eq!(summary["time_budget"]["limit_secs"], 81 * 60);
    assert_eq!(summary["budget"]["checks"]["limit"], 33);
    assert_eq!(summary["budget"]["source_ops"]["limit"], 66);
    assert_eq!(summary["budget"]["commits"]["limit"], 9);
    assert_eq!(summary["budget"]["continuations"]["limit"], 4);
    assert_eq!(summary["retained_draft"]["id"], "character-repair");
    assert_eq!(summary["retained_draft"]["revision"], 7);
}

#[test]
fn nested_wall_clock_budget_stays_below_the_outer_bridge_dispatch_bound() {
    use flow_like::flow::copilot::tool_spec::find_global_tool_spec;
    // flowpilot_board is the long-build delegation whose 30-minute bridge bound previously
    // outlived a hung nested run for a whole outer turn. flowpilot_widget's own 10-minute
    // bridge bound is tighter than this budget, and its frontend deadline path already
    // cancels the nested run explicitly at that bound.
    let spec = find_global_tool_spec("flowpilot_board").expect("flowpilot_board spec");
    assert!(
        NESTED_RUN_WALL_CLOCK_BUDGET < Duration::from_secs(spec.timeout_secs),
        "the nested wall-clock budget must expire before the outer flowpilot_board bridge deadline so the waiting agent receives a terminal result instead of a channel loss"
    );
}

#[test]
fn delegated_heartbeats_carry_nested_tool_and_budget_progress() {
    let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
        edit_attempts: 3,
        flowscript_operation_attempts: 9,
        flowscript_commit_attempts: 1,
        ..Default::default()
    }));
    record_delegated_run_tool_progress("patch_flowscript", 23, Some(&state));

    let base = "FlowPilot flowpilot_board is still running";
    let message = delegated_run_heartbeat_message(base);
    assert!(message.starts_with(base));
    assert!(message.contains("patch_flowscript"));
    assert!(message.contains("tool call 23"));
    assert!(message.contains(&format!("checks 3/{MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS}")));
    assert!(message.contains(&format!(
        "source operations 9/{MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS}"
    )));
    assert!(message.contains(&format!(
        "commit attempts 1/{MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS}"
    )));

    // The delegation tools themselves are the wait, not the nested progress.
    record_delegated_run_tool_progress("flowpilot_board", 99, None);
    let unchanged = delegated_run_heartbeat_message(base);
    assert!(unchanged.contains("patch_flowscript"));
    assert!(!unchanged.contains("tool call 99"));

    // Stale progress must not narrate a hung wait as movement.
    if let Ok(mut latest) = LATEST_DELEGATED_RUN_TOOL_PROGRESS.lock()
        && let Some((recorded_at, _)) = latest.as_mut()
    {
        *recorded_at = Instant::now()
            .checked_sub(DELEGATED_RUN_PROGRESS_FRESHNESS + Duration::from_secs(1))
            .expect("test instant supports freshness subtraction");
    }
    assert_eq!(delegated_run_heartbeat_message(base), base);
}

#[test]
fn ack_race_diagnostic_states_the_persisted_board_was_kept() {
    let released = flow_ir_ack_race_diagnostic(true);
    assert!(released.contains("applied and persisted"));
    assert!(released.contains("released"));
    let kept = flow_ir_ack_race_diagnostic(false);
    assert!(kept.contains("applied and persisted board was kept"));
    assert!(kept.contains("lost response channel"));
}

#[test]
fn continuation_prompt_marks_omitted_diagnostics() {
    let snapshot = WorkflowToolLoopSnapshot {
        last_status: Some("validation_errors".to_string()),
        last_errors: (0..23)
            .map(|index| format!("diagnostic number {index}"))
            .collect(),
        ..Default::default()
    };
    let prompt = build_external_workflow_continuation_prompt("build it", Some(&snapshot), 1);
    assert!(prompt.contains("Validation diagnostics (23 total)"));
    assert!(prompt.contains("diagnostic number 19"));
    assert!(!prompt.contains("diagnostic number 20"));
    assert!(prompt.contains("+3 more diagnostics omitted"));
}

#[test]
fn workflow_snapshot_carries_terminal_budget_state() {
    let stalled = WorkflowToolLoopState {
        stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
        flowscript_commit_attempts: 1,
        ..Default::default()
    };
    let snapshot = stalled.snapshot();
    assert_eq!(
        snapshot.stalled_edit_attempts,
        MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
    );
    assert_eq!(snapshot.flowscript_commit_attempts, 1);
    assert!(
        snapshot
            .exhausted_budget
            .as_deref()
            .is_some_and(|budget| budget.contains("stalled repair progress"))
    );

    let ops_exhausted = WorkflowToolLoopState {
        flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
        ..Default::default()
    };
    assert!(
        ops_exhausted
            .snapshot()
            .exhausted_budget
            .as_deref()
            .is_some_and(|budget| budget.contains("source operation budget"))
    );

    let queued = WorkflowToolLoopState {
        queued: true,
        flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
        ..Default::default()
    };
    assert_eq!(queued.snapshot().exhausted_budget, None);

    let healthy = WorkflowToolLoopState::default();
    assert_eq!(healthy.snapshot().exhausted_budget, None);
}

#[test]
fn run_summary_payload_is_built_from_a_populated_snapshot() {
    let state = WorkflowToolLoopState {
        queued: true,
        edit_attempts: 5,
        flowscript_operation_attempts: 9,
        stalled_edit_attempts: 1,
        flowscript_commit_attempts: 2,
        flowscript_draft_id: Some("draft-1".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(7),
        last_flowscript: Some("events {}".to_string()),
        last_review_notes: 3,
        last_structured_diagnostics: vec![
            serde_json::json!({ "code": "FS_TYPE_MISMATCH", "occurrences": 2 }),
            serde_json::json!({ "code": "FS_TYPE_MISMATCH" }),
            serde_json::json!({ "code": "FS_UNKNOWN_DECLARATION" }),
            serde_json::json!({ "message": "entry without a code is skipped" }),
        ],
        ..Default::default()
    };
    let snapshot = state.snapshot();
    assert_eq!(snapshot.last_review_notes, 3);

    let payload = workflow_run_summary_payload(
        "committed",
        "codex",
        "gpt-5",
        1_234,
        2,
        1,
        u32::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS),
        Some(&snapshot),
        6,
    );
    assert_eq!(payload["kind"], "run_summary");
    assert_eq!(payload["status"], "done");
    assert_eq!(payload["outcome"], "committed");
    assert_eq!(payload["provider"], "codex");
    assert_eq!(payload["model"], "gpt-5");
    assert_eq!(payload["duration_ms"], 1_234);
    assert_eq!(payload["phases"], 2);
    assert_eq!(payload["budget"]["checks"]["used"], 5);
    assert_eq!(
        payload["budget"]["checks"]["limit"],
        u64::from(MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS)
    );
    assert_eq!(payload["budget"]["source_ops"]["used"], 9);
    assert_eq!(
        payload["budget"]["source_ops"]["limit"],
        u64::from(MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS)
    );
    assert_eq!(payload["budget"]["commits"]["used"], 2);
    assert_eq!(payload["budget"]["stalled"]["used"], 1);
    assert_eq!(payload["budget"]["continuations"]["used"], 1);
    assert_eq!(
        payload["budget"]["continuations"]["limit"],
        u64::from(MAX_EXTERNAL_WORKFLOW_CONTINUATIONS)
    );
    assert_eq!(payload["diagnostics_by_code"]["FS_TYPE_MISMATCH"], 3);
    assert_eq!(payload["diagnostics_by_code"]["FS_UNKNOWN_DECLARATION"], 1);
    assert_eq!(
        payload["diagnostics_by_code"]
            .as_object()
            .map(serde_json::Map::len),
        Some(2)
    );
    assert_eq!(payload["retained_draft"]["id"], "draft-1");
    assert_eq!(payload["retained_draft"]["revision"], 7);
    assert_eq!(payload["review_notes"], 3);
    assert_eq!(payload["applied_commands"], 6);
    // The frame must stay invisible to the process-step UIs, which key on tool_call_id.
    assert!(payload.get("tool_call_id").is_none());

    let bare = workflow_run_summary_payload("provider_failure", "bits", "o4", 10, 1, 0, 2, None, 0);
    assert_eq!(bare["status"], "error");
    assert!(bare["retained_draft"].is_null());
    assert_eq!(bare["budget"]["checks"]["used"], 0);
    assert_eq!(bare["review_notes"], 0);
    assert_eq!(
        bare["diagnostics_by_code"]
            .as_object()
            .map(serde_json::Map::len),
        Some(0)
    );
}

#[test]
fn run_summary_review_notes_follow_the_latest_lifecycle_result() {
    let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
    workflow_tool_record(
        &state,
        "commit_flowscript",
        &serde_json::json!({ "draft_id": "draft-1" }),
        &serde_json::json!({
            "status": "queued",
            "review_notes": [
                { "code": "REVIEW_SECRET_PLACEHOLDER", "message": "fill the secret" },
                { "code": "REVIEW_DESTRUCTIVE", "message": "removes two nodes" },
            ],
        })
        .to_string(),
    );
    let snapshot = state.lock().expect("state lock").snapshot();
    assert_eq!(snapshot.last_review_notes, 2);
    assert!(snapshot.queued);

    // A follow-up result without the field keeps the last known count.
    workflow_tool_record(
        &state,
        "check_flowscript",
        &serde_json::json!({}),
        &serde_json::json!({ "status": "valid" }).to_string(),
    );
    let snapshot = state.lock().expect("state lock").snapshot();
    assert_eq!(snapshot.last_review_notes, 2);
}

#[test]
fn continuation_slice_makes_exhausted_budgets_executable_again() {
    let mut state = WorkflowToolLoopState {
        initial_declaration_lookup_complete: true,
        mutation_path: Some(WorkflowMutationPath::FlowScript),
        flowscript_draft_id: Some("mail-agent".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(4),
        stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
        flowscript_operation_attempts: MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS,
        edit_attempts: MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS,
        flowscript_commit_attempts: MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS,
        ..Default::default()
    };
    state
        .flowscript_seen_repair_signatures
        .insert("stale".to_string());
    assert!(state.exhausted_budget().is_some());

    state.grant_continuation_slice();
    assert_eq!(state.exhausted_budget(), None);
    assert_eq!(state.stalled_edit_attempts, 0);
    assert!(state.flowscript_seen_repair_signatures.is_empty());
    assert_eq!(
        state.flowscript_operation_attempts,
        MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS - EXTERNAL_CONTINUATION_OPERATION_HEADROOM
    );
    assert_eq!(
        state.edit_attempts,
        MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS - EXTERNAL_CONTINUATION_CHECK_HEADROOM
    );
    assert_eq!(
        state.flowscript_commit_attempts,
        MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS - 1
    );

    // A patch on the granted slice must be dispatchable, not refused-on-arrival.
    let shared = Arc::new(StdMutex::new(state));
    assert!(
        workflow_tool_preflight_with_args(
            &shared,
            "patch_flowscript",
            &serde_json::json!({ "draft_id": "mail-agent", "expected_revision": 4 }),
        )
        .is_none()
    );

    // Queued work never hands out another slice.
    let mut queued = WorkflowToolLoopState {
        queued: true,
        stalled_edit_attempts: MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS,
        ..Default::default()
    };
    queued.grant_continuation_slice();
    assert_eq!(
        queued.stalled_edit_attempts,
        MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
    );
}

#[test]
fn sdk_idle_continuation_grants_one_slice_then_stops_the_same_budget_honestly() {
    // UI-only sessions have no workflow loop state; continuations stay executable.
    assert_eq!(
        prepare_sdk_idle_continuation_budget(None, None),
        IdleContinuationBudget::Executable
    );

    let state = Arc::new(StdMutex::new(WorkflowToolLoopState::default()));
    assert_eq!(
        prepare_sdk_idle_continuation_budget(Some(&state), None),
        IdleContinuationBudget::Executable
    );

    state.lock().expect("state lock").edit_attempts = MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS;
    let granted = prepare_sdk_idle_continuation_budget(Some(&state), None);
    let IdleContinuationBudget::SliceGranted(budget) = granted else {
        panic!("expected a granted continuation slice, got {granted:?}");
    };
    assert!(budget.contains("check budget"), "{budget}");
    // The granted slice makes the continuation instructions executable on arrival.
    assert_eq!(state.lock().expect("state lock").exhausted_budget(), None);

    // The continuation burned its slice on the same budget: no second grant, stop honestly.
    state.lock().expect("state lock").edit_attempts = MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS;
    let terminal = prepare_sdk_idle_continuation_budget(Some(&state), Some(budget.as_str()));
    let IdleContinuationBudget::Terminal(reason) = terminal else {
        panic!("expected an honest terminal outcome, got {terminal:?}");
    };
    assert!(reason.contains(&budget), "{reason}");
    assert!(reason.contains("exhausted again"), "{reason}");
}

#[test]
fn parallel_flowscript_operations_are_refused_without_consuming_budget() {
    let state = Arc::new(StdMutex::new(WorkflowToolLoopState {
        initial_declaration_lookup_complete: true,
        mutation_path: Some(WorkflowMutationPath::FlowScript),
        flowscript_draft_id: Some("mail-agent".to_string()),
        flowscript_draft_retained: true,
        flowscript_revision: Some(2),
        flowscript_operation_attempts: 5,
        edit_attempts: 1,
        edit_in_flight: true,
        last_status: Some("valid".to_string()),
        ..Default::default()
    }));
    let args = serde_json::json!({
        "draft_id": "mail-agent",
        "expected_revision": 2,
        "source": "eventsSimple() { logInfo({ message: \"parallel\" }) }"
    });

    for tool in [
        "write_flowscript",
        "patch_flowscript",
        "check_flowscript",
        "commit_flowscript",
    ] {
        let refusal = workflow_tool_preflight_with_args(&state, tool, &args)
            .unwrap_or_else(|| panic!("{tool} must be refused while an edit is in flight"));
        assert_eq!(
            workflow_call_result_json(&refusal)["status"],
            "edit_in_flight",
            "{tool} refusal must be the cheap in-flight response"
        );
        let guard = state.lock().expect("state lock");
        assert_eq!(
            guard.flowscript_operation_attempts, 5,
            "{tool} refusal must not consume the source operation budget"
        );
        assert_eq!(
            guard.edit_attempts, 1,
            "{tool} refusal must not consume the check budget"
        );
        assert_eq!(guard.flowscript_commit_attempts, 0);
    }
}

#[test]
fn structured_diagnostic_retention_ranks_root_causes_and_marks_truncation() {
    let mut diagnostics = Vec::new();
    // Emission order intentionally lists validation noise first and the parse root cause last.
    for index in 0..MAX_RETAINED_STRUCTURED_DIAGNOSTICS {
        diagnostics.push(serde_json::json!({
            "code": "FS_EXECUTION_ENTRY_UNCONNECTED",
            "phase": "validation",
            "message": format!("validation cascade {index}")
        }));
    }
    diagnostics.push(serde_json::json!({
        "code": "FS_UNKNOWN_INPUT_PIN",
        "phase": "type_check",
        "message": "unknown pin `mail_subject`"
    }));
    diagnostics.push(serde_json::json!({
        "code": "FS_PARSE_ERROR",
        "phase": "parse",
        "message": "return value 1 is not a resolvable FlowScript value"
    }));
    let payload = serde_json::json!({ "diagnostics": diagnostics });

    let retained = workflow_result_structured_diagnostics(Some(&payload));
    assert_eq!(retained.len(), MAX_RETAINED_STRUCTURED_DIAGNOSTICS + 1);
    assert_eq!(retained[0]["phase"], "parse");
    assert_eq!(retained[1]["phase"], "type_check");
    assert_eq!(retained[2]["phase"], "validation");
    let sentinel = retained
        .last()
        .expect("truncated retention appends a sentinel");
    assert_eq!(sentinel["truncated"], true);
    assert_eq!(sentinel["omitted_count"], 2);

    // One oversized item is skipped instead of ending retention for smaller later items.
    let oversized = serde_json::json!({
        "diagnostics": [
            {
                "code": "FS_TYPE_MISMATCH",
                "phase": "type_check",
                "message": "x".repeat(MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES + 1)
            },
            {
                "code": "FS_PARSE_ERROR",
                "phase": "validation",
                "message": "small trailing diagnostic"
            }
        ]
    });
    let retained = workflow_result_structured_diagnostics(Some(&oversized));
    assert!(
        retained
            .iter()
            .any(|entry| entry["message"] == "small trailing diagnostic")
    );
    assert!(
        retained
            .iter()
            .any(|entry| entry["truncated"] == true && entry["omitted_count"] == 1)
    );
}

#[test]
fn unchanged_source_echo_is_replaced_with_a_retention_summary() {
    let source = "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n";

    // write_flowscript: byte-identical response source is not re-echoed.
    let mut write_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "draft_written",
            "draft_id": "mail-agent",
            "revision": 3,
            "source": source
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "write_flowscript",
        &serde_json::json!({ "draft_id": "mail-agent", "source": source }),
        &mut write_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&write_result.text_result_for_llm).expect("payload stays JSON");
    assert!(payload.get("source").is_none());
    let echo = payload["source_echo"].as_str().expect("summary line");
    assert!(echo.contains("revision 3"));
    assert!(echo.contains("3 lines"));
    assert!(echo.contains(&flowscript_source_fingerprint(source)));

    // Host-normalized writes keep the full echo.
    let mut normalized_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "draft_written",
            "revision": 0,
            "source": format!("{source}// host-added anchor\n")
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "write_flowscript",
        &serde_json::json!({ "source": source }),
        &mut normalized_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&normalized_result.text_result_for_llm).expect("payload is JSON");
    assert!(payload["source"].as_str().is_some());

    // An initial write made from edits returns the host's merged source for the first time.
    let mut edited_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "draft_written",
            "draft_id": "mail-agent",
            "revision": 0,
            "source": source
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "write_flowscript",
        &serde_json::json!({
            "draft_id": "mail-agent",
            "edits": [{ "old_text": "hello before", "new_text": "hello" }]
        }),
        &mut edited_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&edited_result.text_result_for_llm).expect("payload is JSON");
    assert_eq!(payload["source"], source);
    assert!(payload.get("source_echo").is_none());

    // check_flowscript on the expected revision cannot change the source.
    let mut check_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "valid",
            "draft_id": "mail-agent",
            "revision": 3,
            "source": source
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "check_flowscript",
        &serde_json::json!({ "draft_id": "mail-agent", "expected_revision": 3 }),
        &mut check_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&check_result.text_result_for_llm).expect("payload is JSON");
    assert!(payload.get("source").is_none());
    assert_eq!(payload["status"], "valid");

    // A revision conflict returns a source the model has not seen; keep it.
    let mut conflict_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "error",
            "code": "FLOWSCRIPT_REVISION_CONFLICT",
            "revision": 4,
            "source": source
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "check_flowscript",
        &serde_json::json!({ "expected_revision": 3 }),
        &mut conflict_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&conflict_result.text_result_for_llm).expect("payload is JSON");
    assert!(payload["source"].as_str().is_some());

    // patch_flowscript results are host-computed merges and always keep the echo.
    let mut patch_result = copilot_sdk::ToolResultObject::text(
        serde_json::json!({
            "status": "draft_patched",
            "revision": 4,
            "source": source
        })
        .to_string(),
    );
    suppress_unchanged_flowscript_source_echo(
        "patch_flowscript",
        &serde_json::json!({ "expected_revision": 4 }),
        &mut patch_result,
    );
    let payload: serde_json::Value =
        serde_json::from_str(&patch_result.text_result_for_llm).expect("payload is JSON");
    assert!(payload["source"].as_str().is_some());
}
