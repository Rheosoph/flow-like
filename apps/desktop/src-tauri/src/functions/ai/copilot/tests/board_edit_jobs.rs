use super::super::{
    board_commits::flow_ir_durable_receipt_ref_key,
    board_jobs::{board_edit_job_dismissal_receipt, recover_board_edit_job_dismissal_receipt},
};
use super::*;

#[test]
fn board_edit_review_is_deterministic_and_host_derived() {
    let review = board_command_review(
        &[
            BoardCommand::AddNode {
                node_type: "events_generic".to_string(),
                ref_id: Some("event".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: Some("  Add the entry event.  ".to_string()),
            },
            BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("log".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::RemoveNode {
                node_id: "old-node".to_string(),
                summary: Some("Remove the obsolete node.".to_string()),
            },
        ],
        false,
    );

    assert_eq!(review.command_count, 3);
    assert_eq!(review.command_counts.get("AddNode"), Some(&2));
    assert_eq!(review.command_counts.get("RemoveNode"), Some(&1));
    assert_eq!(
        review.command_summaries,
        vec![
            "Add the entry event.".to_string(),
            "Remove the obsolete node.".to_string(),
        ]
    );
    assert_eq!(
        review.destructive_effects,
        vec!["node `old-node`".to_string()]
    );
}

#[test]
fn terminal_board_edit_identity_blocks_stale_direct_receipt_replay() {
    let now = Instant::now();
    let terminal = board_edit_job_test_record("terminal", BoardEditJobPhase::Applied, now);
    assert!(board_edit_job_matches_terminal_delivery(
        &terminal,
        &terminal.job.app_id,
        &terminal.job.token,
    ));

    let mut pending = terminal.clone();
    pending.job.phase = BoardEditJobPhase::AppliedPendingDelivery;
    assert!(!board_edit_job_matches_terminal_delivery(
        &pending,
        &pending.job.app_id,
        &pending.job.token,
    ));

    let mut other = terminal.job.token.clone();
    other.claim_id.push_str("-other");
    assert!(!board_edit_job_matches_terminal_delivery(
        &terminal,
        &terminal.job.app_id,
        &other,
    ));
}

#[test]
fn board_edit_job_pruning_expires_reviews_and_preserves_the_exact_capacity() {
    let now = Instant::now();
    let expired_at = now
        .checked_sub(BOARD_EDIT_JOB_TTL + Duration::from_secs(1))
        .expect("test instant supports TTL subtraction");
    let mut jobs = HashMap::from([(
        "expired".to_string(),
        board_edit_job_test_record("expired", BoardEditJobPhase::AwaitingApproval, expired_at),
    )]);
    assert!(prune_board_edit_jobs(&mut jobs, now));
    let expired = &jobs["expired"].job;
    assert_eq!(expired.phase, BoardEditJobPhase::Stale);
    assert!(
        expired
            .error
            .as_deref()
            .is_some_and(|error| error.contains("expired"))
    );

    let mut at_capacity = (0..BOARD_EDIT_JOB_MAX_ENTRIES)
        .map(|index| {
            let id = format!("settled-{index:03}");
            (
                id.clone(),
                board_edit_job_test_record(id, BoardEditJobPhase::Applied, now),
            )
        })
        .collect::<HashMap<_, _>>();
    assert!(!prune_board_edit_jobs(&mut at_capacity, now));
    assert_eq!(at_capacity.len(), BOARD_EDIT_JOB_MAX_ENTRIES);

    at_capacity.insert(
        "old-terminal".to_string(),
        board_edit_job_test_record(
            "old-terminal",
            BoardEditJobPhase::Denied,
            now.checked_sub(Duration::from_secs(1))
                .expect("test instant supports subtraction"),
        ),
    );
    assert!(prune_board_edit_jobs(&mut at_capacity, now));
    assert_eq!(at_capacity.len(), BOARD_EDIT_JOB_MAX_ENTRIES);
    assert!(!at_capacity.contains_key("old-terminal"));
}

#[test]
fn failed_board_edit_jobs_remain_retryable_until_the_review_ttl() {
    let now = Instant::now();
    let mut jobs = HashMap::from([(
        "retryable".to_string(),
        board_edit_job_test_record("retryable", BoardEditJobPhase::Failed, now),
    )]);

    assert!(!prune_board_edit_jobs(&mut jobs, now));
    assert_eq!(jobs["retryable"].job.phase, BoardEditJobPhase::Failed);
    assert!(!jobs["retryable"].job.phase.is_terminal());

    let expired_at = now
        .checked_sub(BOARD_EDIT_JOB_TTL + Duration::from_secs(1))
        .expect("test instant supports TTL subtraction");
    jobs.get_mut("retryable").expect("retryable job").touched_at = expired_at;
    assert!(prune_board_edit_jobs(&mut jobs, now));
    assert_eq!(jobs["retryable"].job.phase, BoardEditJobPhase::Stale);
}

#[test]
fn applied_receipt_stays_recoverable_until_renderer_delivery_is_acknowledged() {
    let now = Instant::now();
    let older_than_review_ttl = now
        .checked_sub(BOARD_EDIT_JOB_TTL + Duration::from_secs(1))
        .expect("test instant supports TTL subtraction");
    let mut record = board_edit_job_test_record(
        "delivery",
        BoardEditJobPhase::AppliedPendingDelivery,
        older_than_review_ttl,
    );
    record.delivery_lease = Some(BoardEditJobDeliveryLease {
        lease_id: "abandoned-renderer".to_string(),
        expires_at: now
            .checked_sub(Duration::from_millis(1))
            .expect("test instant supports subtraction"),
    });
    let mut jobs = HashMap::from([("delivery".to_string(), record)]);

    assert!(!prune_board_edit_jobs(&mut jobs, now));

    let retained = &jobs["delivery"];
    assert_eq!(
        retained.job.phase,
        BoardEditJobPhase::AppliedPendingDelivery
    );
    assert!(!retained.job.phase.is_terminal());
    assert!(retained.delivery_lease.is_none());
}

#[test]
fn board_edit_job_commit_identity_is_exact_and_claim_scoped() {
    let token = FlowIrCommitToken {
        board_id: "board".to_string(),
        draft_id: "draft".to_string(),
        revision: 7,
        base_fingerprint: "base".to_string(),
        claim_id: "claim-a".to_string(),
        requires_destructive_approval: false,
    };
    assert_eq!(
        flow_ir_commit_identity(&token),
        flow_ir_commit_identity(&token.clone())
    );

    let mut next_claim = token.clone();
    next_claim.claim_id = "claim-b".to_string();
    assert_ne!(
        flow_ir_commit_identity(&token),
        flow_ir_commit_identity(&next_claim)
    );
    let mut next_revision = token.clone();
    next_revision.revision += 1;
    assert_ne!(
        flow_ir_commit_identity(&token),
        flow_ir_commit_identity(&next_revision)
    );
}

#[test]
fn board_edit_job_serializes_the_frontend_contract() {
    let mut record = board_edit_job_test_record(
        "wire-contract",
        BoardEditJobPhase::AwaitingApproval,
        Instant::now(),
    );
    record.job.review.command_count = 2;
    record
        .job
        .review
        .command_counts
        .insert("AddNode".to_string(), 2);
    let value = serde_json::to_value(record.job).expect("serialize board-edit job");

    assert_eq!(value["schemaVersion"], BOARD_EDIT_JOB_SCHEMA_VERSION);
    assert_eq!(value["jobId"], "wire-contract");
    assert_eq!(value["phase"], "awaiting_approval");
    assert_eq!(value["review"]["commandCount"], 2);
    assert_eq!(value["review"]["commandCounts"]["AddNode"], 2);
    assert_eq!(value["token"]["board_id"], "review-board");
    assert!(value.get("result").is_none());
    assert!(value.get("error").is_none());
}

#[test]
fn actual_executed_receipt_is_bounded_before_remote_persistence() {
    let mut command = flow_like::flow::board::commands::nodes::copy_paste::CopyPasteCommand::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        (0.0, 0.0, 0.0),
    );
    command.original_refs.insert(
        "large-public-ref".to_string(),
        "x".repeat(BOARD_EDIT_JOB_MAX_REMOTE_COMMAND_BYTES),
    );
    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: "applied".to_string(),
        commands: vec![GenericCommand::CopyPaste(command)],
        board_commands: Vec::new(),
        diagnostics: Vec::new(),
        final_board_node_count: Some(0),
        persisted_board_fingerprint: Some("flowpilot-board-v1:test".to_string()),
    };

    let error = validate_board_edit_delivery_bounds(&result, true)
        .expect_err("an unsendable executed command must fail before board save");
    assert!(error.contains("delivery limit"), "{error}");
}

