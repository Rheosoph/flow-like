use super::{
    fixture::{at, conversation, database, delivery, insert_bot, recipient},
    ledger,
    recovery::{self, Claim},
};
use crate::entity::bot_turn;
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{BotRunCheckpoint, MAX_CHECKPOINT_BYTES, STORED_FAILED},
    inbox::ConversationKind,
};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait, sea_query::Expr,
};
use serde_json::json;

async fn setup() -> (DatabaseConnection, Claim) {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    conversation(&db, "c1", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
    let mut recipient = recipient("bot_1", "user-a", BotLocation::Server);
    recipient.conversation_id = "c1".into();
    recipient.trigger_message_id = "m1".into();
    ledger::enqueue_in(
        &db,
        &delivery(ConversationKind::Direct, vec![recipient]),
        at(0),
    )
    .await
    .unwrap();
    let claim = Claim {
        turn_id: flow_like_types::bot_turn::turn_id("bot_1", "m1"),
        owner_id: "user-a".into(),
        host_id: "host-old".into(),
        attempt: 1,
    };
    assert!(
        ledger::take_in(&db, &claim.turn_id, &claim.host_id, at(1))
            .await
            .unwrap()
    );
    (db, claim)
}

fn snapshot() -> BotRunCheckpoint {
    BotRunCheckpoint {
        revision: 1,
        started_at_ms: at(1).timestamp_millis(),
        deadline_at_ms: at(1801).timestamp_millis(),
        state: json!({"version":1,"phase":"ready"}),
    }
}

async fn save(
    db: &DatabaseConnection,
    claim: &Claim,
    saved: &BotRunCheckpoint,
    seconds: i64,
) -> Result<(), crate::error::ApiError> {
    let txn = db.begin().await.unwrap();
    let result = recovery::save_in(&txn, claim, saved, at(seconds)).await;
    if result.is_ok() {
        txn.commit().await.unwrap();
    } else {
        txn.rollback().await.unwrap();
    }
    result
}

#[tokio::test]
async fn checkpoint_revisions_are_idempotent_and_preserve_the_original_deadline() {
    let (db, claim) = setup().await;
    let first = snapshot();
    assert_eq!(recovery::load_in(&db, &claim, at(2)).await.unwrap(), None);
    save(&db, &claim, &first, 2).await.unwrap();
    save(&db, &claim, &first, 3).await.unwrap();
    let mut changed = first.clone();
    changed.state = json!({"version":1,"phase":"tools_in_flight"});
    assert_eq!(
        save(&db, &claim, &changed, 4)
            .await
            .unwrap_err()
            .public_code(),
        recovery::CONFLICT
    );
    changed.revision = 2;
    save(&db, &claim, &changed, 4).await.unwrap();
    let mut extended = changed.clone();
    extended.revision = 3;
    extended.started_at_ms += 1;
    extended.deadline_at_ms += 1;
    assert_eq!(
        save(&db, &claim, &extended, 5)
            .await
            .unwrap_err()
            .public_code(),
        recovery::CONFLICT
    );
    assert_eq!(
        recovery::load_in(&db, &claim, at(6)).await.unwrap(),
        Some(changed)
    );
}

#[tokio::test]
async fn reclaimed_turn_reads_the_checkpoint_and_fences_the_previous_attempt() {
    let (db, old) = setup().await;
    let saved = snapshot();
    save(&db, &old, &saved, 2).await.unwrap();
    assert!(
        ledger::take_in(&db, &old.turn_id, "host-new", at(62))
            .await
            .unwrap()
    );
    let current = Claim {
        host_id: "host-new".into(),
        attempt: 2,
        ..old.clone()
    };
    assert_eq!(
        recovery::load_in(&db, &current, at(63)).await.unwrap(),
        Some(saved.clone())
    );
    let next = BotRunCheckpoint {
        revision: 2,
        ..saved
    };
    for stale in [
        old,
        Claim {
            attempt: 1,
            ..current.clone()
        },
        Claim {
            owner_id: "user-b".into(),
            ..current.clone()
        },
    ] {
        assert_eq!(
            save(&db, &stale, &next, 63)
                .await
                .unwrap_err()
                .public_code(),
            ledger::BOT_TURN_LEASE_LOST
        );
        assert!(recovery::load_in(&db, &stale, at(63)).await.is_err());
    }
    save(&db, &current, &next, 63).await.unwrap();
}

#[tokio::test]
async fn cancellation_and_expired_leases_prevent_checkpoint_access() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    assert!(recovery::load_in(&db, &claim, at(62)).await.is_err());
    assert!(save(&db, &claim, &snapshot(), 62).await.is_err());
    ledger::cancel_in(&db, &claim.turn_id, ledger::ERROR_OWNER_CANCELLED, at(3))
        .await
        .unwrap();
    assert!(recovery::load_in(&db, &claim, at(4)).await.is_err());
    assert!(save(&db, &claim, &snapshot(), 4).await.is_err());
    assert!(
        !ledger::take_in(&db, &claim.turn_id, "host-new", at(62))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn checkpoint_limits_and_expired_deadlines_survive_storage() {
    let (db, claim) = setup().await;
    let oversized = BotRunCheckpoint {
        state: json!({"text":"x".repeat(MAX_CHECKPOINT_BYTES)}),
        ..snapshot()
    };
    assert!(save(&db, &claim, &oversized, 2).await.is_err());
    let expired = BotRunCheckpoint {
        started_at_ms: at(0).timestamp_millis(),
        deadline_at_ms: at(1).timestamp_millis(),
        ..snapshot()
    };
    save(&db, &claim, &expired, 2).await.unwrap();
    assert_eq!(
        recovery::load_in(&db, &claim, at(3)).await.unwrap(),
        Some(expired)
    );
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_FAILED))
        .filter(bot_turn::Column::Id.eq(&claim.turn_id))
        .exec(&db)
        .await
        .unwrap();
    assert!(recovery::load_in(&db, &claim, at(4)).await.is_err());
}

