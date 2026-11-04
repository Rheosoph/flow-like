use super::{
    fixture::{
        assert_code, at, conversation, database, delete_bot, delivery, insert_bot, recipient, says,
    },
    ledger::{
        BOT_TURN_LEASE_LOST, BOT_TURN_NOT_RETRYABLE, ClaimRequest, DESKTOP_HANDOFF_SECS, Refusal,
        claim_in, enqueue_in, fail_held_in, hand_back_in, heartbeat_in, mark_failed_after_reply_in,
        next_head, queued_heads, queues_turn, recover_in, release_for_retry_in, retry_in, take_in,
        watched_turn,
    },
};
use crate::{
    entity::{bot, bot_turn},
    routes::{
        bots::presence::{ONLINE_WINDOW_SECS, report_in},
        inbox::{delivery::BotRecipient, fixture::insert_flux},
    },
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{
        ERROR_BATCHED, ERROR_BOT_DELETED, ERROR_HOST_LOST, ERROR_QUEUE_EXPIRED, LEASE_SECS,
        MAX_ATTEMPTS, MAX_HOPS, MAX_RUNNING_PER_OWNER, QUEUED_EXPIRY_SECS, STORED_CANCELLED,
        STORED_DONE, STORED_FAILED, STORED_QUEUED, STORED_RUNNING, TurnHostKind, TurnStatus,
        reply_message_id, turn_id,
    },
    inbox::{ConversationKind, InboxEntryKind},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    sea_query::Expr,
};

const SERVER: BotLocation = BotLocation::Server;

async fn turns(db: &DatabaseConnection) -> Vec<bot_turn::Model> {
    bot_turn::Entity::find()
        .order_by_asc(bot_turn::Column::CreatedAt)
        .order_by_asc(bot_turn::Column::Id)
        .all(db)
        .await
        .unwrap()
}

async fn row(db: &DatabaseConnection, bot_id: &str, trigger: &str) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(turn_id(bot_id, trigger))
        .one(db)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no turn for {bot_id} / {trigger}"))
}

/// Queues a turn for `bot_id` (owned by `owner`) for `trigger` in `conversation_id` at `now`.
async fn queue(
    db: &DatabaseConnection,
    bot_id: &str,
    owner: &str,
    location: BotLocation,
    conversation_id: &str,
    trigger: &str,
    now: DateTime<FixedOffset>,
) {
    let mut recipient = recipient(bot_id, owner, location);
    recipient.conversation_id = conversation_id.into();
    recipient.trigger_message_id = trigger.into();
    let queued = enqueue_in(
        db,
        &delivery(ConversationKind::Direct, vec![recipient]),
        now,
    )
    .await
    .unwrap();
    assert_eq!(queued, 1, "{bot_id} / {trigger}");
}

fn claim_request(
    turn: &str,
    host: &str,
    kind: TurnHostKind,
    now: DateTime<FixedOffset>,
) -> ClaimRequest {
    ClaimRequest {
        turn_id: turn.into(),
        owner_id: None,
        host_kind: kind,
        host_id: host.into(),
        now,
    }
}