#[test]
fn aggregate_executed_receipt_must_fit_one_atomic_remote_request() {
    let commands = (0..2)
        .map(|index| {
            let mut command =
                flow_like::flow::board::commands::nodes::copy_paste::CopyPasteCommand::new(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    (0.0, 0.0, 0.0),
                );
            command.original_refs.insert(
                format!("large-public-ref-{index}"),
                "x".repeat(BOARD_EDIT_JOB_MAX_REMOTE_COMMAND_BYTES / 2),
            );
            GenericCommand::CopyPaste(command)
        })
        .collect();
    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: "applied".to_string(),
        commands,
        board_commands: Vec::new(),
        diagnostics: Vec::new(),
        final_board_node_count: Some(0),
        persisted_board_fingerprint: Some("flowpilot-board-v1:test".to_string()),
    };

    let error = validate_board_edit_delivery_bounds(&result, true)
        .expect_err("individually small commands must not be split across board saves");
    assert!(error.contains("atomic executed command batch"), "{error}");
}

#[test]
fn durable_receipt_drops_redundant_compiler_commands() {
    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: "applied".to_string(),
        commands: Vec::new(),
        board_commands: vec![BoardCommand::RemoveNode {
            node_id: "old-node".to_string(),
            summary: None,
        }],
        diagnostics: Vec::new(),
        final_board_node_count: Some(0),
        persisted_board_fingerprint: Some("flowpilot-board-v1:test".to_string()),
    };

    let compact = compact_durable_apply_receipt(&result);
    assert!(compact.board_commands.is_empty());
    assert_eq!(compact.commands.len(), result.commands.len());
    assert_eq!(
        compact.persisted_board_fingerprint,
        result.persisted_board_fingerprint
    );
}

#[test]
fn board_edit_job_snapshot_round_trips_the_exact_host_batch_and_policy() {
    let mut record = board_edit_job_test_record(
        "durable-batch",
        BoardEditJobPhase::AwaitingApproval,
        Instant::now(),
    );
    record.replacement_mode = true;
    record.job.review.replacement_mode = true;
    let source = "// Preserve the authored source, including comments.\nevent start() {}\n";
    record.job.flowscript_source = Some(source.to_string());
    let reviewed_commands = record.board_commands.clone();
    let encoded = serde_json::to_vec(&PersistedBoardEditJobEntry::Current(
        PersistedBoardEditJobRecord {
            job: record.job,
            board_commands: record.board_commands,
            replacement_mode: record.replacement_mode,
        },
    ))
    .expect("serialize persisted board-edit job");
    let persisted: PersistedBoardEditJobEntry =
        serde_json::from_slice(&encoded).expect("deserialize persisted board-edit job");
    let recovered = board_edit_job_record_from_persisted(persisted)
        .expect("persisted exact batch remains recoverable");

    assert_eq!(recovered.job.phase, BoardEditJobPhase::AwaitingApproval);
    assert!(recovered.replacement_mode);
    assert_eq!(recovered.job.review.command_count, reviewed_commands.len());
    assert!(recovered.job.review.replacement_mode);
    assert_eq!(recovered.job.flowscript_source.as_deref(), Some(source));
    assert!(exact_board_command_batch_matches(
        &reviewed_commands,
        &recovered.board_commands
    ));
}