#[tokio::test]
async fn encrypted_conversations_reject_plaintext_and_accept_large_sealed_checkpoints() {
    let (db, claim) = setup().await;
    use crate::entity::conversation;
    conversation::Entity::update_many()
        .col_expr(conversation::Column::Encryption, Expr::value("E2E"))
        .filter(conversation::Column::Id.eq("c1"))
        .exec(&db)
        .await
        .unwrap();
    assert!(save(&db, &claim, &snapshot(), 2).await.is_err());
    let sealed = BotRunCheckpoint {
        state: json!({"sealed_checkpoint":1,"envelope": {
            "v":1,"enc":"hpke","key_id":"key","enc_key":"key","ct":"x".repeat(100_000)
        }}),
        ..snapshot()
    };
    save(&db, &claim, &sealed, 2).await.unwrap();
    let smuggled = BotRunCheckpoint {
        revision: 2,
        state: json!({"sealed_checkpoint":1,"envelope": {
            "v":1,"enc":"hpke","key_id":"key","enc_key":"key","ct":"ciphertext","body":{"text":"private"}
        }}),
        ..snapshot()
    };
    assert!(save(&db, &claim, &smuggled, 3).await.is_err());
}

#[test]
fn recovery_runtime_preserves_caps_and_allows_only_a_bounded_delivery_grace() {
    use super::worker::{MAX_TURN_RUNTIME_MS, recovery_runtime_ms};
    let saved = snapshot();
    assert_eq!(
        recovery_runtime_ms(None, saved.started_at_ms),
        MAX_TURN_RUNTIME_MS
    );
    assert_eq!(
        recovery_runtime_ms(Some(&saved), saved.deadline_at_ms - 1234),
        1234
    );
    assert_eq!(
        recovery_runtime_ms(Some(&saved), saved.deadline_at_ms),
        30_000
    );
    assert_eq!(
        recovery_runtime_ms(Some(&saved), saved.deadline_at_ms + 60_000),
        30_000
    );
}

