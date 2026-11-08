//! Durable waits for one room participant. Sends and parking share the room's transaction lock.

use super::ledger;
use crate::{
    entity::{bot, bot_decision_wait, bot_turn, conversation, conversation_member, inbox_entry},
    error::ApiError,
    routes::inbox::{delivery::BotDelivery, now_millis},
    state::AppState,
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot_message::MessageBody,
    bot_turn::{ERROR_SILENT, MAX_HOPS, STORED_DONE, STORED_QUEUED},
    inbox::{ConversationKind, InboxEntryKind, TranscriptBody, is_tombstone},
};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, OnConflict},
};
use std::collections::{HashMap, HashSet};

const WAIT_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParkOutcome {
    Parked,
    AlreadyAnswered,
    Unavailable,
    Superseded,
    LeaseLost,
}

async fn active<C: ConnectionTrait>(db: &C, room: &str, participant: &str) -> Result<bool, DbErr> {
    Ok(
        conversation_member::Entity::find_by_id((room.to_owned(), participant.to_owned()))
            .filter(conversation_member::Column::LeftAt.is_null())
            .filter(conversation_member::Column::Status.eq("ACTIVE"))
            .one(db)
            .await?
            .is_some(),
    )
}

async fn continuous_member<C: ConnectionTrait>(
    db: &C,
    wait: &bot_decision_wait::Model,
    participant: &str,
) -> Result<bool, DbErr> {
    Ok(conversation_member::Entity::find_by_id((
        wait.conversation_id.clone(),
        participant.to_owned(),
    ))
    .filter(conversation_member::Column::LeftAt.is_null())
    .filter(conversation_member::Column::Status.eq("ACTIVE"))
    .filter(conversation_member::Column::JoinedAt.lte(wait.created_at))
    .one(db)
    .await?
    .is_some())
}

async fn enabled<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<bool, DbErr> {
    Ok(bot::Entity::find_by_id(bot_id)
        .one(db)
        .await?
        .is_some_and(|row| {
            row.deleted_at.is_none()
                && row
                    .model_policy
                    .get("decision")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty())
        }))
}

async fn remove<C: ConnectionTrait>(db: &C, wait: &bot_decision_wait::Model) -> Result<(), DbErr> {
    bot_decision_wait::Entity::delete_by_id((wait.bot_id.clone(), wait.conversation_id.clone()))
        .filter(bot_decision_wait::Column::TriggerMessageId.eq(&wait.trigger_message_id))
        .filter(bot_decision_wait::Column::CreatedAt.eq(wait.created_at))
        .exec(db)
        .await?;
    Ok(())
}

/// An explicit continuation supersedes the room dependency that preceded it. Use the stored
/// event time so replaying an old continuation cannot erase a later decision to wait.
pub(crate) async fn clear_before_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    cutoff: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    bot_decision_wait::Entity::delete_by_id((bot_id.to_owned(), conversation_id.to_owned()))
        .filter(bot_decision_wait::Column::CreatedAt.lte(cutoff))
        .exec(db)
        .await?;
    Ok(())
}

async fn participant_input<C: ConnectionTrait>(
    db: &C,
    entry: &inbox_entry::Model,
) -> Result<bool, DbErr> {
    if is_tombstone(&entry.envelope) {
        return Ok(false);
    }
    let Some(body) = entry.envelope.get("body") else {
        return Ok(false);
    };
    if flow_like::flow::copilot::decision::is_acknowledgement_body(body)
        && bot::Entity::find_by_id(&entry.sender_id)
            .one(db)
            .await?
            .is_some()
    {
        return Ok(false);
    }
    Ok(match entry.kind.as_str() {
        "MESSAGE" => MessageBody::parse(body).is_ok_and(|message| {
            message.approval.is_none()
                && message.approval_response.is_none()
                && (!message.text.trim().is_empty() || !message.parts.is_empty())
        }),
        "TRANSCRIPT" => TranscriptBody::parse(body).is_ok(),
        _ => false,
    })
}