#[test]
fn board_edit_job_snapshot_without_source_remains_recoverable() {
    let record = board_edit_job_test_record(
        "older-review",
        BoardEditJobPhase::AwaitingApproval,
        Instant::now(),
    );
    let value = serde_json::to_value(PersistedBoardEditJobRecord {
        job: record.job,
        board_commands: record.board_commands,
        replacement_mode: record.replacement_mode,
    })
    .unwrap();
    assert!(value["job"].get("flowscriptSource").is_none());
    let recovered = board_edit_job_record_from_persisted(serde_json::from_value(value).unwrap())
        .expect("existing reviews remain recoverable without authored source");
    assert_eq!(recovered.job.phase, BoardEditJobPhase::AwaitingApproval);
    assert!(recovered.job.flowscript_source.is_none());
    assert!(!recovered.board_commands.is_empty());
}

#[test]
fn legacy_board_edit_job_without_an_exact_batch_fails_closed_after_restart() {
    let legacy = board_edit_job_test_record(
        "legacy-batch",
        BoardEditJobPhase::AwaitingApproval,
        Instant::now(),
    )
    .job;
    let recovered =
        board_edit_job_record_from_persisted(PersistedBoardEditJobEntry::Legacy(legacy))
            .expect("legacy job remains visible for an actionable stale result");

    assert_eq!(recovered.job.phase, BoardEditJobPhase::Stale);
    assert!(recovered.board_commands.is_empty());
    assert!(
        recovered
            .job
            .error
            .as_deref()
            .is_some_and(|message| message.contains("cannot be applied safely"))
    );
}

#[test]
fn interrupted_apply_restarts_as_retryable_failed_with_the_exact_batch() {
    let record = board_edit_job_test_record(
        "interrupted-apply",
        BoardEditJobPhase::Applying,
        Instant::now(),
    );
    let reviewed_commands = record.board_commands.clone();
    let recovered = board_edit_job_record_from_persisted(PersistedBoardEditJobEntry::Current(
        PersistedBoardEditJobRecord {
            job: record.job,
            board_commands: record.board_commands,
            replacement_mode: record.replacement_mode,
        },
    ))
    .expect("interrupted exact apply remains retryable");

    assert_eq!(recovered.job.phase, BoardEditJobPhase::Failed);
    assert!(exact_board_command_batch_matches(
        &reviewed_commands,
        &recovered.board_commands
    ));
    assert!(
        recovered
            .job
            .error
            .as_deref()
            .is_some_and(|message| message.contains("restarted"))
    );
}

#[tokio::test]
async fn dismissal_recovers_a_committed_receipt_after_an_interrupted_apply_restarts() {
    use flow_like::flow_like_storage::object_store::memory::InMemory;
    use flow_like_types::FromProto;

    let record = board_edit_job_test_record(
        "dismiss-after-restart",
        BoardEditJobPhase::Applying,
        Instant::now(),
    );
    let mut board =
        Board::new_detached(Some(record.job.board_id.clone()), "apps/review-app".into());
    let receipt = compact_job_test_receipt();
    board
        .insert_internal_ref(
            flow_ir_durable_receipt_ref_key(&record.job.app_id, &record.job.token),
            serde_json::json!({
                "version": 1,
                "created_at_ms": 1,
                "identity": flow_ir_applied_receipt_key(&record.job.app_id, &record.job.token),
                "result": compact_durable_apply_receipt(&receipt),
            })
            .to_string(),
        )
        .unwrap();
    let store = Arc::new(InMemory::new());
    board.save(Some(store.clone())).await.unwrap();
    let saved = Board::from_proto(
        Board::load_proto(store, &board.board_dir, &board.id, None)
            .await
            .unwrap(),
    );
    let recovered = board_edit_job_record_from_persisted(PersistedBoardEditJobEntry::Current(
        PersistedBoardEditJobRecord {
            job: record.job,
            board_commands: record.board_commands,
            replacement_mode: false,
        },
    ))
    .unwrap();
    assert_eq!(recovered.job.phase, BoardEditJobPhase::Failed);

    let pending_receipt = board_edit_job_dismissal_receipt(&recovered.job, &saved)
        .expect("a committed edit must finish receipt delivery when dismissed");
    assert_eq!(pending_receipt.status, "applied");
    assert!(pending_receipt.replayed);
    assert_eq!(pending_receipt.commands.len(), receipt.commands.len());
    assert_eq!(
        pending_receipt.persisted_board_fingerprint,
        receipt.persisted_board_fingerprint
    );

    let mut unrelated = recovered.job.clone();
    unrelated.token.claim_id.push_str("-other");
    assert!(board_edit_job_dismissal_receipt(&unrelated, &saved).is_none());
}

#[test]
fn failed_review_without_a_committed_receipt_can_still_be_dismissed() {
    let record = board_edit_job_test_record(
        "dismiss-uncommitted",
        BoardEditJobPhase::Failed,
        Instant::now(),
    );
    let board = Board::new_detached(Some(record.job.board_id.clone()), "apps/review-app".into());
    assert!(board_edit_job_dismissal_receipt(&record.job, &board).is_none());
}