#[tokio::test]
async fn foreground_hold_binds_one_host_and_fences_it_after_background_takeover() {
    let (db, running) = setup().await;
    bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::Status,
            Expr::value(flow_like_types::bot_turn::STORED_QUEUED),
        )
        .col_expr(bot_turn::Column::Attempts, Expr::value(0))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .col_expr(
            bot_turn::Column::HostKind,
            Expr::value(flow_like_types::bot_turn::STORED_DESKTOP),
        )
        .filter(bot_turn::Column::Id.eq(&running.turn_id))
        .exec(&db)
        .await
        .unwrap();
    let foreground = Claim {
        host_id: "interactive:first".into(),
        attempt: 0,
        ..running.clone()
    };
    assert_eq!(
        recovery::load_in(&db, &foreground, at(2)).await.unwrap(),
        None
    );
    save(&db, &foreground, &snapshot(), 2).await.unwrap();
    let competing = Claim {
        host_id: "interactive:other".into(),
        ..foreground.clone()
    };
    assert!(recovery::load_in(&db, &competing, at(3)).await.is_err());
    assert!(save(&db, &competing, &snapshot(), 3).await.is_err());
    assert!(
        ledger::take_in(&db, &running.turn_id, "host-new", at(62))
            .await
            .unwrap()
    );
    let background = Claim {
        host_id: "host-new".into(),
        ..running
    };
    assert_eq!(
        recovery::load_in(&db, &background, at(63)).await.unwrap(),
        Some(snapshot())
    );
    assert!(save(&db, &foreground, &snapshot(), 63).await.is_err());
}

#[tokio::test]
async fn only_the_bound_foreground_host_can_complete_the_interactive_reply() {
    let (db, running) = setup().await;
    bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::Status,
            Expr::value(flow_like_types::bot_turn::STORED_QUEUED),
        )
        .col_expr(bot_turn::Column::Attempts, Expr::value(0))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .filter(bot_turn::Column::Id.eq(&running.turn_id))
        .exec(&db)
        .await
        .unwrap();
    let foreground = Claim {
        host_id: "interactive:first".into(),
        attempt: 0,
        ..running
    };
    save(&db, &foreground, &snapshot(), 2).await.unwrap();
    use crate::routes::inbox::{model::TurnClaim, turns::complete_interactive_in};
    let mut reply = TurnClaim {
        id: foreground.turn_id.clone(),
        host_id: "interactive:other".into(),
    };
    assert!(
        complete_interactive_in(&db, &reply, "bot_1", "c1", "reply", at(3))
            .await
            .is_err()
    );
    reply.host_id = foreground.host_id;
    complete_interactive_in(&db, &reply, "bot_1", "c1", "reply", at(3))
        .await
        .unwrap();
    assert!(stored_checkpoint(&db, &foreground.turn_id).await.is_none());
    let done = ledger::find(&db, &foreground.turn_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.error_code, None);
}