async fn claim(
    db: &DatabaseConnection,
    bot_id: &str,
    trigger: &str,
    host: &str,
    now: DateTime<FixedOffset>,
) -> Result<super::ledger::Claimed, Refusal> {
    claim_in(
        db,
        &claim_request(&turn_id(bot_id, trigger), host, TurnHostKind::Server, now),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_direct_message_queues_a_turn_for_every_bot_but_the_sender() {
    let db = database().await;
    let recipients = vec![
        recipient("bot_1", "user-a", SERVER),
        recipient("bot_2", "user-a", BotLocation::Desktop),
        recipient("bot_3", "user-b", BotLocation::Device),
    ];
    let queued = enqueue_in(&db, &delivery(ConversationKind::Direct, recipients), at(1))
        .await
        .unwrap();
    assert_eq!(queued, 3);
    let rows = turns(&db).await;
    let mut shape: Vec<_> = rows
        .iter()
        .map(|turn| {
            (
                turn.bot_id.as_str(),
                turn.owner_id.as_str(),
                turn.host_kind.as_str(),
            )
        })
        .collect();
    shape.sort();
    assert_eq!(
        shape,
        [
            ("bot_1", "user-a", "SERVER"),
            ("bot_2", "user-a", "DESKTOP"),
            ("bot_3", "user-b", "DEVICE"),
        ]
    );
    assert!(rows.iter().all(|turn| turn.status == STORED_QUEUED
        && turn.attempts == 0
        && turn.hops == 0
        && turn.trigger_message_id == "m1"
        && turn.conversation_id == "c1"
        && turn.created_at == at(1)));

    let mut from_bot = delivery(
        ConversationKind::Direct,
        vec![recipient("bot_1", "user-a", SERVER)],
    );
    from_bot.sender_id = "bot_1".into();
    assert_eq!(enqueue_in(&db, &from_bot, at(2)).await.unwrap(), 0);
    assert_eq!(turns(&db).await.len(), 3);
}

#[tokio::test]
async fn in_a_group_only_a_mentioned_or_replied_to_bot_takes_a_turn() {
    let db = database().await;
    let mut mentioned = recipient("bot_1", "user-a", SERVER);
    mentioned.mentioned = true;
    let mut replied = recipient("bot_2", "user-a", SERVER);
    replied.replied_to = true;
    let quiet = recipient("bot_3", "user-a", SERVER);
    let queued = enqueue_in(
        &db,
        &delivery(ConversationKind::Group, vec![mentioned, replied, quiet]),
        at(1),
    )
    .await
    .unwrap();
    assert_eq!(queued, 2);
    let mut bots: Vec<_> = turns(&db)
        .await
        .into_iter()
        .map(|turn| turn.bot_id)
        .collect();
    bots.sort();
    assert_eq!(bots, ["bot_1", "bot_2"]);
}

#[tokio::test]
async fn a_persons_unaddressed_message_wakes_every_reading_bot_but_a_bots_message_wakes_none() {
    let db = database().await;
    let mut reader = recipient("bot_1", "user-a", SERVER);
    reader.read_along = true;
    let mut mention_only = recipient("bot_2", "user-a", SERVER);
    mention_only.read_along = false;
    let mut named = recipient("bot_3", "user-a", SERVER);
    named.read_along = false;
    named.mentioned = true;
    let found = delivery(
        ConversationKind::Group,
        vec![reader.clone(), mention_only.clone(), named.clone()],
    );
    for (recipient, wakes) in [(&reader, true), (&mention_only, false), (&named, true)] {
        assert_eq!(
            queues_turn(&found, recipient),
            wakes,
            "{}",
            recipient.bot_id
        );
    }
    assert!(reader.reads_along_only() && !named.reads_along_only());
    assert_eq!(enqueue_in(&db, &found, at(1)).await.unwrap(), 2);

    let mut by_bot = delivery(ConversationKind::Group, vec![]);
    by_bot.sender_id = "bot_9".into();
    let mut addressed = named.clone();
    addressed.hops = 1;
    reader.read_along = false;
    assert!(!queues_turn(&by_bot, &reader));
    assert!(queues_turn(&by_bot, &addressed));
    addressed.hops = MAX_HOPS + 1;
    assert!(!queues_turn(&by_bot, &addressed));
}

#[test]
fn a_reaction_wakes_only_the_bot_it_reacts_to_in_a_group_or_a_direct_conversation() {
    for kind in [ConversationKind::Group, ConversationKind::Direct] {
        let mut reacted = recipient("bot_1", "user-a", SERVER);
        reacted.reacted = true;
        let mut other = recipient("bot_2", "user-a", SERVER);
        other.read_along = true;
        other.mentioned = true;
        let mut found = delivery(kind, vec![reacted.clone(), other.clone()]);
        found.kind = InboxEntryKind::Reaction;
        assert!(queues_turn(&found, &reacted), "{kind:?}");
        assert!(!queues_turn(&found, &other), "{kind:?}");
        found.sender_id = "bot_1".into();
        assert!(!queues_turn(&found, &reacted), "a bot's own reaction");
    }
}

#[test]
fn direct_conversations_wake_the_bot_whether_or_not_it_reads_along() {
    let mut quiet = recipient("bot_1", "user-a", SERVER);
    quiet.read_along = false;
    let found = delivery(ConversationKind::Direct, vec![quiet.clone()]);
    assert!(queues_turn(&found, &quiet));
}

#[tokio::test]
async fn messages_carry_hops_and_the_cap_itself_still_queues() {
    let db = database().await;
    let mut at_cap = recipient("bot_1", "user-a", SERVER);
    at_cap.hops = MAX_HOPS;
    let mut past_cap = recipient("bot_2", "user-a", SERVER);
    past_cap.hops = MAX_HOPS + 1;
    let queued = enqueue_in(
        &db,
        &delivery(ConversationKind::Direct, vec![at_cap, past_cap]),
        at(1),
    )
    .await
    .unwrap();
    assert_eq!(queued, 1);
    let rows = turns(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].bot_id.as_str(), rows[0].hops),
        ("bot_1", MAX_HOPS as i32)
    );
}