/// Overrides read-along routing only while a participant wait is active. A direct address
/// clears it. Matching replies wake just the waiting bot, including unmentioned bot replies.
pub(crate) async fn routing<C: ConnectionTrait>(
    db: &C,
    delivery: &BotDelivery,
    now: DateTime<FixedOffset>,
) -> Result<HashMap<String, bool>, ApiError> {
    let mut result = HashMap::new();
    if delivery.conversation_kind != ConversationKind::Group || delivery.recipients.is_empty() {
        return Ok(result);
    }
    let room = &delivery.recipients[0].conversation_id;
    let waits = bot_decision_wait::Entity::find()
        .filter(bot_decision_wait::Column::ConversationId.eq(room))
        .filter(
            bot_decision_wait::Column::BotId.is_in(delivery.recipients.iter().map(|r| &r.bot_id)),
        )
        .all(db)
        .await?;
    for wait in waits {
        let Some(recipient) = delivery.recipients.iter().find(|r| r.bot_id == wait.bot_id) else {
            continue;
        };
        if wait.expires_at <= now
            || !enabled(db, &wait.bot_id).await?
            || !continuous_member(db, &wait, &wait.participant_id).await?
            || !continuous_member(db, &wait, &wait.bot_id).await?
        {
            remove(db, &wait).await?;
            continue;
        }
        if delivery.kind == InboxEntryKind::Topic && delivery.seq > wait.after_seq {
            remove(db, &wait).await?;
            result.insert(wait.bot_id, false);
            continue;
        }
        if recipient.hops > MAX_HOPS {
            result.insert(wait.bot_id, false);
            continue;
        }
        let speaker = delivery.sender_id == wait.participant_id && delivery.seq > wait.after_seq;
        if (speaker || recipient.mentioned || recipient.replied_to)
            && matches!(
                delivery.kind,
                InboxEntryKind::Message | InboxEntryKind::Transcript
            )
        {
            let entry = inbox_entry::Entity::find()
                .filter(inbox_entry::Column::InboxId.eq(&wait.bot_id))
                .filter(inbox_entry::Column::ConversationId.eq(room))
                .filter(inbox_entry::Column::MessageId.eq(&recipient.trigger_message_id))
                .one(db)
                .await?;
            let input = match entry.as_ref() {
                Some(entry) => participant_input(db, entry).await?,
                None => false,
            };
            if !input {
                result.insert(wait.bot_id, false);
                continue;
            }
        }
        if recipient.mentioned || recipient.replied_to {
            remove(db, &wait).await?;
            continue;
        }
        let matches = matches!(
            delivery.kind,
            InboxEntryKind::Message | InboxEntryKind::Transcript
        ) && speaker;
        if matches {
            remove(db, &wait).await?;
        }
        result.insert(wait.bot_id, matches);
    }
    Ok(result)
}

/// Takes the same room lock as a send, so a response arriving during inference cannot be lost.
pub(crate) async fn park_wait(
    state: &AppState,
    turn: &bot_turn::Model,
    host_id: &str,
    participant_id: &str,
) -> Result<ParkOutcome, ApiError> {
    let turn = turn.clone();
    let host_id = host_id.to_owned();
    let participant_id = participant_id.to_owned();
    state
        .transaction(|txn| {
            let (turn, host_id, participant_id) =
                (turn.clone(), host_id.clone(), participant_id.clone());
            Box::pin(async move {
                crate::routes::inbox::approvals::lock(txn, &turn.conversation_id).await?;
                park_wait_in(txn, &turn, &host_id, &participant_id, now_millis()).await
            })
        })
        .await
}