#[tokio::test]
async fn an_interactive_completion_needs_a_live_hold_and_a_live_bot() {
    use crate::routes::inbox::{model::TurnClaim, turns::complete_interactive_in};
    for (name, now, delete) in [
        ("expired hold", at(10_000), false),
        ("deleted bot", at(3), true),
    ] {
        let (db, running) = setup().await;
        bot_turn::Entity::update_many()
            .col_expr(
                bot_turn::Column::Status,
                Expr::value(flow_like_types::bot_turn::STORED_QUEUED),
            )
            .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
            .filter(bot_turn::Column::Id.eq(&running.turn_id))
            .exec(&db)
            .await
            .unwrap();
        if delete {
            super::fixture::delete_bot(&db, "bot_1").await;
        }
        let reply = TurnClaim {
            id: running.turn_id.clone(),
            host_id: "interactive:first".into(),
        };
        assert!(
            complete_interactive_in(&db, &reply, "bot_1", "c1", "reply", now)
                .await
                .is_err(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_completed_turn_clears_its_checkpoint_but_a_failure_report_keeps_it() {
    use crate::routes::inbox::{model::TurnClaim, turns::complete_turn_in};
    for (reply_id, kept) in [
        ("reply".to_string(), false),
        (
            flow_like_types::bot_turn::failure_message_id("bot_1", "m1"),
            true,
        ),
    ] {
        let (db, claim) = setup().await;
        save(&db, &claim, &snapshot(), 2).await.unwrap();
        bot_turn::Entity::update_many()
            .col_expr(
                bot_turn::Column::ErrorCode,
                Expr::value(ledger::ERROR_RESUMING),
            )
            .filter(bot_turn::Column::Id.eq(&claim.turn_id))
            .exec(&db)
            .await
            .unwrap();
        let host = TurnClaim {
            id: claim.turn_id.clone(),
            host_id: claim.host_id.clone(),
        };
        complete_turn_in(&db, &host, "bot_1", "c1", &reply_id, at(3))
            .await
            .unwrap();
        assert_eq!(stored_checkpoint(&db, &claim.turn_id).await.is_some(), kept);
        let done = ledger::find(&db, &claim.turn_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.error_code, None);
    }
}

#[tokio::test]
async fn manual_retry_keeps_claim_generations_unique_and_resets_only_failure_budget() {
    let (db, old) = setup().await;
    save(&db, &old, &snapshot(), 2).await.unwrap();
    ledger::fail_held_in(&db, &old.turn_id, &old.host_id, "TEST", at(3))
        .await
        .unwrap();
    use crate::entity::bot_turn_checkpoint;
    bot_turn_checkpoint::Entity::update_many()
        .col_expr(
            bot_turn_checkpoint::Column::RecoveryAttempts,
            Expr::value(flow_like_types::bot_turn::MAX_ATTEMPTS as i32),
        )
        .filter(bot_turn_checkpoint::Column::TurnId.eq(&old.turn_id))
        .exec(&db)
        .await
        .unwrap();
    let queued = ledger::retry_in(&db, &old.owner_id, "bot_1", &old.turn_id, at(4))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.attempts, 1);
    let new = ledger::claim_in(
        &db,
        &ledger::ClaimRequest {
            turn_id: old.turn_id.clone(),
            owner_id: Some(old.owner_id.clone()),
            host_kind: flow_like_types::bot_turn::TurnHostKind::Server,
            host_id: old.host_id.clone(),
            now: at(5),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .turn;
    assert_eq!(new.attempts, 2);
    assert_eq!(recovery::failures(&db, &new).await.unwrap(), 0);
    assert!(
        save(
            &db,
            &old,
            &BotRunCheckpoint {
                revision: 2,
                ..snapshot()
            },
            6
        )
        .await
        .is_err()
    );
    assert_eq!(
        recovery::load_in(&db, &Claim { attempt: 2, ..old }, at(6))
            .await
            .unwrap(),
        Some(snapshot())
    );
}

#[tokio::test]
async fn approval_handoffs_do_not_spend_the_crash_recovery_budget() {
    let (db, first) = setup().await;
    save(&db, &first, &snapshot(), 2).await.unwrap();
    for attempt in 2..=7 {
        // An approved WAITING turn returns to QUEUED without reporting execution failure.
        bot_turn::Entity::update_many()
            .col_expr(
                bot_turn::Column::Status,
                Expr::value(flow_like_types::bot_turn::STORED_QUEUED),
            )
            .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
            .col_expr(
                bot_turn::Column::LeaseUntil,
                Expr::value(None::<chrono::DateTime<chrono::FixedOffset>>),
            )
            .filter(bot_turn::Column::Id.eq(&first.turn_id))
            .exec(&db)
            .await
            .unwrap();
        let claimed = ledger::claim_in(
            &db,
            &ledger::ClaimRequest {
                turn_id: first.turn_id.clone(),
                owner_id: Some(first.owner_id.clone()),
                host_kind: flow_like_types::bot_turn::TurnHostKind::Server,
                host_id: first.host_id.clone(),
                now: at(attempt as i64 + 2),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(claimed.turn.attempts, attempt);
        assert_eq!(recovery::failures(&db, &claimed.turn).await.unwrap(), 0);
    }
    assert_eq!(
        recovery::load_in(
            &db,
            &Claim {
                attempt: 7,
                ..first
            },
            at(10)
        )
        .await
        .unwrap(),
        Some(snapshot())
    );
}

#[tokio::test]
async fn released_transient_failures_share_the_bounded_recovery_budget() {
    let (db, first) = setup().await;
    for failure in 1..=flow_like_types::bot_turn::MAX_ATTEMPTS {
        let now = at(2 + failure as i64);
        let released = ledger::release_for_retry_in(
            &db,
            &first.turn_id,
            &first.host_id,
            now,
            "TRANSIENT",
            now,
        )
        .await
        .unwrap();
        assert_eq!(
            released,
            if failure == flow_like_types::bot_turn::MAX_ATTEMPTS {
                ledger::RetryRelease::Spent
            } else {
                ledger::RetryRelease::Requeued
            }
        );
        let turn = bot_turn::Entity::find_by_id(&first.turn_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            recovery::failures(&db, &turn).await.unwrap(),
            failure as i32
        );
        if failure == flow_like_types::bot_turn::MAX_ATTEMPTS {
            assert_eq!(turn.status, flow_like_types::bot_turn::STORED_RUNNING);
            assert_eq!(turn.host_id.as_deref(), Some(first.host_id.as_str()));
        } else {
            ledger::claim_in(
                &db,
                &ledger::ClaimRequest {
                    turn_id: first.turn_id.clone(),
                    owner_id: Some(first.owner_id.clone()),
                    host_kind: flow_like_types::bot_turn::TurnHostKind::Server,
                    host_id: first.host_id.clone(),
                    now,
                },
            )
            .await
            .unwrap()
            .unwrap();
        }
    }
}

#[tokio::test]
async fn automatic_recovery_is_bounded_even_before_the_first_execution_checkpoint() {
    let (db, first) = setup().await;
    for failure in 1..=flow_like_types::bot_turn::MAX_ATTEMPTS {
        let now = at(1 + failure as i64 * 61);
        let recovered = ledger::recover_in(&db, now).await.unwrap();
        if failure == flow_like_types::bot_turn::MAX_ATTEMPTS {
            assert_eq!(recovered.lost, 1);
        } else {
            assert_eq!(recovered.requeued, 1);
            let turn = ledger::claim_in(
                &db,
                &ledger::ClaimRequest {
                    turn_id: first.turn_id.clone(),
                    owner_id: Some(first.owner_id.clone()),
                    host_kind: flow_like_types::bot_turn::TurnHostKind::Server,
                    host_id: first.host_id.clone(),
                    now,
                },
            )
            .await
            .unwrap()
            .unwrap()
            .turn;
            assert_eq!(turn.attempts as u32, failure + 1);
            assert_eq!(
                recovery::load_in(
                    &db,
                    &Claim {
                        attempt: failure + 1,
                        ..first.clone()
                    },
                    now
                )
                .await
                .unwrap(),
                None
            );
        }
    }
    assert_eq!(
        bot_turn::Entity::find_by_id(&first.turn_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        STORED_FAILED
    );
}

#[tokio::test]
async fn retired_bot_checkpoints_are_purged_and_cannot_be_recreated() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    super::fixture::delete_bot(&db, "bot_1").await;
    assert!(recovery::load_in(&db, &claim, at(3)).await.is_err());
    assert!(save(&db, &claim, &snapshot(), 3).await.is_err());
    use crate::entity::bot_turn_checkpoint;
    // The production retirement planner selects checkpoints through the retained BotTurn rows.
    let (table, predicate) = crate::deletion::retired_bot::retired_bot_rows()
        .into_iter()
        .find(|(table, _)| *table == "BotTurnCheckpoint")
        .unwrap();
    let meta = crate::deletion::graph::fk_graph().table(table).unwrap();
    let keys = crate::deletion::drain::select_page(&db, meta, &predicate, "bot_1", 200)
        .await
        .unwrap();
    crate::deletion::drain::delete_page(&db, meta, &keys, &predicate, "bot_1")
        .await
        .unwrap();
    assert!(
        bot_turn_checkpoint::Entity::find_by_id(&claim.turn_id)
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(save(&db, &claim, &snapshot(), 4).await.is_err());
}

#[tokio::test]
async fn an_interactive_claim_cannot_touch_a_server_turn() {
    let (db, running) = setup().await;
    bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::Status,
            Expr::value(flow_like_types::bot_turn::STORED_QUEUED),
        )
        .col_expr(bot_turn::Column::Attempts, Expr::value(0))
        .col_expr(bot_turn::Column::HostId, Expr::value(None::<String>))
        .filter(bot_turn::Column::Id.eq(&running.turn_id))
        .exec(&db)
        .await
        .unwrap();
    let foreground = Claim {
        host_id: "interactive:first".into(),
        attempt: 0,
        ..running
    };
    assert!(recovery::load_in(&db, &foreground, at(2)).await.is_err());
    assert!(save(&db, &foreground, &snapshot(), 2).await.is_err());
}

async fn stored_checkpoint(
    db: &DatabaseConnection,
    turn_id: &str,
) -> Option<crate::entity::bot_turn_checkpoint::Model> {
    crate::entity::bot_turn_checkpoint::Entity::find_by_id(turn_id)
        .one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_retry_rebases_the_deadline_and_keeps_revision_and_state() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    assert!(
        ledger::fail_held_in(
            &db,
            &claim.turn_id,
            &claim.host_id,
            "BOT_TURN_TIMEOUT",
            at(3)
        )
        .await
        .unwrap()
    );
    let later = at(10_000);
    ledger::retry_in(&db, "user-a", "bot_1", &claim.turn_id, later)
        .await
        .unwrap()
        .unwrap();
    let row = stored_checkpoint(&db, &claim.turn_id).await.unwrap();
    assert_eq!(row.started_at_ms, later.timestamp_millis());
    assert_eq!(
        row.deadline_at_ms,
        later.timestamp_millis() + super::worker::MAX_TURN_RUNTIME_MS
    );
    assert_eq!((row.revision, row.state), (1, snapshot().state));
    let rebased = BotRunCheckpoint {
        started_at_ms: row.started_at_ms,
        deadline_at_ms: row.deadline_at_ms,
        ..snapshot()
    };
    assert_eq!(
        super::worker::recovery_runtime_ms(Some(&rebased), later.timestamp_millis()),
        super::worker::MAX_TURN_RUNTIME_MS
    );
}

#[tokio::test]
async fn crash_recovery_keeps_the_saved_deadline() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    ledger::recover_in(&db, at(10_000)).await.unwrap();
    let row = stored_checkpoint(&db, &claim.turn_id).await.unwrap();
    assert_eq!(row.started_at_ms, snapshot().started_at_ms);
    assert_eq!(row.deadline_at_ms, snapshot().deadline_at_ms);
}

#[tokio::test]
async fn finished_turns_drop_their_checkpoint_but_failed_turns_keep_it_for_a_retry() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    assert!(
        ledger::fail_held_in(
            &db,
            &claim.turn_id,
            &claim.host_id,
            "BOT_TURN_TIMEOUT",
            at(3)
        )
        .await
        .unwrap()
    );
    assert!(stored_checkpoint(&db, &claim.turn_id).await.is_some());
    ledger::retry_in(&db, "user-a", "bot_1", &claim.turn_id, at(4))
        .await
        .unwrap()
        .unwrap();
    assert!(
        ledger::take_in(&db, &claim.turn_id, "host-2", at(5))
            .await
            .unwrap()
    );
    assert!(
        ledger::complete_silent_in(&db, &claim.turn_id, "host-2", at(6))
            .await
            .unwrap()
    );
    assert!(stored_checkpoint(&db, &claim.turn_id).await.is_none());
}

#[tokio::test]
async fn cancelling_a_turn_drops_its_checkpoint() {
    let (db, claim) = setup().await;
    save(&db, &claim, &snapshot(), 2).await.unwrap();
    ledger::cancel_owned_in(&db, "user-a", "bot_1", &claim.turn_id, at(3))
        .await
        .unwrap();
    assert!(stored_checkpoint(&db, &claim.turn_id).await.is_none());
}