#[tokio::test]
async fn dismissal_checks_the_saved_receipt_after_the_live_draft_was_discarded() {
    use flow_like::flow_like_storage::object_store::memory::InMemory;
    use flow_like::{
        flow::{
            board::{BoardCell, BoardParent},
            node::Node,
        },
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::FromProto;

    let mut record = board_edit_job_test_record(
        "dismiss-save-failed",
        BoardEditJobPhase::Failed,
        Instant::now(),
    );
    record.job.token.claim_id = uuid::Uuid::new_v4().to_string();
    record
        .job
        .record_apply_result(ApplyFlowIrCommitResult::empty(
            "error",
            "IR_COMMIT_SAVE_FAILED",
            "Saving and restoring the board both failed.",
        ));
    let mut live = Board::new_detached(Some(record.job.board_id.clone()), "apps/review-app".into());
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    live.app_state = Some(state.clone());
    live.parent = Some(BoardParent::App(std::sync::Weak::new()));
    let mut saved = live.clone();
    let saved_node = Node::new("saved_node", "Saved node", "", "test");
    let saved_node_id = saved_node.id.clone();
    saved.nodes.insert(saved_node_id.clone(), saved_node);
    let receipt = compact_job_test_receipt();
    saved
        .insert_internal_ref(
            flow_ir_durable_receipt_ref_key(&record.job.app_id, &record.job.token),
            serde_json::json!({
                "version": 1,
                "created_at_ms": 1,
                "identity": flow_ir_applied_receipt_key(&record.job.app_id, &record.job.token),
                "result": compact_durable_apply_receipt(&receipt),
            })
            .to_string(),
        )
        .unwrap();
    let store = Arc::new(InMemory::new());
    saved.save(Some(store.clone())).await.unwrap();
    assert!(board_edit_job_dismissal_receipt(&record.job, &live).is_none());
    let page_source = live.page_metadata_source();
    let board_dir = live.board_dir.clone();
    let live = BoardCell::new(live);
    let original = live.snapshot();

    let recovered = recover_board_edit_job_dismissal_receipt(&record.job, Some(&live), async {
        Board::load_proto(store, &saved.board_dir, &saved.id, None)
            .await
            .map(Board::from_proto)
            .map_err(|error| error.to_string())
    })
    .await
    .unwrap()
    .expect("a saved receipt must survive discarding the live draft");
    assert_eq!(recovered.status, "applied");
    assert!(recovered.replayed);
    assert_eq!(recovered.commands.len(), receipt.commands.len());
    assert_eq!(
        recovered.persisted_board_fingerprint,
        receipt.persisted_board_fingerprint
    );
    // The graph is published before the resolver can offer this receipt for acknowledgement.
    let restored = live.snapshot();
    assert!(restored.nodes.contains_key(&saved_node_id));
    assert!(!original.nodes.contains_key(&saved_node_id));
    assert_eq!(restored.board_dir, board_dir);
    assert_eq!(restored.page_metadata_source(), page_source);
    assert!(matches!(restored.parent, Some(BoardParent::App(_))));
    assert!(Arc::ptr_eq(restored.app_state.as_ref().unwrap(), &state));
    assert!(board_edit_job_dismissal_receipt(&record.job, &restored).is_some());
    let replay = replay_flow_ir_applied_receipt(&record.job.app_id, &record.job.token)
        .expect("receipt delivery can replay the saved result after restoring the live board");
    assert_eq!(replay.commands.len(), receipt.commands.len());

    FLOW_IR_APPLIED_RECEIPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&flow_ir_applied_receipt_key(
            &record.job.app_id,
            &record.job.token,
        ));
}

#[tokio::test]
async fn uncertain_dismissal_requires_a_readable_saved_board() {
    let mut record = board_edit_job_test_record(
        "dismiss-unreadable-board",
        BoardEditJobPhase::Failed,
        Instant::now(),
    );
    record.job.token.claim_id = uuid::Uuid::new_v4().to_string();
    let live = Board::new_detached(Some(record.job.board_id.clone()), "apps/review-app".into());
    let live = flow_like::flow::board::BoardCell::new(live);
    let result = recover_board_edit_job_dismissal_receipt(&record.job, Some(&live), async {
        Err("The saved board is unavailable.".to_string())
    })
    .await;
    assert_eq!(
        result.err().as_deref(),
        Some("The saved board is unavailable.")
    );
}

#[tokio::test]
async fn unapplied_failures_can_be_dismissed_without_loading_a_saved_board() {
    for code in [
        "IR_COMMIT_PERSISTENCE_UNAVAILABLE",
        "IR_COMMIT_CATALOG_UNAVAILABLE",
        "IR_COMMIT_APP_UNAVAILABLE",
        "IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
        "IR_COMMIT_APPLY_FAILED",
        "IR_COMMIT_FINGERPRINT_FAILED",
        "IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
    ] {
        let mut record = board_edit_job_test_record(
            "dismiss-unapplied",
            BoardEditJobPhase::Failed,
            Instant::now(),
        );
        record.job.token.claim_id = uuid::Uuid::new_v4().to_string();
        record
            .job
            .record_apply_result(ApplyFlowIrCommitResult::empty(
                "error",
                code,
                "Nothing was persisted.",
            ));
        let receipt = recover_board_edit_job_dismissal_receipt(&record.job, None, async {
            panic!("{code} must not need a readable saved board")
        })
        .await
        .unwrap();
        assert!(receipt.is_none());
    }
}