#[tokio::test]
async fn only_messages_wake_bots() {
    let db = database().await;
    for kind in InboxEntryKind::ALL
        .iter()
        .copied()
        .filter(|kind| !matches!(kind, InboxEntryKind::Message | InboxEntryKind::Trigger))
    {
        let mut event = delivery(
            ConversationKind::Direct,
            vec![recipient("bot_1", "user-a", SERVER)],
        );
        event.kind = kind;
        assert_eq!(enqueue_in(&db, &event, at(1)).await.unwrap(), 0, "{kind:?}");
    }
    assert!(turns(&db).await.is_empty());
}

#[tokio::test]
async fn a_transcript_wakes_only_the_bots_it_mentions() {
    let db = database().await;
    for kind in [ConversationKind::Direct, ConversationKind::Group] {
        let mut named = recipient("bot_1", "user-a", SERVER);
        named.mentioned = true;
        let mut other = recipient("bot_2", "user-b", SERVER);
        other.read_along = true;
        let mut event = delivery(kind, vec![named.clone(), other.clone()]);
        event.kind = InboxEntryKind::Transcript;
        assert!(queues_turn(&event, &named), "{kind:?}");
        assert!(!queues_turn(&event, &other), "{kind:?}");
    }
    let mut event = delivery(
        ConversationKind::Group,
        vec![recipient("bot_1", "user-a", SERVER)],
    );
    event.kind = InboxEntryKind::Call;
    let mut named = recipient("bot_1", "user-a", SERVER);
    named.mentioned = true;
    assert!(!queues_turn(&event, &named), "a call wakes no one");
    assert_eq!(enqueue_in(&db, &event, at(1)).await.unwrap(), 0);
}

#[tokio::test]
async fn a_message_queues_its_turn_once() {
    let db = database().await;
    let event = delivery(
        ConversationKind::Direct,
        vec![recipient("bot_1", "user-a", SERVER)],
    );
    assert_eq!(enqueue_in(&db, &event, at(1)).await.unwrap(), 1);
    assert_eq!(enqueue_in(&db, &event, at(9)).await.unwrap(), 0);
    let rows = turns(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].created_at, at(1));
    assert_eq!(rows[0].id, turn_id("bot_1", "m1"));
}

#[tokio::test]
async fn the_server_takes_no_turn_in_an_encrypted_conversation_but_hosts_that_hold_keys_do() {
    let db = database().await;
    conversation(&db, "c1", "DIRECT", "E2E", &["user-a", "bot_1", "bot_2"]).await;
    let recipients = vec![
        recipient("bot_1", "user-a", SERVER),
        recipient("bot_2", "user-a", BotLocation::Desktop),
    ];
    assert_eq!(
        enqueue_in(&db, &delivery(ConversationKind::Direct, recipients), at(1))
            .await
            .unwrap(),
        1
    );
    assert_eq!(turns(&db).await[0].bot_id, "bot_2");
}

#[tokio::test]
async fn a_claim_takes_the_newest_queued_turn_and_folds_the_older_ones_into_it() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    for (n, trigger) in ["m1", "m2", "m3"].into_iter().enumerate() {
        queue(
            &db,
            "bot_1",
            "user-a",
            SERVER,
            "c1",
            trigger,
            at(n as i64 + 1),
        )
        .await;
    }
    let claimed = claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();
    assert_eq!(claimed.turn.id, turn_id("bot_1", "m3"));
    assert_eq!(claimed.batched, ["m1", "m2"]);
    assert_eq!(
        (
            claimed.turn.status.as_str(),
            claimed.turn.host_id.as_deref(),
            claimed.turn.attempts
        ),
        (STORED_RUNNING, Some("h1"), 1)
    );
    assert_eq!(
        claimed.turn.lease_until,
        Some(at(10) + Duration::seconds(LEASE_SECS))
    );
    for trigger in ["m1", "m2"] {
        let folded = row(&db, "bot_1", trigger).await;
        assert_eq!(folded.status, STORED_CANCELLED);
        assert_eq!(folded.error_code.as_deref(), Some(ERROR_BATCHED));
        assert_eq!(
            folded.reply_message_id,
            Some(reply_message_id("bot_1", "m3"))
        );
    }
    assert_eq!(
        claim(&db, "bot_1", "m2", "h2", at(11)).await.unwrap_err(),
        Refusal::NotQueued(TurnStatus::Cancelled)
    );
}

