use super::*;

#[test]
fn nested_runs_in_one_conversation_share_request_identity() {
    let outer_prompt = "yes, build it";
    let context = FrontendToolContext {
        conversation_id: Some("conversation-1".to_string()),
        source_user_prompt: Some(outer_prompt.to_string()),
        ..Default::default()
    };

    let first_nested = request_identity_prompt_for(
        Some(&context),
        "Execute the change NOW: build the intake workflow.",
    );
    let repair_nested = request_identity_prompt_for(
        Some(&context),
        "Repair the retained draft draft-1 at revision 3.",
    );

    assert_eq!(first_nested, repair_nested);
    assert_eq!(first_nested, format!("conversation-1\n{outer_prompt}"));
}

#[test]
fn identical_prompts_in_different_conversations_never_share_identity() {
    let outer_prompt = "yes, build it";
    let conversation = |id: &str| FrontendToolContext {
        conversation_id: Some(id.to_string()),
        source_user_prompt: Some(outer_prompt.to_string()),
        ..Default::default()
    };

    assert_ne!(
        request_identity_prompt_for(Some(&conversation("conversation-a")), outer_prompt),
        request_identity_prompt_for(Some(&conversation("conversation-b")), outer_prompt),
    );
}

#[test]
fn request_identity_falls_back_to_raw_prompt_without_conversation_scope() {
    assert_eq!(
        request_identity_prompt_for(None, "add a logging node"),
        "add a logging node"
    );
    assert_eq!(
        request_identity_prompt_for(Some(&FrontendToolContext::default()), "add a logging node"),
        "add a logging node"
    );
    let conversation_only = FrontendToolContext {
        conversation_id: Some("conversation-1".to_string()),
        ..Default::default()
    };
    assert_eq!(
        request_identity_prompt_for(Some(&conversation_only), "add a logging node"),
        "conversation-1\nadd a logging node"
    );
    let blank_scope = FrontendToolContext {
        conversation_id: Some("   ".to_string()),
        source_user_prompt: Some("  ".to_string()),
        ..Default::default()
    };
    assert_eq!(
        request_identity_prompt_for(Some(&blank_scope), "add a logging node"),
        "add a logging node"
    );
}

#[test]
fn desktop_recovery_injects_only_exact_request_coordinates() {
    let exact = flow_like::flow::copilot::FlowIrDraftRecovery {
        status: flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch,
        auto_resume: true,
        exact_match: Some(retained_recovery_context("exact-draft", 7)),
        conflicting_draft: None,
        next_actions: vec!["resume_exact_draft".to_string()],
        message: "exact request".to_string(),
    };
    let mut exact_prompt = String::new();
    append_typed_ir_recovery_context(&mut exact_prompt, &exact);
    assert!(exact_prompt.contains("EXACT TYPED-DRAFT RECOVERY"));
    assert!(exact_prompt.contains("exact-draft"));
    assert!(exact_prompt.contains("\"revision\": 7"));

    let mismatch = flow_like::flow::copilot::FlowIrDraftRecovery {
        status: flow_like::flow::copilot::FlowIrDraftRecoveryStatus::RequestMismatch,
        auto_resume: false,
        exact_match: None,
        conflicting_draft: Some(retained_recovery_context("secret-old-draft", 11)),
        next_actions: vec![
            "recover_with_original_request".to_string(),
            "abandon_retained_draft_via_host".to_string(),
        ],
        message: "different immutable request".to_string(),
    };
    let mut mismatch_prompt = String::new();
    append_typed_ir_recovery_context(&mut mismatch_prompt, &mismatch);
    assert!(mismatch_prompt.contains("TYPED-DRAFT REQUEST MISMATCH"));
    assert!(mismatch_prompt.contains("abandon_retained_draft_via_host"));
    assert!(mismatch_prompt.contains("\"auto_resume\": false"));
    assert!(!mismatch_prompt.contains("secret-old-draft"));
    assert!(!mismatch_prompt.contains("\"revision\""));
}