#[test]
fn board_mutation_gate_reserves_matching_apply_recovery_and_delivery_phases() {
    let now = Instant::now();
    let jobs = HashMap::from([
        (
            "awaiting".to_string(),
            board_edit_job_test_record("awaiting", BoardEditJobPhase::AwaitingApproval, now),
        ),
        (
            "applying".to_string(),
            board_edit_job_test_record("applying", BoardEditJobPhase::Applying, now),
        ),
    ]);

    assert!(board_mutation_is_reserved(
        &jobs,
        "review-app",
        "review-board"
    ));
    assert!(!board_mutation_is_reserved(
        &jobs,
        "another-app",
        "review-board"
    ));
    assert!(another_board_edit_job_reserves_mutation(
        &jobs,
        "awaiting",
        "review-app",
        "review-board"
    ));
    assert!(!another_board_edit_job_reserves_mutation(
        &jobs,
        "applying",
        "review-app",
        "review-board"
    ));
    assert!(!board_mutation_is_reserved(
        &jobs,
        "review-app",
        "another-board"
    ));

    let delivery = HashMap::from([(
        "delivery".to_string(),
        board_edit_job_test_record("delivery", BoardEditJobPhase::AppliedPendingDelivery, now),
    )]);
    assert!(board_mutation_is_reserved(
        &delivery,
        "review-app",
        "review-board"
    ));
    let failed = HashMap::from([(
        "failed".to_string(),
        board_edit_job_test_record("failed", BoardEditJobPhase::Failed, now),
    )]);
    assert!(board_mutation_is_reserved(
        &failed,
        "review-app",
        "review-board"
    ));
}

#[test]
fn known_unapplied_failures_allow_manual_edits_and_corrected_reviews() {
    for code in [
        "IR_COMMIT_PERSISTENCE_UNAVAILABLE",
        "IR_COMMIT_CATALOG_UNAVAILABLE",
        "IR_COMMIT_APP_UNAVAILABLE",
        "IR_COMMIT_DESTRUCTIVE_APPROVAL_DENIED",
        "IR_COMMIT_APPLY_FAILED",
        "IR_COMMIT_FINGERPRINT_FAILED",
        "IR_COMMIT_RECEIPT_PERSISTENCE_FAILED",
    ] {
        let mut failed =
            board_edit_job_test_record("failed", BoardEditJobPhase::Failed, Instant::now());
        failed
            .job
            .record_apply_result(ApplyFlowIrCommitResult::empty(
                "error",
                code,
                "Nothing was persisted.",
            ));
        let recovered = board_edit_job_record_from_persisted(PersistedBoardEditJobEntry::Current(
            PersistedBoardEditJobRecord {
                job: failed.job,
                board_commands: failed.board_commands,
                replacement_mode: false,
            },
        ))
        .expect("failed review remains available after restart");
        assert_eq!(recovered.job.phase, BoardEditJobPhase::Failed);
        assert!(!recovered.board_commands.is_empty());
        let jobs = HashMap::from([("failed".to_string(), recovered)]);

        assert!(
            !board_mutation_is_reserved(&jobs, "review-app", "review-board"),
            "manual edits should proceed after {code}"
        );
        assert!(
            !another_board_edit_job_reserves_mutation(
                &jobs,
                "corrected-review",
                "review-app",
                "review-board",
            ),
            "a corrected review should proceed after {code}"
        );
    }
}

#[test]
fn uncertain_failed_results_keep_the_board_reserved() {
    for code in [
        None,
        Some("IR_COMMIT_BOARD_UNAVAILABLE"),
        Some("IR_COMMIT_SAVE_FAILED"),
        Some("UNRECOGNIZED_FAILURE"),
    ] {
        let mut failed =
            board_edit_job_test_record("failed", BoardEditJobPhase::Failed, Instant::now());
        let mut result = ApplyFlowIrCommitResult::empty("error", "", "Recovery is required.");
        result.code = code.map(str::to_string);
        failed.job.record_apply_result(result);
        let jobs = HashMap::from([("failed".to_string(), failed)]);

        assert!(board_mutation_is_reserved(
            &jobs,
            "review-app",
            "review-board"
        ));
        assert!(another_board_edit_job_reserves_mutation(
            &jobs,
            "corrected-review",
            "review-app",
            "review-board"
        ));
    }
}

#[test]
fn active_or_replayed_jobs_cannot_release_the_board_using_an_old_failure_code() {
    for (phase, status, replayed) in [
        (BoardEditJobPhase::Applying, "error", false),
        (BoardEditJobPhase::AppliedPendingDelivery, "error", false),
        (BoardEditJobPhase::Failed, "applied", false),
        (BoardEditJobPhase::Failed, "error", true),
    ] {
        let mut record = board_edit_job_test_record("reserved", phase, Instant::now());
        let mut result =
            ApplyFlowIrCommitResult::empty(status, "IR_COMMIT_APPLY_FAILED", "Old result.");
        result.replayed = replayed;
        record.job.result = Some(result);
        let jobs = HashMap::from([("reserved".to_string(), record)]);
        assert!(board_mutation_is_reserved(
            &jobs,
            "review-app",
            "review-board"
        ));
    }
}

#[test]
fn known_failed_apply_attempts_leave_the_board_available_for_repair() {
    let mut record =
        board_edit_job_test_record("retry", BoardEditJobPhase::AwaitingApproval, Instant::now());
    for code in ["IR_COMMIT_APPLY_FAILED", "IR_COMMIT_CATALOG_UNAVAILABLE"] {
        let recovery_required = record.job.begin_resolution(true);
        assert!(!recovery_required);
        assert_eq!(record.job.phase, BoardEditJobPhase::Applying);
        record.job.finish_apply_attempt(
            ApplyFlowIrCommitResult::empty("error", code, "Nothing was persisted."),
            recovery_required,
        );
        assert_eq!(record.job.phase, BoardEditJobPhase::Failed);
        assert_eq!(
            record.job.result.as_ref().unwrap().code.as_deref(),
            Some(code)
        );
        let jobs = HashMap::from([("retry".to_string(), record.clone())]);
        assert!(!board_mutation_is_reserved(
            &jobs,
            "review-app",
            "review-board"
        ));
        assert!(!another_board_edit_job_reserves_mutation(
            &jobs,
            "corrected-review",
            "review-app",
            "review-board"
        ));
    }
}

