use super::*;
use crate::LOCAL_OWNER_ID;
use flow_like_types::bot_turn::{BotRunCheckpoint, reply_message_id};

fn checkpoint(revision: u64) -> BotRunCheckpoint {
    BotRunCheckpoint {
        revision,
        started_at_ms: 1_000,
        deadline_at_ms: 1_801_000,
        state: json!({"phase":"ready", "history":["observed result"]}),
    }
}

#[test]
fn checkpoints_survive_reopening_and_fence_stale_writers() {
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "task");
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "task", &checkpoint(1))
        .unwrap();
    let reopened = f.open_again(TestKey::default()).unwrap();
    assert_eq!(
        reopened.run_checkpoint(&bot.id, &room.id, "task").unwrap(),
        Some(checkpoint(1))
    );
    reopened
        .save_run_checkpoint(&bot.id, &room.id, "task", &checkpoint(1))
        .unwrap();
    reopened
        .save_run_checkpoint(&bot.id, &room.id, "task", &checkpoint(2))
        .unwrap();
    assert!(
        f.store
            .save_run_checkpoint(&bot.id, &room.id, "task", &checkpoint(1))
            .is_err()
    );
    let mut changed = checkpoint(3);
    changed.started_at_ms += 1;
    changed.deadline_at_ms += 1;
    assert!(
        f.store
            .save_run_checkpoint(&bot.id, &room.id, "task", &changed)
            .is_err()
    );
    assert_eq!(
        reopened.run_checkpoint(&bot.id, &room.id, "task").unwrap(),
        Some(checkpoint(2))
    );
}

#[test]
fn cancellation_survives_reopening_and_blocks_future_checkpoints() {
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "running");
    f.text(&room.id, LOCAL_OWNER_ID, "queued");
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "running", &checkpoint(1))
        .unwrap();
    for trigger in ["running", "queued"] {
        f.store.cancel_run(&bot.id, &room.id, trigger).unwrap();
    }
    let reopened = f.open_again(TestKey::default()).unwrap();
    for trigger in ["running", "queued"] {
        assert!(reopened.run_cancelled(&bot.id, trigger).unwrap());
        assert!(reopened.run_checkpoint(&bot.id, &room.id, trigger).is_err());
        assert!(
            reopened
                .save_run_checkpoint(&bot.id, &room.id, trigger, &checkpoint(1))
                .is_err()
        );
    }
    assert!(
        reopened
            .recoverable_run_triggers(&room.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_quiet_completed_turn_keeps_its_checkpoint_without_being_replayed() {
    let f = fixture();
    let bot = f.bot("Monitor");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "quiet-run");
    let mut saved = checkpoint(1);
    saved.state = json!({"phase":"finished", "answer":{"output":"No changes", "stop_reason":null}});
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "quiet-run", &saved)
        .unwrap();
    assert_eq!(f.store.recoverable_run_triggers(&room.id).unwrap().len(), 1);
    f.store
        .settle_room_decision(&bot.id, &room.id, "quiet-run", None)
        .unwrap();
    let reopened = f.open_again(TestKey::default()).unwrap();
    assert!(
        reopened
            .recoverable_run_triggers(&room.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        reopened
            .run_checkpoint(&bot.id, &room.id, "quiet-run")
            .unwrap(),
        Some(saved)
    );
}

#[test]
fn handling_a_newer_message_preserves_unfinished_recovery() {
    for phase in ["ready", "tools_in_flight"] {
        let f = fixture();
        let bot = f.bot("Worker");
        let room = f.direct(&bot);
        f.text(&room.id, LOCAL_OWNER_ID, "pending");
        let mut saved = checkpoint(1);
        saved.state["phase"] = json!(phase);
        f.store
            .save_run_checkpoint(&bot.id, &room.id, "pending", &saved)
            .unwrap();
        f.text(&room.id, LOCAL_OWNER_ID, "newer");
        f.store
            .settle_room_decision(&bot.id, &room.id, "newer", None)
            .unwrap();
        let reopened = f.open_again(TestKey::default()).unwrap();
        let recoverable = reopened.recoverable_run_triggers(&room.id).unwrap();
        assert_eq!(recoverable.len(), 1, "{phase}");
        assert_eq!(recoverable[0].1.message_id, "pending");
        assert_eq!(
            reopened
                .run_checkpoint(&bot.id, &room.id, "pending")
                .unwrap(),
            Some(saved)
        );
    }
}

#[test]
fn monitoring_baselines_are_bounded_scoped_and_reset_when_the_instruction_changes() {
    use flow_like_types::inbox::{TriggerBody, TriggerSource};
    let f = fixture();
    let bot = f.bot("Monitor");
    let other = f.bot("Other");
    let room = f.direct(&bot);
    let trigger = |id: &str, task: &str, instruction: &str, monitoring: bool| {
        f.store
            .post_trigger(
                &bot.id,
                id,
                &TriggerBody {
                    monitoring,
                    source: TriggerSource::Schedule,
                    ref_id: task.into(),
                    title: "Check".into(),
                    text: instruction.into(),
                    payload: None,
                    payload_handle: None,
                    bot_id: None,
                },
            )
            .unwrap();
    };
    trigger("old", "task-a", "Check report", true);
    let mut saved = checkpoint(1);
    saved.state =
        json!({"phase":"finished", "answer":{"output":"🙂".repeat(5_000), "stop_reason":null}});
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "old", &saved)
        .unwrap();
    f.store
        .settle_room_decision(&bot.id, &room.id, "old", None)
        .unwrap();
    trigger("failed", "task-a", "Check report", true);
    let mut failed = checkpoint(1);
    failed.state = json!({"phase":"finished", "answer":{"output":"Unavailable", "stop_reason":"tool_round_limit"}});
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "failed", &failed)
        .unwrap();
    trigger("current", "task-a", "Check report", true);
    let reopened = f.open_again(TestKey::default()).unwrap();
    let result = reopened
        .previous_monitor_result(&bot.id, &room.id, "current")
        .unwrap()
        .unwrap();
    assert_eq!(result.chars().count(), 4_000);
    assert!(
        reopened
            .previous_monitor_result(&other.id, &room.id, "current")
            .is_err()
    );
    trigger("changed", "task-a", "Check another report", true);
    trigger("different-task", "task-b", "Check report", true);
    trigger("reminder", "task-a", "Check report", false);
    for id in ["changed", "different-task", "reminder"] {
        assert_eq!(
            reopened
                .previous_monitor_result(&bot.id, &room.id, id)
                .unwrap(),
            None
        );
    }
    f.store.cancel_run(&bot.id, &room.id, "old").unwrap();
    assert_eq!(
        reopened
            .previous_monitor_result(&bot.id, &room.id, "current")
            .unwrap(),
        None
    );
}