/// The caller holds the room lock and commits this operation in one transaction.
pub(crate) async fn park_wait_in<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    participant_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<ParkOutcome, ApiError> {
    if !ledger::holds_lease(db, &turn.id, host_id, now).await? {
        return Ok(ParkOutcome::LeaseLost);
    }
    let room = &turn.conversation_id;
    if !enabled(db, &turn.bot_id).await?
        || participant_id == turn.bot_id
        || !active(db, room, participant_id).await?
        || !active(db, room, &turn.bot_id).await?
        || !conversation::Entity::find_by_id(room)
            .one(db)
            .await?
            // Encrypted messages hide mentions and reply links from this router, so the hub
            // cannot preserve the explicit-address override while a participant wait is active.
            .is_some_and(|row| row.kind == "GROUP" && row.encryption == "PLAIN")
    {
        return Ok(ParkOutcome::Unavailable);
    }
    // A room-wide lock makes following these edges safe against two bots parking on each other.
    let mut target = participant_id.to_owned();
    let mut visited = HashSet::from([turn.bot_id.clone()]);
    loop {
        if !visited.insert(target.clone()) || visited.len() > 64 {
            return Ok(ParkOutcome::Unavailable);
        }
        let next = bot_decision_wait::Entity::find_by_id((target, room.clone()))
            .one(db)
            .await?;
        let Some(next) = next.filter(|wait| wait.expires_at > now) else {
            break;
        };
        target = next.participant_id;
    }
    let trigger = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&turn.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(room))
        .filter(inbox_entry::Column::MessageId.eq(&turn.trigger_message_id))
        .one(db)
        .await?;
    let Some(after_seq) = trigger.and_then(|entry| entry.seq) else {
        return Ok(ParkOutcome::Unavailable);
    };
    let newer = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&turn.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(room))
        .filter(inbox_entry::Column::Seq.gt(after_seq))
        .filter(inbox_entry::Column::Kind.is_in(["MESSAGE", "TRANSCRIPT", "TOPIC", "REACTION"]))
        .order_by_desc(inbox_entry::Column::Seq)
        .limit(100)
        .all(db)
        .await?;
    if newer
        .iter()
        .any(|entry| entry.kind == "TOPIC" && !is_tombstone(&entry.envelope))
    {
        return Ok(ParkOutcome::Superseded);
    }
    for entry in &newer {
        if !participant_input(db, entry).await? {
            continue;
        }
        let body = entry.envelope.get("body");
        let mentioned = body
            .and_then(|body| body.get("mentions"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|mentions| {
                mentions
                    .iter()
                    .any(|id| id.as_str() == Some(turn.bot_id.as_str()))
            });
        let reply_to = body
            .and_then(|body| body.get("reply_to"))
            .and_then(serde_json::Value::as_str);
        let replied = if let Some(target) = reply_to {
            inbox_entry::Entity::find()
                .filter(inbox_entry::Column::InboxId.eq(&turn.bot_id))
                .filter(inbox_entry::Column::ConversationId.eq(room))
                .filter(inbox_entry::Column::MessageId.eq(target))
                .filter(inbox_entry::Column::SenderId.eq(&turn.bot_id))
                .one(db)
                .await?
                .is_some()
        } else {
            false
        };
        if entry.sender_id == participant_id || mentioned || replied {
            return Ok(ParkOutcome::AlreadyAnswered);
        }
    }
    if newer.len() == 100 {
        return Ok(ParkOutcome::AlreadyAnswered);
    }
    if !ledger::complete_silent_in(db, &turn.id, host_id, now).await? {
        return Ok(ParkOutcome::LeaseLost);
    }
    // Unrelated messages may already have queued while the decision request was in flight.
    // They cannot claim while this turn runs, and must respect the newly installed wait.
    if !newer.is_empty() {
        bot_turn::Entity::update_many()
            .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
            .col_expr(bot_turn::Column::ErrorCode, Expr::value(ERROR_SILENT))
            .col_expr(bot_turn::Column::UpdatedAt, Expr::value(now))
            .filter(bot_turn::Column::BotId.eq(&turn.bot_id))
            .filter(bot_turn::Column::ConversationId.eq(room))
            .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
            .filter(
                bot_turn::Column::TriggerMessageId
                    .is_in(newer.iter().map(|entry| &entry.message_id)),
            )
            .exec(db)
            .await?;
    }
    bot_decision_wait::Entity::insert(bot_decision_wait::ActiveModel {
        bot_id: Set(turn.bot_id.clone()),
        conversation_id: Set(room.clone()),
        participant_id: Set(participant_id.to_owned()),
        trigger_message_id: Set(turn.trigger_message_id.clone()),
        after_seq: Set(after_seq),
        expires_at: Set(now + Duration::hours(WAIT_HOURS)),
        created_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            bot_decision_wait::Column::BotId,
            bot_decision_wait::Column::ConversationId,
        ])
        .update_columns([
            bot_decision_wait::Column::ParticipantId,
            bot_decision_wait::Column::TriggerMessageId,
            bot_decision_wait::Column::AfterSeq,
            bot_decision_wait::Column::ExpiresAt,
            bot_decision_wait::Column::CreatedAt,
        ])
        .to_owned(),
    )
    .exec(db)
    .await?;
    Ok(ParkOutcome::Parked)
}