#[test]
fn uncertain_apply_retries_keep_reserving_the_board_after_known_failures() {
    for (phase, code) in [
        (BoardEditJobPhase::Failed, None),
        (BoardEditJobPhase::Failed, Some("IR_COMMIT_SAVE_FAILED")),
        (BoardEditJobPhase::Applying, None),
    ] {
        let mut record = board_edit_job_test_record("retry", phase, Instant::now());
        if let Some(code) = code {
            record
                .job
                .record_apply_result(ApplyFlowIrCommitResult::empty(
                    "error",
                    code,
                    "The saved board could not be restored.",
                ));
        }
        for retry_code in ["IR_COMMIT_CATALOG_UNAVAILABLE", "IR_COMMIT_APPLY_FAILED"] {
            let recovery_required = record.job.begin_resolution(true);
            assert!(recovery_required);
            record.job.finish_apply_attempt(
                ApplyFlowIrCommitResult::empty("error", retry_code, "The retry failed."),
                recovery_required,
            );
            assert_eq!(record.job.phase, BoardEditJobPhase::Failed);
            assert!(record.job.result.is_none());
            assert_eq!(record.job.error.as_deref(), Some("The retry failed."));
            let jobs = HashMap::from([("retry".to_string(), record.clone())]);
            assert!(board_mutation_is_reserved(
                &jobs,
                "review-app",
                "review-board"
            ));
            assert!(another_board_edit_job_reserves_mutation(
                &jobs,
                "corrected-review",
                "review-app",
                "review-board"
            ));
        }
    }
}

#[test]
fn successful_uncertain_apply_retry_moves_to_receipt_delivery() {
    let mut record = board_edit_job_test_record("retry", BoardEditJobPhase::Failed, Instant::now());
    record
        .job
        .record_apply_result(ApplyFlowIrCommitResult::empty(
            "error",
            "IR_COMMIT_SAVE_FAILED",
            "The saved board could not be restored.",
        ));
    let recovery_required = record.job.begin_resolution(true);
    assert!(recovery_required);
    let mut receipt = compact_job_test_receipt();
    receipt.replayed = true;
    let fingerprint = receipt.persisted_board_fingerprint.clone();
    record.job.finish_apply_attempt(receipt, recovery_required);
    assert_eq!(record.job.phase, BoardEditJobPhase::AppliedPendingDelivery);
    assert_eq!(record.job.persisted_board_fingerprint, fingerprint);
    assert!(record.job.result.is_none());
    assert!(record.job.error.is_none());
    assert!(record.job.expires_at_ms > record.job.updated_at_ms);
    let jobs = HashMap::from([("retry".to_string(), record)]);
    assert!(board_mutation_is_reserved(
        &jobs,
        "review-app",
        "review-board"
    ));
}

#[test]
fn atomic_typed_apply_errors_keep_the_full_apply_result_contract() {
    let response =
        ApplyFlowIrCommitResult::empty("stale", "IR_COMMIT_REVIEW_STALE", "Nothing was applied.");
    let value = serde_json::to_value(response).expect("serialize atomic apply response");

    assert_eq!(value["status"], "stale");
    assert_eq!(value["code"], "IR_COMMIT_REVIEW_STALE");
    assert_eq!(value["commands"], serde_json::json!([]));
    assert_eq!(value["board_commands"], serde_json::json!([]));
    assert_eq!(value["diagnostics"], serde_json::json!([]));
}

#[test]
fn typed_replacement_and_deletions_are_both_destructive_review_gated() {
    assert_eq!(
        typed_commit_destructive_review_items(true, &[]),
        vec!["The draft uses full-board replacement semantics."]
    );
    assert!(typed_commit_destructive_review_items(false, &[]).is_empty());
    assert_eq!(
        typed_commit_destructive_review_items(
            false,
            &[BoardCommand::RemoveNode {
                node_id: "existing-node".to_string(),
                summary: None,
            }],
        ),
        vec!["node `existing-node`"]
    );
}

#[test]
fn native_destructive_dialog_window_revalidates_the_exact_batch() {
    let reviewed = vec![BoardCommand::RemoveNode {
        node_id: "reviewed-node".to_string(),
        summary: None,
    }];
    assert!(exact_board_command_batch_matches(&reviewed, &reviewed));

    let changed = vec![BoardCommand::RemoveNode {
        node_id: "different-node".to_string(),
        summary: None,
    }];
    assert!(!exact_board_command_batch_matches(&reviewed, &changed));
    assert!(!exact_board_command_batch_matches(&reviewed, &[]));
}