#[tokio::test]
async fn the_holding_host_may_claim_again_and_another_host_is_refused() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();
    let again = claim(&db, "bot_1", "m1", "h1", at(11)).await.unwrap();
    assert_eq!(again.turn.attempts, 1);
    assert_eq!(
        claim(&db, "bot_1", "m1", "h2", at(12)).await.unwrap_err(),
        Refusal::NotQueued(TurnStatus::Running)
    );
}

#[tokio::test]
async fn the_claim_is_a_compare_and_set() {
    let db = database().await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let id = turn_id("bot_1", "m1");
    assert!(take_in(&db, &id, "h1", at(10)).await.unwrap());
    assert!(
        !take_in(&db, &id, "h2", at(11)).await.unwrap(),
        "a live lease is not up for grabs"
    );
    let held = row(&db, "bot_1", "m1").await;
    assert_eq!((held.host_id.as_deref(), held.attempts), (Some("h1"), 1));
    let later = at(10) + Duration::seconds(LEASE_SECS + 1);
    assert!(
        take_in(&db, &id, "h2", later).await.unwrap(),
        "an expired lease is"
    );
    let taken = row(&db, "bot_1", "m1").await;
    assert_eq!((taken.host_id.as_deref(), taken.attempts), (Some("h2"), 2));
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
        .exec(&db)
        .await
        .unwrap();
    assert!(
        !take_in(&db, &id, "h3", later + Duration::seconds(500))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_bot_runs_one_turn_per_conversation_and_others_run_beside_it() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    queue(&db, "bot_1", "user-a", SERVER, "c2", "m9", at(1)).await;
    claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();
    let c1 = row(&db, "bot_1", "m1").await;
    assert_eq!(c1.status, STORED_RUNNING);
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m2", at(11)).await;
    assert_eq!(
        claim(&db, "bot_1", "m2", "h2", at(12)).await.unwrap_err(),
        Refusal::Busy
    );
    let other = claim_in(
        &db,
        &ClaimRequest {
            turn_id: bot_turn::Entity::find()
                .filter(bot_turn::Column::ConversationId.eq("c2"))
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .id,
            ..claim_request("", "h2", TurnHostKind::Server, at(12))
        },
    )
    .await
    .unwrap();
    assert!(other.is_ok(), "{other:?}");
    assert!(
        fail_held_in(&db, &c1.id, "h1", "BOT_TURN_FAILED", at(13))
            .await
            .unwrap()
    );
    assert!(claim(&db, "bot_1", "m2", "h2", at(14)).await.is_ok());
}

#[tokio::test]
async fn an_owner_runs_three_turns_at_most_and_others_are_not_held_back() {
    let db = database().await;
    for n in 1..=4 {
        let bot = format!("bot_{n}");
        insert_bot(&db, &bot, "user-a", "SERVER").await;
        queue(&db, &bot, "user-a", SERVER, &format!("c{n}"), "m1", at(1)).await;
    }
    insert_bot(&db, "bot_9", "user-b", "SERVER").await;
    queue(&db, "bot_9", "user-b", SERVER, "c9", "m1", at(1)).await;
    assert_eq!(MAX_RUNNING_PER_OWNER, 3);
    for n in 1..=3 {
        assert!(
            claim(&db, &format!("bot_{n}"), "m1", "h1", at(10))
                .await
                .is_ok(),
            "bot_{n}"
        );
    }
    assert_eq!(
        claim(&db, "bot_4", "m1", "h1", at(11)).await.unwrap_err(),
        Refusal::OwnerBusy
    );
    assert!(claim(&db, "bot_9", "m1", "h1", at(11)).await.is_ok());
    assert!(
        fail_held_in(
            &db,
            &turn_id("bot_1", "m1"),
            "h1",
            "BOT_TURN_FAILED",
            at(12)
        )
        .await
        .unwrap()
    );
    assert!(claim(&db, "bot_4", "m1", "h1", at(13)).await.is_ok());
}

#[tokio::test]
async fn an_expired_lease_is_taken_over_until_the_attempts_run_out() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let mut now = at(10);
    for attempt in 1..=MAX_ATTEMPTS {
        let claimed = claim(&db, "bot_1", "m1", &format!("h{attempt}"), now)
            .await
            .unwrap();
        assert_eq!(claimed.turn.attempts, attempt as i32);
        now += Duration::seconds(LEASE_SECS + 1);
    }
    assert_eq!(
        claim(&db, "bot_1", "m1", "h9", now).await.unwrap_err(),
        Refusal::NotQueued(TurnStatus::Failed)
    );
    let failed = row(&db, "bot_1", "m1").await;
    assert_eq!(failed.status, STORED_FAILED);
    assert_eq!(failed.error_code.as_deref(), Some(ERROR_HOST_LOST));
}