pub(crate) async fn cleanup<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    bot_decision_wait::Entity::delete_many()
        .filter(bot_decision_wait::Column::ExpiresAt.lte(now))
        .exec(db)
        .await?;
    let waits = bot_decision_wait::Entity::find()
        .order_by_asc(bot_decision_wait::Column::CreatedAt)
        .limit(200)
        .all(db)
        .await?;
    for wait in waits {
        if !enabled(db, &wait.bot_id).await?
            || !continuous_member(db, &wait, &wait.participant_id).await?
            || !continuous_member(db, &wait, &wait.bot_id).await?
        {
            remove(db, &wait).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::{
        fixture::{at, conversation, database, delivery, insert_bot, message, recipient},
        ledger::{ClaimRequest, claim_in, enqueue_in},
    };
    use flow_like_types::{
        bot::BotLocation,
        bot_turn::{TurnHostKind, turn_id},
    };
    use sea_orm::{DatabaseConnection, TransactionTrait};
    use serde_json::json;

    async fn setup() -> (DatabaseConnection, bot_turn::Model) {
        let db = database().await;
        for id in ["bot_1", "bot_2"] {
            insert_bot(&db, id, "user-a", "SERVER").await;
            bot::Entity::update_many()
                .col_expr(
                    bot::Column::ModelPolicy,
                    Expr::value(json!({"decision":"decision-bit"})),
                )
                .filter(bot::Column::Id.eq(id))
                .exec(&db)
                .await
                .unwrap();
        }
        conversation(
            &db,
            "c1",
            "GROUP",
            "PLAIN",
            &["user-a", "user-b", "bot_1", "bot_2"],
        )
        .await;
        message(
            &db,
            "c1",
            "m1",
            "user-a",
            1,
            json!({"text":"Discuss this"}),
            &["bot_1", "bot_2"],
        )
        .await;
        let mut r = recipient("bot_1", "user-a", BotLocation::Server);
        r.read_along = true;
        enqueue_in(&db, &delivery(ConversationKind::Group, vec![r]), at(1))
            .await
            .unwrap();
        let turn = claim_in(
            &db,
            &ClaimRequest {
                turn_id: turn_id("bot_1", "m1"),
                owner_id: None,
                host_kind: TurnHostKind::Server,
                host_id: "host".into(),
                now: at(2),
            },
        )
        .await
        .unwrap()
        .unwrap()
        .turn;
        (db, turn)
    }

    async fn park(db: &DatabaseConnection, turn: &bot_turn::Model, who: &str) -> ParkOutcome {
        let txn = db.begin().await.unwrap();
        let result = park_wait_in(&txn, turn, "host", who, at(3)).await.unwrap();
        txn.commit().await.unwrap();
        result
    }

    fn event(sender: &str, seq: i64, kind: InboxEntryKind) -> BotDelivery {
        let mut r = recipient("bot_1", "user-a", BotLocation::Server);
        r.trigger_message_id = format!("m{seq}");
        r.read_along = sender.starts_with("user-");
        r.hops = u32::from(sender.starts_with("bot_"));
        let mut event = delivery(ConversationKind::Group, vec![r]);
        event.sender_id = sender.into();
        event.seq = seq;
        event.kind = kind;
        event
    }

    async fn incoming(
        db: &DatabaseConnection,
        event: &BotDelivery,
        now: DateTime<FixedOffset>,
    ) -> Result<u64, ApiError> {
        let recipient = &event.recipients[0];
        let exists = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(&recipient.bot_id))
            .filter(inbox_entry::Column::MessageId.eq(&recipient.trigger_message_id))
            .one(db)
            .await?
            .is_some();
        if !exists {
            let mentions: Vec<&str> = recipient
                .mentioned
                .then_some(recipient.bot_id.as_str())
                .into_iter()
                .collect();
            let body = if event.kind == InboxEntryKind::Transcript {
                json!({"text":"A spoken answer", "call":"call_1", "offset_ms":0, "duration_ms":100, "mentions":mentions})
            } else {
                json!({"text":"An answer", "mentions":mentions, "hops":recipient.hops})
            };
            crate::bot_host::fixture::entry(
                db,
                "c1",
                &recipient.trigger_message_id,
                &event.sender_id,
                event.seq,
                &event.kind.as_str().to_ascii_uppercase(),
                body,
                &[&recipient.bot_id],
            )
            .await;
        }
        enqueue_in(db, event, now).await
    }

    async fn stored(db: &DatabaseConnection) -> Option<bot_decision_wait::Model> {
        bot_decision_wait::Entity::find_by_id(("bot_1".into(), "c1".into()))
            .one(db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_wait_survives_turn_completion_and_only_its_participant_wakes_it() {
        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        let done = bot_turn::Entity::find_by_id(&turn.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status, STORED_DONE);
        assert_eq!(done.error_code.as_deref(), Some(ERROR_SILENT));
        assert!(done.lease_until.is_none());
        assert_eq!(
            incoming(&db, &event("user-b", 2, InboxEntryKind::Message), at(4))
                .await
                .unwrap(),
            0
        );
        assert!(stored(&db).await.is_some());
        assert_eq!(
            incoming(&db, &event("bot_2", 3, InboxEntryKind::Message), at(5))
                .await
                .unwrap(),
            1
        );
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn a_spoken_reply_wakes_the_waiter_without_a_mention() {
        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "user-b").await, ParkOutcome::Parked);
        let response = event("user-b", 2, InboxEntryKind::Transcript);
        assert_eq!(incoming(&db, &response, at(4)).await.unwrap(), 1);
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn a_direct_address_overrides_the_wait() {
        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        let mut addressed = event("user-b", 2, InboxEntryKind::Message);
        addressed.recipients[0].mentioned = true;
        assert_eq!(incoming(&db, &addressed, at(4)).await.unwrap(), 1);
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn a_reply_arriving_during_inference_is_not_missed() {
        let (db, turn) = setup().await;
        message(
            &db,
            "c1",
            "m2",
            "bot_2",
            2,
            json!({"text":"Here is the answer"}),
            &["bot_1"],
        )
        .await;
        assert_eq!(
            park(&db, &turn, "bot_2").await,
            ParkOutcome::AlreadyAnswered
        );
        assert!(stored(&db).await.is_none());
        assert_eq!(
            bot_turn::Entity::find_by_id(&turn.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            "RUNNING"
        );
    }

    #[tokio::test]
    async fn unrelated_queued_messages_also_respect_a_new_wait() {
        let (db, turn) = setup().await;
        message(
            &db,
            "c1",
            "m2",
            "user-b",
            2,
            json!({"text":"Unrelated"}),
            &["bot_1"],
        )
        .await;
        assert_eq!(
            incoming(&db, &event("user-b", 2, InboxEntryKind::Message), at(2))
                .await
                .unwrap(),
            1
        );
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        assert_eq!(
            bot_turn::Entity::find_by_id(turn_id("bot_1", "m2"))
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            STORED_DONE
        );
    }

    #[tokio::test]
    async fn expired_departed_and_disabled_waits_are_cleared() {
        for state in [
            "expired",
            "departed",
            "disabled",
            "rejoined",
            "bot_rejoined",
        ] {
            let (db, turn) = setup().await;
            assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
            let mut now = at(4);
            if state == "expired" {
                now += Duration::hours(24);
            }
            if state == "departed" {
                conversation_member::Entity::update_many()
                    .col_expr(conversation_member::Column::LeftAt, Expr::value(now))
                    .filter(conversation_member::Column::PrincipalId.eq("bot_2"))
                    .exec(&db)
                    .await
                    .unwrap();
            }
            if state == "disabled" {
                bot::Entity::update_many()
                    .col_expr(bot::Column::ModelPolicy, Expr::value(json!({})))
                    .filter(bot::Column::Id.eq("bot_1"))
                    .exec(&db)
                    .await
                    .unwrap();
            }
            if state == "rejoined" || state == "bot_rejoined" {
                let principal = if state == "rejoined" {
                    "bot_2"
                } else {
                    "bot_1"
                };
                conversation_member::Entity::update_many()
                    .col_expr(conversation_member::Column::JoinedAt, Expr::value(now))
                    .filter(conversation_member::Column::PrincipalId.eq(principal))
                    .exec(&db)
                    .await
                    .unwrap();
            }
            assert_eq!(
                incoming(&db, &event("user-b", 2, InboxEntryKind::Message), now)
                    .await
                    .unwrap(),
                1,
                "{state}"
            );
            assert!(stored(&db).await.is_none(), "{state}");
        }
    }

    #[tokio::test]
    async fn a_lost_lease_or_invalid_target_cannot_park() {
        let (db, turn) = setup().await;
        assert_eq!(
            park_wait_in(&db, &turn, "stranger", "bot_2", at(3))
                .await
                .unwrap(),
            ParkOutcome::LeaseLost
        );
        assert_eq!(park(&db, &turn, "bot_1").await, ParkOutcome::Unavailable);
        assert_eq!(park(&db, &turn, "unknown").await, ParkOutcome::Unavailable);
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn mutual_waits_cannot_deadlock_the_room() {
        let (db, turn) = setup().await;
        bot_decision_wait::Entity::insert(bot_decision_wait::ActiveModel {
            bot_id: Set("bot_2".into()),
            conversation_id: Set("c1".into()),
            participant_id: Set("bot_1".into()),
            trigger_message_id: Set("other".into()),
            after_seq: Set(1),
            expires_at: Set(at(100)),
            created_at: Set(at(2)),
        })
        .exec(&db)
        .await
        .unwrap();
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Unavailable);
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn sealed_groups_cannot_park_a_wait_the_hub_cannot_override() {
        let (db, turn) = setup().await;
        conversation::Entity::update_many()
            .col_expr(conversation::Column::Encryption, Expr::value("E2E"))
            .filter(conversation::Column::Id.eq("c1"))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(
            park_wait_in(&db, &turn, "host", "bot_2", at(3))
                .await
                .unwrap(),
            ParkOutcome::Unavailable
        );
        assert!(
            bot_decision_wait::Entity::find_by_id(("bot_1".into(), "c1".into()))
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            ledger::holds_lease(&db, &turn.id, "host", at(3))
                .await
                .unwrap()
        );
    }

    fn approval_body() -> serde_json::Value {
        json!({"text":"I need approval", "approval":{
            "id":"card", "turn_id":"turn", "tool":"tool", "summary":"Run tool",
            "scope":"app:test", "args_hash":"hash"
        }})
    }

    #[tokio::test]
    async fn approval_cards_do_not_resolve_or_prevent_a_participant_wait() {
        let (db, turn) = setup().await;
        let body = approval_body();
        assert!(MessageBody::parse(&body).is_ok());
        message(&db, "c1", "m2", "bot_2", 2, body.clone(), &["bot_1"]).await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        message(&db, "c1", "m3", "bot_2", 3, body, &["bot_1"]).await;
        assert_eq!(
            incoming(&db, &event("bot_2", 3, InboxEntryKind::Message), at(4))
                .await
                .unwrap(),
            0
        );
        assert!(stored(&db).await.is_some());
        message(
            &db,
            "c1",
            "m4",
            "bot_2",
            4,
            json!({"text":"The request failed", "error":{"code":"BOT_TURN_FAILED"}}),
            &["bot_1"],
        )
        .await;
        assert_eq!(
            incoming(&db, &event("bot_2", 4, InboxEntryKind::Message), at(5))
                .await
                .unwrap(),
            1
        );
        assert!(
            stored(&db).await.is_none(),
            "a terminal failure permits reconsidering the dependency"
        );
    }

    #[tokio::test]
    async fn bot_acknowledgements_do_not_release_waits_but_user_input_does() {
        let (db, turn) = setup().await;
        let acknowledgement = json!({"text":"Understood.", "system_one_acknowledgement":true});
        message(
            &db,
            "c1",
            "m2",
            "bot_2",
            2,
            acknowledgement.clone(),
            &["bot_1"],
        )
        .await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        message(
            &db,
            "c1",
            "m3",
            "bot_2",
            3,
            acknowledgement.clone(),
            &["bot_1"],
        )
        .await;
        assert_eq!(
            incoming(&db, &event("bot_2", 3, InboxEntryKind::Message), at(4))
                .await
                .unwrap(),
            0
        );
        assert!(stored(&db).await.is_some());

        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "user-b").await, ParkOutcome::Parked);
        message(&db, "c1", "m2", "user-b", 2, acknowledgement, &["bot_1"]).await;
        assert_eq!(
            incoming(&db, &event("user-b", 2, InboxEntryKind::Message), at(4))
                .await
                .unwrap(),
            1
        );
        assert!(stored(&db).await.is_none());
    }

    #[tokio::test]
    async fn a_new_topic_cancels_waiting_without_waking_the_bot() {
        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        assert_eq!(
            incoming(&db, &event("user-a", 2, InboxEntryKind::Topic), at(4))
                .await
                .unwrap(),
            0
        );
        assert!(stored(&db).await.is_none());
        assert_eq!(
            incoming(&db, &event("user-a", 3, InboxEntryKind::Message), at(5))
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn a_target_reply_above_the_hop_limit_does_not_clear_the_wait() {
        let (db, turn) = setup().await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        let mut reply = event("bot_2", 2, InboxEntryKind::Message);
        reply.recipients[0].mentioned = true;
        reply.recipients[0].hops = MAX_HOPS + 1;
        assert_eq!(incoming(&db, &reply, at(4)).await.unwrap(), 0);
        assert!(stored(&db).await.is_some());
    }

    #[tokio::test]
    async fn a_topic_started_during_inference_supersedes_the_old_decision() {
        let (db, turn) = setup().await;
        crate::bot_host::fixture::entry(
            &db,
            "c1",
            "new-topic",
            "user-a",
            2,
            "TOPIC",
            json!({"title":"Next topic"}),
            &["bot_1"],
        )
        .await;
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Superseded);
        assert!(stored(&db).await.is_none());
        assert!(
            ledger::holds_lease(&db, &turn.id, "host", at(3))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn a_reaction_queued_during_inference_also_respects_the_new_wait() {
        let (db, turn) = setup().await;
        crate::bot_host::fixture::entry(
            &db,
            "c1",
            "m2",
            "user-a",
            2,
            "REACTION",
            json!({"target":"m1", "emoji":"?", "add":true}),
            &["bot_1"],
        )
        .await;
        let mut reaction = event("user-a", 2, InboxEntryKind::Reaction);
        reaction.recipients[0].reacted = true;
        assert_eq!(incoming(&db, &reaction, at(2)).await.unwrap(), 1);
        assert_eq!(park(&db, &turn, "bot_2").await, ParkOutcome::Parked);
        let queued = bot_turn::Entity::find_by_id(turn_id("bot_1", "m2"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queued.status, STORED_DONE);
        assert_eq!(queued.error_code.as_deref(), Some(ERROR_SILENT));
        assert!(stored(&db).await.is_some());
    }
}