#[test]
fn atomic_typed_apply_receipt_replays_exact_success_after_lost_response() {
    let token = FlowIrCommitToken {
        board_id: "receipt-board".to_string(),
        draft_id: "receipt-draft".to_string(),
        revision: 7,
        base_fingerprint: "base".to_string(),
        claim_id: uuid::Uuid::new_v4().to_string(),
        requires_destructive_approval: false,
    };
    let result = ApplyFlowIrCommitResult {
        status: "applied".to_string(),
        replayed: false,
        code: None,
        message: "Applied exact typed batch.".to_string(),
        commands: Vec::new(),
        board_commands: Vec::new(),
        diagnostics: Vec::new(),
        final_board_node_count: Some(3),
        persisted_board_fingerprint: Some("flowpilot-board-v1:original".to_string()),
    };

    retain_flow_ir_applied_receipt("receipt-app", &token, &result);
    let replay = replay_flow_ir_applied_receipt("receipt-app", &token)
        .expect("exact token replays its applied receipt");
    assert_eq!(replay.status, "applied");
    assert!(replay.replayed);
    assert_eq!(replay.final_board_node_count, Some(3));
    assert_eq!(
        replay.persisted_board_fingerprint,
        result.persisted_board_fingerprint
    );
    assert!(replay.message.contains("idempotent replay"));

    let mut wrong_claim = token.clone();
    wrong_claim.claim_id = uuid::Uuid::new_v4().to_string();
    assert!(replay_flow_ir_applied_receipt("receipt-app", &wrong_claim).is_none());

    FLOW_IR_APPLIED_RECEIPTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&flow_ir_applied_receipt_key("receipt-app", &token));
}

fn atomic_readback_test_board() -> flow_like::flow::board::Board {
    use flow_like::flow::{board::Board, node::Node, variable::VariableType};
    let mut board = Board::new_detached(Some("readback-board".into()), "apps/readback-app".into());
    let mut event = Node::new("events_simple", "Submit", "", "Events");
    event.start = Some(true);
    let event_output = event
        .add_output_pin("exec", "Exec", "", VariableType::Execution)
        .id
        .clone();
    let mut log = Node::new("log_info", "Log", "", "Log");
    let log_exec = log
        .add_input_pin("exec", "Exec", "", VariableType::Execution)
        .id
        .clone();
    let log_message = log
        .add_input_pin("message", "Message", "", VariableType::String)
        .id
        .clone();
    log.add_output_pin("exec_out", "Exec", "", VariableType::Execution);
    let mut getter = Node::new("a2ui_get_element_value", "Read input", "", "UI/Elements");
    getter.set_flowscript_name("ui", "getElementValue");
    getter
        .add_input_pin("element_ref", "Element", "", VariableType::String)
        .default_value = Some(br#""page/input""#.to_vec());
    let value = getter
        .add_output_pin("value", "Value", "", VariableType::String)
        .id
        .clone();
    event
        .pins
        .get_mut(&event_output)
        .unwrap()
        .connected_to
        .insert(log_exec.clone());
    log.pins
        .get_mut(&log_exec)
        .unwrap()
        .depends_on
        .insert(event_output);
    getter
        .pins
        .get_mut(&value)
        .unwrap()
        .connected_to
        .insert(log_message.clone());
    log.pins
        .get_mut(&log_message)
        .unwrap()
        .depends_on
        .insert(value);
    for node in [event, log, getter] {
        board.nodes.insert(node.id.clone(), node);
    }
    board
}

#[test]
fn atomic_graph_fingerprint_tracks_getter_identity_hidden_by_flowscript() {
    use super::super::board_commits::persisted_board_graph_fingerprint;
    use flow_like::flow::copilot::board_fingerprint;
    let board = atomic_readback_test_board();
    let mut changed = board.clone();
    let getter_id = changed
        .nodes
        .values()
        .find(|node| node.name == "a2ui_get_element_value")
        .unwrap()
        .id
        .clone();
    let mut getter = changed.nodes.remove(&getter_id).unwrap();
    getter.id = "replacement-getter".into();
    let output_id = getter
        .pins
        .values()
        .find(|pin| pin.name == "value")
        .unwrap()
        .id
        .clone();
    let mut output = getter.pins.remove(&output_id).unwrap();
    output.id = "replacement-output".into();
    for node in changed.nodes.values_mut() {
        for pin in node.pins.values_mut() {
            if pin.depends_on.remove(&output_id) {
                pin.depends_on.insert(output.id.clone());
            }
        }
    }
    getter.pins.insert(output.id.clone(), output);
    changed.nodes.insert(getter.id.clone(), getter);
    assert_eq!(board_fingerprint(&board), board_fingerprint(&changed));
    assert_ne!(
        persisted_board_graph_fingerprint(&board).unwrap(),
        persisted_board_graph_fingerprint(&changed).unwrap()
    );
}

#[test]
fn atomic_graph_fingerprint_survives_storage_order_and_internal_receipts() {
    use super::super::board_commits::persisted_board_graph_fingerprint;
    use flow_like::flow::board::Board;
    use flow_like_types::{FromProto, ToProto};
    let mut board = atomic_readback_test_board();
    let expected = persisted_board_graph_fingerprint(&board).unwrap();
    board
        .insert_internal_ref(
            "__flow_like_internal_v1/flowpilot-apply-receipt/example",
            "opaque receipt",
        )
        .unwrap();
    board.updated_at = std::time::UNIX_EPOCH;
    board.hash = Some(42);
    assert_eq!(persisted_board_graph_fingerprint(&board).unwrap(), expected);
    for _ in 0..8 {
        let loaded = Board::from_proto(board.to_proto());
        assert_eq!(
            persisted_board_graph_fingerprint(&loaded).unwrap(),
            expected
        );
    }
    board
        .refs
        .insert("user-schema".into(), "{\"type\":\"string\"}".into());
    assert_ne!(persisted_board_graph_fingerprint(&board).unwrap(), expected);
}

#[tokio::test]
async fn atomic_graph_readback_reads_saved_object_and_rejects_later_edits() {
    use super::super::board_commits::persisted_board_graph_fingerprint;
    use flow_like::flow::board::Board;
    use flow_like::flow_like_storage::object_store::memory::InMemory;
    use flow_like_types::FromProto;
    let store = std::sync::Arc::new(InMemory::new());
    let mut board = atomic_readback_test_board();
    let expected = persisted_board_graph_fingerprint(&board).unwrap();
    board.save(Some(store.clone())).await.unwrap();
    board.description = "Unsaved editor edit".into();
    let saved = Board::from_proto(
        Board::load_proto(store.clone(), &board.board_dir, &board.id, None)
            .await
            .unwrap(),
    );
    assert_eq!(persisted_board_graph_fingerprint(&saved).unwrap(), expected);
    assert_ne!(persisted_board_graph_fingerprint(&board).unwrap(), expected);
    board.save(Some(store.clone())).await.unwrap();
    let later = Board::from_proto(
        Board::load_proto(store, &board.board_dir, &board.id, None)
            .await
            .unwrap(),
    );
    assert_ne!(persisted_board_graph_fingerprint(&later).unwrap(), expected);
}

#[test]
fn legacy_atomic_receipt_deserializes_without_claiming_graph_verification() {
    let legacy = serde_json::json!({
        "status": "applied", "message": "old receipt", "commands": [],
        "board_commands": [], "diagnostics": [], "final_board_node_count": 0,
    });
    let receipt: ApplyFlowIrCommitResult = serde_json::from_value(legacy).unwrap();
    assert!(receipt.persisted_board_fingerprint.is_none());
}

fn compact_job_test_receipt() -> ApplyFlowIrCommitResult {
    let mut receipt = ApplyFlowIrCommitResult::empty("applied", "", "Applied exact batch.");
    receipt.persisted_board_fingerprint = Some(format!("flowpilot-board-v1:{}", "a".repeat(64)));
    receipt.commands.push(GenericCommand::CopyPaste(
        flow_like::flow::board::commands::nodes::copy_paste::CopyPasteCommand::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            (0.0, 0.0, 0.0),
        ),
    ));
    receipt.board_commands.push(BoardCommand::RemoveNode {
        node_id: "old".to_string(),
        summary: None,
    });
    receipt
}