#[tokio::test]
async fn a_claim_checks_ownership_host_kind_bot_and_retry_time() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let id = turn_id("bot_1", "m1");

    let foreign = ClaimRequest {
        owner_id: Some("user-b".into()),
        ..claim_request(&id, "h1", TurnHostKind::Server, at(10))
    };
    assert_eq!(
        claim_in(&db, &foreign).await.unwrap().unwrap_err(),
        Refusal::NotFound
    );
    let unknown = claim_request("bt_nope", "h1", TurnHostKind::Server, at(10));
    assert_eq!(
        claim_in(&db, &unknown).await.unwrap().unwrap_err(),
        Refusal::NotFound
    );
    let desktop = claim_request(&id, "h1", TurnHostKind::Desktop, at(10));
    assert_eq!(
        claim_in(&db, &desktop).await.unwrap().unwrap_err(),
        Refusal::HostKind(TurnHostKind::Server)
    );
    assert_eq!(row(&db, "bot_1", "m1").await.status, STORED_QUEUED);

    claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();
    let retry_at = at(30);
    assert!(
        release_for_retry_in(&db, &id, "h1", retry_at, "BOT_TURN_MODEL_ERROR", at(20))
            .await
            .unwrap()
    );
    assert_eq!(
        claim(&db, "bot_1", "m1", "h2", at(25)).await.unwrap_err(),
        Refusal::NotYet
    );
    assert!(claim(&db, "bot_1", "m1", "h2", at(30)).await.is_ok());
    assert_eq!(row(&db, "bot_1", "m1").await.attempts, 2);
}

#[tokio::test]
async fn a_deleted_bots_turn_is_cancelled_when_claimed_and_by_recovery() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_2", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    queue(&db, "bot_2", "user-a", SERVER, "c2", "m1", at(1)).await;
    queue(&db, "bot_2", "user-a", SERVER, "c3", "m3", at(1)).await;
    claim(&db, "bot_2", "m1", "h1", at(5)).await.ok();
    delete_bot(&db, "bot_1").await;
    delete_bot(&db, "bot_2").await;

    assert_eq!(
        claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap_err(),
        Refusal::BotGone
    );
    let cancelled = row(&db, "bot_1", "m1").await;
    assert_eq!(cancelled.status, STORED_CANCELLED);
    assert_eq!(cancelled.error_code.as_deref(), Some(ERROR_BOT_DELETED));

    let recovered = recover_in(&db, at(11)).await.unwrap();
    assert_eq!(recovered.cancelled, 2);
    assert!(
        turns(&db)
            .await
            .iter()
            .all(|turn| turn.status == STORED_CANCELLED && turn.lease_until.is_none())
    );
}

#[tokio::test]
async fn a_claim_moves_a_turn_whose_bot_changed_hosts() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    bot::Entity::update_many()
        .col_expr(bot::Column::Location, Expr::value("DESKTOP"))
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(
        claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap_err(),
        Refusal::HostKind(TurnHostKind::Desktop)
    );
    assert_eq!(row(&db, "bot_1", "m1").await.host_kind, "DESKTOP");
}