#[test]
fn recovery_is_scoped_and_keeps_the_original_trigger_until_the_reply_is_published() {
    let f = fixture();
    let bot = f.bot("Scout");
    let other = f.bot("Other");
    let room = f.direct(&bot);
    let another_room = f.direct(&other);
    f.text(&room.id, LOCAL_OWNER_ID, "original");
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "original", &checkpoint(1))
        .unwrap();
    for index in 0..120 {
        f.text(&room.id, LOCAL_OWNER_ID, &format!("newer_{index}"));
    }
    let recent = f
        .store
        .entries_as(
            &room.id,
            &bot.id,
            &crate::EntriesQuery {
                limit: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        recent
            .entries
            .iter()
            .all(|entry| entry.message_id != "original")
    );
    let pending = f.store.recoverable_run_triggers(&room.id).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.message_id, "original");
    assert!(
        f.store
            .run_checkpoint(&other.id, &room.id, "original")
            .is_err()
    );
    assert!(
        f.store
            .run_checkpoint(&bot.id, &another_room.id, "original")
            .is_err()
    );
    f.text(&room.id, &bot.id, &reply_message_id(&bot.id, "original"));
    assert!(
        f.store
            .recoverable_run_triggers(&room.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_new_topic_prevents_automatic_resumption_of_old_work() {
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "original");
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "original", &checkpoint(1))
        .unwrap();
    f.send(
        &room.id,
        LOCAL_OWNER_ID,
        "topic",
        "topic",
        json!({"title":"Next task"}),
    )
    .unwrap();
    assert!(
        f.store
            .recoverable_run_triggers(&room.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deleting_the_bot_removes_its_saved_execution_state() {
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "original");
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "original", &checkpoint(1))
        .unwrap();
    f.store.delete_bot(&bot.id).unwrap();
    let count: i64 = f
        .raw()
        .query_row("SELECT COUNT(*) FROM bot_run_checkpoints", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn an_approval_answer_rebases_the_deadline_and_keeps_revision_and_state() {
    use flow_like_types::bot_message::{ApprovalRequest, ApprovalResponse};
    use flow_like_types::tool_grant::{ApprovalDecision, approval_message_id};
    let f = fixture();
    let bot = f.bot("Scout");
    let room = f.direct(&bot);
    f.text(&room.id, LOCAL_OWNER_ID, "task");
    let saved = checkpoint(1);
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "task", &saved)
        .unwrap();
    let turn = flow_like_types::bot_turn::turn_id(&bot.id, "task");
    let card = ApprovalRequest {
        id: approval_message_id(&turn, "hash1"),
        turn_id: turn.clone(),
        tool: "code_run".into(),
        summary: "Run code".into(),
        scope: "host:example.com".into(),
        args_hash: "hash1".into(),
        requester_id: None,
        requester_name: None,
        room: None,
        secret: None,
        configuration_changes: None,
    };
    f.store
        .park_turn(&turn, &bot.id, &room.id, "task", &card)
        .unwrap();
    let response = ApprovalResponse {
        approval_id: card.id.clone(),
        decision: ApprovalDecision::Once,
    };
    f.store.resolve_parked(&bot.id, &turn, &response).unwrap();
    let rebased = f
        .store
        .run_checkpoint(&bot.id, &room.id, "task")
        .unwrap()
        .unwrap();
    assert_eq!((rebased.revision, &rebased.state), (1, &saved.state));
    assert!(rebased.started_at_ms > saved.started_at_ms);
    assert_eq!(
        rebased.deadline_at_ms - rebased.started_at_ms,
        flow_like_types::bot_turn::DEFAULT_TURN_DEADLINE_SECS as i64 * 1000
    );
    let mut next = checkpoint(2);
    next.started_at_ms = rebased.started_at_ms;
    next.deadline_at_ms = rebased.deadline_at_ms;
    f.store
        .save_run_checkpoint(&bot.id, &room.id, "task", &next)
        .unwrap();
    let mut shifted = checkpoint(3);
    shifted.started_at_ms = next.started_at_ms + 1;
    shifted.deadline_at_ms = next.deadline_at_ms + 1;
    assert!(
        f.store
            .save_run_checkpoint(&bot.id, &room.id, "task", &shifted)
            .is_err()
    );
}