#[test]
fn desktop_source_recovery_resumes_exact_request_and_hides_mismatches() {
    let board = flowscript_recovery_test_board();
    let request = "Build a durable customer-support logging workflow.";
    let draft_id = format!("source-draft-{}", uuid::Uuid::new_v4());
    let source = "function retainedRecoveryMarker() {\n    missingCatalogCall()\n}\n";
    let store = retained_flow_ir_draft_store_for_board(&board)
        .expect("desktop source recovery should acquire the board-scoped store");
    let binding = store.bind_request_acceptance_contract(&board.id, request);
    let written = store.write_flowscript_with_acceptance_binding(
        &board,
        &[],
        flow_like::flow::copilot::WriteFlowScriptArgs {
            draft_id: draft_id.clone(),
            replace_existing: false,
            mode: flow_like::flow::copilot::FlowIrDraftMode::Additive,
            source: source.to_string(),
            edits: Vec::new(),
            allow_scope_reduction: false,
        },
        &binding,
    );
    assert_eq!(written.revision, Some(0));

    let mut exact_prompt = String::new();
    append_flowscript_recovery_context(&mut exact_prompt, &board, request);
    assert!(exact_prompt.contains("EXACT RETAINED FLOWSCRIPT RECOVERY"));
    assert!(exact_prompt.contains(&draft_id));
    assert!(exact_prompt.contains("retainedRecoveryMarker"));
    assert!(exact_prompt.contains("revision: `0`"));

    let mut mismatch_prompt = String::new();
    append_flowscript_recovery_context(
        &mut mismatch_prompt,
        &board,
        "Build an unrelated invoice workflow.",
    );
    assert!(mismatch_prompt.contains("FLOWSCRIPT REQUEST MISMATCH"));
    assert!(mismatch_prompt.contains("source is intentionally hidden"));
    assert!(!mismatch_prompt.contains(&draft_id));
    assert!(!mismatch_prompt.contains("retainedRecoveryMarker"));

    let mut advanced_board = board.clone();
    let variable = flow_like::flow::variable::Variable::new(
        "board_changed_after_timeout",
        flow_like::flow::variable::VariableType::String,
        flow_like::flow::pin::ValueType::Normal,
    );
    advanced_board
        .variables
        .insert(variable.id.clone(), variable);
    let mut stale_prompt = String::new();
    append_flowscript_recovery_context(&mut stale_prompt, &advanced_board, request);
    assert!(stale_prompt.contains("STALE RETAINED FLOWSCRIPT"));
    assert!(stale_prompt.contains("retainedRecoveryMarker"));
    assert!(stale_prompt.contains("fresh draft_id"));
}

#[test]
fn desktop_pending_source_redelivery_preserves_the_exact_review_payload() {
    let commands = vec![BoardCommand::RemoveNode {
        node_id: "exact-redelivery-node".to_string(),
        summary: None,
    }];
    let token = FlowIrCommitToken {
        board_id: "redelivery-board".to_string(),
        draft_id: "redelivery-draft".to_string(),
        revision: 4,
        base_fingerprint: "redelivery-base".to_string(),
        claim_id: "redelivery-claim".to_string(),
        requires_destructive_approval: true,
    };
    let response = pending_flowscript_redelivery_response(
        CopilotScope::Board,
        FlowScriptPendingDelivery {
            source: "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n".to_string(),
            token: token.clone(),
            stale_board: false,
            commands: commands.clone(),
        },
    );

    assert_eq!(
        serde_json::to_value(&response.commands).unwrap(),
        serde_json::to_value(&commands).unwrap()
    );
    assert_eq!(response.flow_ir_commit, Some(token));
    assert_eq!(response.active_scope, CopilotScope::Board);
    let workspace: serde_json::Value = serde_json::from_str(
        response
            .flowscript_workspace
            .as_deref()
            .expect("redelivery includes the exact source workspace"),
    )
    .expect("workspace is valid JSON");
    assert_eq!(workspace["status"], "queued");
    assert!(workspace["source"].as_str().unwrap().contains("logInfo"));
    assert!(response.message.contains("No model generation"));

    let stale = pending_flowscript_redelivery_response(
        CopilotScope::Board,
        FlowScriptPendingDelivery {
            source: "eventsSimple() {}\n".to_string(),
            token: response.flow_ir_commit.clone().unwrap(),
            stale_board: true,
            commands: Vec::new(),
        },
    );
    assert!(stale.commands.is_empty());
    let stale_workspace: serde_json::Value = serde_json::from_str(
        stale
            .flowscript_workspace
            .as_deref()
            .expect("stale redelivery includes retained source"),
    )
    .expect("stale workspace is valid JSON");
    assert_eq!(stale_workspace["status"], "stale");
    assert!(stale.message.contains("dismiss this stale review"));
}