#[tokio::test]
async fn a_heartbeat_extends_the_lease_of_its_host_for_its_owner_only() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let id = turn_id("bot_1", "m1");
    claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();

    let extended = heartbeat_in(&db, &id, Some("user-a"), "h1", at(40))
        .await
        .unwrap();
    assert_eq!(
        extended.lease_until,
        Some(at(40) + Duration::seconds(LEASE_SECS))
    );

    let other_host = heartbeat_in(&db, &id, Some("user-a"), "h2", at(41))
        .await
        .unwrap_err();
    assert_code(&other_host, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
    assert!(says(&other_host, "another host"));
    let foreign = heartbeat_in(&db, &id, Some("user-b"), "h1", at(41))
        .await
        .unwrap_err();
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let expired = heartbeat_in(&db, &id, Some("user-a"), "h1", at(40 + LEASE_SECS + 1))
        .await
        .unwrap_err();
    assert_code(&expired, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
    assert!(says(&expired, "lease ran out"));
    assert!(
        heartbeat_in(&db, "bt_nope", None, "h1", at(41))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recovery_requeues_lost_hosts_fails_the_spent_and_the_stale() {
    let db = database().await;
    for bot in ["bot_1", "bot_2", "bot_3", "bot_4"] {
        insert_bot(&db, bot, "user-a", "DESKTOP").await;
    }
    let desktop = BotLocation::Desktop;
    queue(&db, "bot_1", "user-a", desktop, "c1", "m1", at(1)).await;
    queue(&db, "bot_2", "user-a", desktop, "c2", "m1", at(1)).await;
    queue(&db, "bot_3", "user-a", desktop, "c3", "m1", at(1)).await;
    queue(&db, "bot_4", "user-a", desktop, "c4", "m1", at(2000)).await;
    let lost =
        |bot: &str| claim_request(&turn_id(bot, "m1"), "desk", TurnHostKind::Desktop, at(10));
    for bot in ["bot_1", "bot_2"] {
        claim_in(&db, &lost(bot)).await.unwrap().unwrap();
    }
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Attempts, Expr::value(MAX_ATTEMPTS as i32))
        .filter(bot_turn::Column::Id.eq(turn_id("bot_2", "m1")))
        .exec(&db)
        .await
        .unwrap();
    bot_turn::Entity::update_many()
        .col_expr(
            bot_turn::Column::UpdatedAt,
            Expr::value(at(QUEUED_EXPIRY_SECS + 5)),
        )
        .filter(bot_turn::Column::Id.eq(turn_id("bot_4", "m1")))
        .exec(&db)
        .await
        .unwrap();

    let now = at(QUEUED_EXPIRY_SECS + 100);
    let recovered = recover_in(&db, now).await.unwrap();
    assert_eq!(
        (
            recovered.requeued,
            recovered.lost,
            recovered.expired,
            recovered.cancelled
        ),
        (1, 1, 1, 0)
    );
    let requeued = row(&db, "bot_1", "m1").await;
    assert_eq!(
        (requeued.status.as_str(), requeued.host_id),
        (STORED_QUEUED, None)
    );
    assert!(requeued.lease_until.is_none());
    let spent = row(&db, "bot_2", "m1").await;
    assert_eq!(spent.status, STORED_FAILED);
    assert_eq!(spent.error_code.as_deref(), Some(ERROR_HOST_LOST));
    let stale = row(&db, "bot_3", "m1").await;
    assert_eq!(stale.status, STORED_FAILED);
    assert_eq!(stale.error_code.as_deref(), Some(ERROR_QUEUE_EXPIRED));
    assert_eq!(row(&db, "bot_4", "m1").await.status, STORED_QUEUED);
    assert_eq!(recover_in(&db, now).await.unwrap().requeued, 0);
}

#[tokio::test]
async fn an_owner_retries_only_their_own_failed_turn_of_that_bot() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let id = turn_id("bot_1", "m1");
    claim(&db, "bot_1", "m1", "h1", at(10)).await.unwrap();

    let running = retry_in(&db, "user-a", "bot_1", &id, at(20))
        .await
        .unwrap_err();
    assert_code(&running, StatusCode::CONFLICT, BOT_TURN_NOT_RETRYABLE);

    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
        .col_expr(bot_turn::Column::ReplyMessageId, Expr::value("rf-x"))
        .exec(&db)
        .await
        .unwrap();
    assert!(
        mark_failed_after_reply_in(&db, &id, "rf-other", "BOT_TURN_TIMEOUT", at(21))
            .await
            .map(|marked| !marked)
            .unwrap()
    );
    assert!(
        mark_failed_after_reply_in(&db, &id, "rf-x", "BOT_TURN_TIMEOUT", at(21))
            .await
            .unwrap()
    );
    let failed = row(&db, "bot_1", "m1").await;
    assert_eq!(
        (failed.status.as_str(), failed.error_code.as_deref()),
        (STORED_FAILED, Some("BOT_TURN_TIMEOUT"))
    );

    assert!(
        retry_in(&db, "user-b", "bot_1", &id, at(22))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        retry_in(&db, "user-a", "bot_2", &id, at(22))
            .await
            .unwrap()
            .is_none()
    );
    let queued = retry_in(&db, "user-a", "bot_1", &id, at(22))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (queued.status.as_str(), queued.attempts, queued.updated_at),
        (STORED_QUEUED, 0, at(22))
    );
    assert!(queued.reply_message_id.is_none() && queued.error_code.is_none());
    assert!(claim(&db, "bot_1", "m1", "h2", at(23)).await.is_ok());
}