#[test]
fn applied_job_retains_compact_proof_without_a_command_receipt() {
    let mut record = board_edit_job_test_record(
        "compact-proof",
        BoardEditJobPhase::AppliedPendingDelivery,
        Instant::now(),
    );
    let receipt = compact_job_test_receipt();
    let expected = receipt.persisted_board_fingerprint.clone();
    record.job.record_apply_result(receipt);
    assert!(record.job.result.is_none());
    assert_eq!(record.job.persisted_board_fingerprint, expected);
    record.board_commands.clear();

    for phase in [
        BoardEditJobPhase::AppliedPendingDelivery,
        BoardEditJobPhase::Applied,
    ] {
        record.job.phase = phase;
        let persisted = PersistedBoardEditJobEntry::Current(PersistedBoardEditJobRecord {
            job: record.job.clone(),
            board_commands: Vec::new(),
            replacement_mode: false,
        });
        let value = serde_json::to_value(&persisted).unwrap();
        assert!(value["job"].get("result").is_none());
        assert_eq!(
            value["job"]["persistedBoardFingerprint"],
            expected.as_deref().unwrap()
        );
        let recovered =
            board_edit_job_record_from_persisted(serde_json::from_value(value.clone()).unwrap())
                .unwrap();
        assert_eq!(recovered.job.phase, phase);
        assert_eq!(recovered.job.persisted_board_fingerprint, expected);
        assert!(recovered.job.result.is_none());
        assert!(recovered.board_commands.is_empty());

        let mut legacy = value;
        legacy["job"]
            .as_object_mut()
            .unwrap()
            .remove("persistedBoardFingerprint");
        let recovered =
            board_edit_job_record_from_persisted(serde_json::from_value(legacy).unwrap()).unwrap();
        assert!(recovered.job.persisted_board_fingerprint.is_none());
    }
}

#[test]
fn job_attempt_reset_and_failed_apply_discard_previous_proof() {
    let mut job = board_edit_job_test_record(
        "retry-proof",
        BoardEditJobPhase::AppliedPendingDelivery,
        Instant::now(),
    )
    .job;
    job.record_apply_result(compact_job_test_receipt());
    job.clear_apply_result();
    assert!(job.persisted_board_fingerprint.is_none());
    assert!(job.result.is_none());

    job.record_apply_result(compact_job_test_receipt());
    job.phase = BoardEditJobPhase::Failed;
    let mut failed = compact_job_test_receipt();
    failed.status = "error".to_string();
    job.record_apply_result(failed);
    assert!(job.persisted_board_fingerprint.is_none());
    let result = job.result.unwrap();
    assert!(result.persisted_board_fingerprint.is_none());
    assert!(result.commands.is_empty());
    assert!(result.board_commands.is_empty());
}

#[test]
fn restarted_incomplete_job_cannot_keep_post_apply_proof() {
    let mut record = board_edit_job_test_record(
        "interrupted-proof",
        BoardEditJobPhase::Applying,
        Instant::now(),
    );
    record.job.persisted_board_fingerprint = compact_job_test_receipt().persisted_board_fingerprint;
    record.job.result = Some(ApplyFlowIrCommitResult::empty(
        "error",
        "IR_COMMIT_APPLY_FAILED",
        "An earlier attempt was rolled back.",
    ));
    let recovered = board_edit_job_record_from_persisted(PersistedBoardEditJobEntry::Current(
        PersistedBoardEditJobRecord {
            job: record.job,
            board_commands: record.board_commands,
            replacement_mode: false,
        },
    ))
    .unwrap();
    assert_eq!(recovered.job.phase, BoardEditJobPhase::Failed);
    assert!(recovered.job.persisted_board_fingerprint.is_none());
    assert!(recovered.job.result.is_none());
    let jobs = HashMap::from([("interrupted-proof".to_string(), recovered)]);
    assert!(board_mutation_is_reserved(
        &jobs,
        "review-app",
        "review-board"
    ));
}