#[tokio::test]
async fn listings_show_the_newest_waiting_turn_per_conversation() {
    let db = database().await;
    for (bot, owner, location) in [
        ("bot_1", "user-a", "DESKTOP"),
        ("bot_2", "user-a", "DESKTOP"),
        ("bot_3", "user-a", "DESKTOP"),
        ("bot_4", "user-b", "DESKTOP"),
        ("bot_5", "user-a", "SERVER"),
    ] {
        insert_bot(&db, bot, owner, location).await;
    }
    let desktop = BotLocation::Desktop;
    queue(&db, "bot_1", "user-a", desktop, "c1", "m1", at(1)).await;
    queue(&db, "bot_1", "user-a", desktop, "c1", "m2", at(2)).await;
    queue(&db, "bot_1", "user-a", desktop, "c2", "m3", at(3)).await;
    queue(&db, "bot_2", "user-a", desktop, "c1", "m1", at(4)).await;
    queue(&db, "bot_3", "user-a", desktop, "c1", "m1", at(5)).await;
    queue(&db, "bot_4", "user-b", desktop, "c1", "m1", at(6)).await;
    queue(&db, "bot_5", "user-a", SERVER, "c1", "m1", at(7)).await;
    delete_bot(&db, "bot_3").await;
    claim_in(
        &db,
        &claim_request(
            &turn_id("bot_2", "m1"),
            "desk",
            TurnHostKind::Desktop,
            at(10),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    queue(&db, "bot_2", "user-a", desktop, "c1", "m2", at(11)).await;

    let heads = queued_heads(&db, Some("user-a"), TurnHostKind::Desktop, at(12), 10)
        .await
        .unwrap();
    let listed: Vec<_> = heads
        .iter()
        .map(|(turn, batched)| {
            (
                turn.bot_id.as_str(),
                turn.trigger_message_id.as_str(),
                batched.clone(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("bot_1", "m2", vec!["m1".to_string()]),
            ("bot_1", "m3", vec![]),
        ]
    );
    let server = queued_heads(&db, None, TurnHostKind::Server, at(12), 10)
        .await
        .unwrap();
    assert_eq!(server.len(), 1);
    assert_eq!(server[0].0.bot_id, "bot_5");
    let everyone = queued_heads(&db, None, TurnHostKind::Desktop, at(12), 1)
        .await
        .unwrap();
    assert_eq!(everyone.len(), 1);
    let head = next_head(&db, "bot_1", "c1", TurnHostKind::Desktop, at(12))
        .await
        .unwrap();
    assert_eq!(head.unwrap().trigger_message_id, "m2");
}

#[tokio::test]
async fn a_turn_is_watched_by_its_bots_owner_and_the_conversations_people() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    conversation(&db, "c1", "GROUP", "PLAIN", &["user-b", "bot_1", "user-c"]).await;
    queue(&db, "bot_1", "user-a", SERVER, "c1", "m1", at(1)).await;
    let id = turn_id("bot_1", "m1");
    assert!(
        watched_turn(&db, "user-a", "c1", &id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        watched_turn(&db, "user-b", "c1", &id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        watched_turn(&db, "user-d", "c1", &id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        watched_turn(&db, "user-b", "c2", &id)
            .await
            .unwrap()
            .is_none()
    );
    crate::entity::conversation_member::Entity::update_many()
        .col_expr(
            crate::entity::conversation_member::Column::LeftAt,
            Expr::value(at(5)),
        )
        .filter(crate::entity::conversation_member::Column::PrincipalId.eq("user-b"))
        .exec(&db)
        .await
        .unwrap();
    assert!(
        watched_turn(&db, "user-b", "c1", &id)
            .await
            .unwrap()
            .is_none()
    );
}

/// A ledger whose owner `user-a` has Flux (`bot_flux`) and a plain server bot (`bot_plain`).
async fn handoff_db() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotHostHeartbeat" (
            "botId" TEXT NOT NULL, "hostId" TEXT NOT NULL, "hostKind" TEXT NOT NULL,
            "lastSeenAt" TEXT NOT NULL, PRIMARY KEY ("botId", "hostId"))"#,
    )
    .await
    .unwrap();
    insert_flux(&db, "bot_flux", "user-a").await;
    insert_bot(&db, "bot_plain", "user-a", "SERVER").await;
    db
}

async fn desktop_saw_flux(db: &DatabaseConnection, now: DateTime<FixedOffset>) {
    let report = report_in(
        db,
        "user-a",
        "desk-1",
        TurnHostKind::Desktop,
        &["bot_flux".to_string()],
        now,
    )
    .await
    .unwrap();
    assert_eq!(report.accepted, ["bot_flux"]);
}

/// Flux and the plain bot receive `trigger` in `c1`.
async fn both_receive(db: &DatabaseConnection, trigger: &str, now: DateTime<FixedOffset>) {
    let recipients = ["bot_flux", "bot_plain"]
        .into_iter()
        .map(|bot_id| BotRecipient {
            trigger_message_id: trigger.into(),
            hands_off: bot_id == "bot_flux",
            ..recipient(bot_id, "user-a", SERVER)
        })
        .collect();
    let queued = enqueue_in(db, &delivery(ConversationKind::Direct, recipients), now)
        .await
        .unwrap();
    assert_eq!(queued, 2, "{trigger}");
}

async fn host_kind(db: &DatabaseConnection, bot_id: &str, trigger: &str) -> String {
    row(db, bot_id, trigger).await.host_kind
}

#[tokio::test]
async fn flux_s_turns_go_to_the_owner_s_desktop_while_it_is_online() {
    let db = handoff_db().await;
    desktop_saw_flux(&db, at(0)).await;

    both_receive(&db, "m1", at(ONLINE_WINDOW_SECS)).await;
    assert_eq!(host_kind(&db, "bot_flux", "m1").await, "DESKTOP");
    assert_eq!(host_kind(&db, "bot_plain", "m1").await, "SERVER");

    both_receive(&db, "m2", at(ONLINE_WINDOW_SECS + 1)).await;
    assert_eq!(host_kind(&db, "bot_flux", "m2").await, "SERVER");
}

#[tokio::test]
async fn only_the_desktop_claims_a_turn_flux_handed_off() {
    let db = handoff_db().await;
    desktop_saw_flux(&db, at(0)).await;
    both_receive(&db, "m1", at(1)).await;
    let id = turn_id("bot_flux", "m1");

    assert_eq!(
        claim(&db, "bot_flux", "m1", "server", at(2))
            .await
            .unwrap_err(),
        Refusal::HostKind(TurnHostKind::Desktop)
    );
    let claimed = claim_in(
        &db,
        &claim_request(&id, "desk-1", TurnHostKind::Desktop, at(3)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(claimed.turn.host_id.as_deref(), Some("desk-1"));

    let plain = turn_id("bot_plain", "m1");
    assert_eq!(
        claim_in(
            &db,
            &claim_request(&plain, "desk-1", TurnHostKind::Desktop, at(3))
        )
        .await
        .unwrap()
        .unwrap_err(),
        Refusal::HostKind(TurnHostKind::Server)
    );
}

#[tokio::test]
async fn the_server_takes_back_a_handed_off_turn_no_desktop_claimed() {
    let db = handoff_db().await;
    insert_bot(&db, "bot_desk", "user-a", "DESKTOP").await;
    desktop_saw_flux(&db, at(0)).await;
    both_receive(&db, "m1", at(1)).await;
    queue(
        &db,
        "bot_desk",
        "user-a",
        BotLocation::Desktop,
        "c1",
        "m1",
        at(1),
    )
    .await;

    let waiting = hand_back_in(&db, at(1 + DESKTOP_HANDOFF_SECS))
        .await
        .unwrap();
    assert!(waiting.is_empty(), "{waiting:?}");
    assert_eq!(host_kind(&db, "bot_flux", "m1").await, "DESKTOP");

    let late = at(2 + DESKTOP_HANDOFF_SECS);
    let handed_back = hand_back_in(&db, late).await.unwrap();
    assert_eq!(handed_back, [turn_id("bot_flux", "m1")]);
    let taken = row(&db, "bot_flux", "m1").await;
    assert_eq!(
        (
            taken.host_kind.as_str(),
            taken.status.as_str(),
            taken.updated_at
        ),
        ("SERVER", STORED_QUEUED, late)
    );
    assert_eq!(host_kind(&db, "bot_desk", "m1").await, "DESKTOP");
    assert!(claim(&db, "bot_flux", "m1", "server", late).await.is_ok());
}
