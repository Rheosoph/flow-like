use super::{
    fixture::{assert_code, at, conversation, database, delivery, insert_bot, message, recipient},
    interactive::InteractiveTurn,
    ledger::{self, ClaimRequest, Refusal, claim_in, enqueue_in, hold_in, queued_heads},
};
use crate::{
    entity::{bot_turn, inbox_entry},
    routes::inbox::{
        BOT_TURN_NOT_CLAIMED, delivery::is_interactive, entries::validate_send, model::TurnClaim,
        turns::reply_interactive_in,
    },
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot::BotLocation,
    bot_message::MessageBody,
    bot_turn::{
        INTERACTIVE_GRACE_SECS, STORED_DONE, STORED_QUEUED, STORED_RUNNING, TurnHostKind,
        reply_message_id, turn_id,
    },
    inbox::ConversationKind,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::json;

async fn chat(db: &DatabaseConnection) {
    insert_bot(db, "bot_1", "user-a", "SERVER").await;
    conversation(db, "c1", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
    message(
        db,
        "c1",
        "m1",
        "user-a",
        1,
        json!({ "text": "hi" }),
        &["user-a", "bot_1"],
    )
    .await;
    db.execute_unprepared(r#"UPDATE public."Conversation" SET "lastSeq" = 1"#)
        .await
        .unwrap();
}

use sea_orm::ConnectionTrait;

async fn queue(db: &DatabaseConnection, interactive: bool, now: DateTime<FixedOffset>) {
    let mut recipient = recipient("bot_1", "user-a", BotLocation::Server);
    recipient.interactive = interactive;
    enqueue_in(
        db,
        &delivery(ConversationKind::Direct, vec![recipient]),
        now,
    )
    .await
    .unwrap();
}

fn claim(host: &str, now: DateTime<FixedOffset>) -> ClaimRequest {
    ClaimRequest {
        turn_id: turn_id("bot_1", "m1"),
        owner_id: None,
        host_kind: TurnHostKind::Server,
        host_id: host.into(),
        now,
    }
}

async fn waiting(db: &DatabaseConnection, now: DateTime<FixedOffset>) -> usize {
    queued_heads(db, None, TurnHostKind::Server, now, 10)
        .await
        .unwrap()
        .len()
}

async fn turn(db: &DatabaseConnection) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(turn_id("bot_1", "m1"))
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

fn interactive(turn: &bot_turn::Model) -> InteractiveTurn {
    InteractiveTurn {
        turn: turn.clone(),
        host_id: "interactive:run_1".into(),
        trail: None,
    }
}

#[test]
fn only_a_body_with_a_run_id_is_marked_interactive() {
    let marked = json!({ "v": 1, "enc": "none", "body": { "text": "x", "interactive": { "client_run_id": "run_1" } } });
    let plain = json!({ "v": 1, "enc": "none", "body": { "text": "x" } });
    let empty = json!({ "v": 1, "enc": "none", "body": { "text": "x", "interactive": { "client_run_id": "" } } });
    assert!(is_interactive(&marked));
    assert!(!is_interactive(&plain));
    assert!(!is_interactive(&empty));
    assert!(
        MessageBody::parse(&marked["body"])
            .unwrap()
            .interactive
            .is_some()
    );
    assert!(
        MessageBody::parse(
            &json!({ "text": "x", "interactive": { "client_run_id": "has space" } })
        )
        .is_err()
    );
}

#[tokio::test]
async fn a_marked_message_is_left_to_its_chat_until_the_grace_has_passed() {
    let db = database().await;
    chat(&db).await;
    queue(&db, true, at(100)).await;

    let queued = turn(&db).await;
    assert_eq!(queued.status, STORED_QUEUED);
    assert_eq!(queued.lease_until, Some(at(100 + INTERACTIVE_GRACE_SECS)));
    assert_eq!(waiting(&db, at(101)).await, 0);
    assert_eq!(waiting(&db, at(100 + INTERACTIVE_GRACE_SECS - 1)).await, 0);
    assert_eq!(
        claim_in(&db, &claim("host", at(160))).await.unwrap().err(),
        Some(Refusal::NotYet)
    );

    assert_eq!(waiting(&db, at(100 + INTERACTIVE_GRACE_SECS + 1)).await, 1);
    let taken = claim_in(&db, &claim("host", at(100 + INTERACTIVE_GRACE_SECS + 1)))
        .await
        .unwrap()
        .expect("the host takes over after the grace");
    assert_eq!(taken.turn.status, STORED_RUNNING);
}

#[tokio::test]
async fn an_unmarked_message_wakes_its_host_at_once() {
    let db = database().await;
    chat(&db).await;
    queue(&db, false, at(100)).await;
    assert!(turn(&db).await.lease_until.is_none());
    assert_eq!(waiting(&db, at(100)).await, 1);
}

#[tokio::test]
async fn holding_pushes_the_grace_on_until_a_host_claims() {
    let db = database().await;
    chat(&db).await;
    queue(&db, true, at(100)).await;
    let id = turn_id("bot_1", "m1");

    assert!(hold_in(&db, &id, "user-a", at(200)).await.unwrap());
    assert_eq!(
        turn(&db).await.lease_until,
        Some(at(200 + INTERACTIVE_GRACE_SECS))
    );
    assert_eq!(waiting(&db, at(100 + INTERACTIVE_GRACE_SECS + 1)).await, 0);
    assert!(
        !hold_in(&db, &id, "user-b", at(210)).await.unwrap(),
        "only the owner holds"
    );

    claim_in(&db, &claim("host", at(200 + INTERACTIVE_GRACE_SECS + 1)))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !hold_in(&db, &id, "user-a", at(400)).await.unwrap(),
        "a claimed turn is no longer held"
    );
}

#[tokio::test]
async fn the_chat_stores_its_reply_as_the_bot_and_ends_the_turn() {
    let db = database().await;
    chat(&db).await;
    queue(&db, true, at(100)).await;
    let held = turn(&db).await;
    let chat_turn = interactive(&held);

    let outgoing = validate_send(chat_turn.request("It is Tuesday.", None)).unwrap();
    let claim_row = TurnClaim {
        id: held.id.clone(),
        host_id: chat_turn.host_id.clone(),
    };
    let sent = reply_interactive_in(&db, "c1", "bot_1", &claim_row, &outgoing, at(130))
        .await
        .unwrap();

    assert_eq!(sent.entry.sender_id, "bot_1");
    assert_eq!(sent.entry.message_id, reply_message_id("bot_1", "m1"));
    assert_eq!(sent.entry.seq, Some(2));
    assert_eq!(sent.entry.envelope["body"]["text"], json!("It is Tuesday."));
    assert_eq!(sent.entry.envelope["body"]["reply_to"], json!("m1"));
    let done = turn(&db).await;
    assert_eq!(done.status, STORED_DONE);
    assert!(done.lease_until.is_none());
    assert_eq!(waiting(&db, at(1_000)).await, 0, "no host answers it again");
    assert_eq!(
        claim_in(&db, &claim("host", at(1_000)))
            .await
            .unwrap()
            .err(),
        Some(Refusal::NotQueued(
            flow_like_types::bot_turn::TurnStatus::Done
        ))
    );
}

#[tokio::test]
async fn a_chat_that_lost_the_turn_to_a_host_stores_no_second_reply() {
    let db = database().await;
    chat(&db).await;
    queue(&db, true, at(100)).await;
    let held = turn(&db).await;
    claim_in(&db, &claim("host", at(100 + INTERACTIVE_GRACE_SECS + 5)))
        .await
        .unwrap()
        .unwrap();

    let chat_turn = interactive(&held);
    let outgoing = validate_send(chat_turn.request("too late", None)).unwrap();
    let claim_row = TurnClaim {
        id: held.id.clone(),
        host_id: chat_turn.host_id.clone(),
    };
    let refused = reply_interactive_in(&db, "c1", "bot_1", &claim_row, &outgoing, at(400))
        .await
        .unwrap_err();
    assert_code(&refused, StatusCode::CONFLICT, BOT_TURN_NOT_CLAIMED);

    let written = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(reply_message_id("bot_1", "m1")))
        .all(&db)
        .await
        .unwrap();
    assert!(written.is_empty());
}

#[tokio::test]
async fn the_reply_id_is_the_hosts_so_a_repeat_returns_the_first_copy() {
    let db = database().await;
    chat(&db).await;
    queue(&db, true, at(100)).await;
    let held = turn(&db).await;
    let chat_turn = interactive(&held);
    let claim_row = TurnClaim {
        id: held.id.clone(),
        host_id: chat_turn.host_id.clone(),
    };
    let first = validate_send(chat_turn.request("one", None)).unwrap();
    let again = validate_send(chat_turn.request("two", None)).unwrap();
    let stored = reply_interactive_in(&db, "c1", "bot_1", &claim_row, &first, at(130))
        .await
        .unwrap();
    let repeat = reply_interactive_in(&db, "c1", "bot_1", &claim_row, &again, at(131))
        .await
        .unwrap();
    assert_eq!(stored.entry.id, repeat.entry.id);
    assert_eq!(repeat.entry.envelope["body"]["text"], json!("one"));
    let _ = ledger::LIST_LIMIT;
}

fn transcript_envelope() -> serde_json::Value {
    flow_like_types::inbox::plain_envelope(json!({
        "call": "call_1",
        "text": "what is the weather",
        "offset_ms": 0,
        "duration_ms": 1800,
        "mentions": ["bot_1"],
        "interactive": { "client_run_id": "run_1" },
    }))
}

async fn queue_transcript(db: &DatabaseConnection, mentioned: bool, now: DateTime<FixedOffset>) {
    let mut recipient = recipient("bot_1", "user-a", BotLocation::Server);
    recipient.interactive = is_interactive(&transcript_envelope());
    recipient.mentioned = mentioned;
    let mut delivery = delivery(ConversationKind::Direct, vec![recipient]);
    delivery.kind = flow_like_types::inbox::InboxEntryKind::Transcript;
    enqueue_in(db, &delivery, now).await.unwrap();
}

#[tokio::test]
async fn a_marked_transcript_is_held_for_the_chat_and_the_chats_reply_ends_the_turn() {
    let request = |envelope| crate::routes::inbox::model::SendEntryRequest {
        message_id: "m1".into(),
        kind: flow_like_types::inbox::InboxEntryKind::Transcript,
        envelope: Some(envelope),
        envelopes: None,
        turn: None,
    };
    let stored = validate_send(request(transcript_envelope())).expect("the mark is accepted");
    assert!(matches!(
        &stored.content,
        crate::routes::inbox::entries::Content::Plain(envelope)
            if envelope["body"]["interactive"]["client_run_id"] == json!("run_1")
    ));

    let db = database().await;
    chat(&db).await;
    queue_transcript(&db, true, at(100)).await;
    let held = turn(&db).await;
    assert_eq!(held.status, STORED_QUEUED);
    assert_eq!(held.lease_until, Some(at(100 + INTERACTIVE_GRACE_SECS)));
    assert_eq!(waiting(&db, at(101)).await, 0);

    let chat_turn = interactive(&held);
    let outgoing = validate_send(chat_turn.request("Sunny.", None)).unwrap();
    let claim_row = TurnClaim {
        id: held.id.clone(),
        host_id: chat_turn.host_id.clone(),
    };
    reply_interactive_in(&db, "c1", "bot_1", &claim_row, &outgoing, at(130))
        .await
        .unwrap();
    assert_eq!(turn(&db).await.status, STORED_DONE);
    assert_eq!(waiting(&db, at(1_000)).await, 0);
}

#[tokio::test]
async fn a_transcript_that_does_not_name_the_bot_queues_no_turn() {
    let db = database().await;
    chat(&db).await;
    queue_transcript(&db, false, at(100)).await;
    let queued = bot_turn::Entity::find_by_id(turn_id("bot_1", "m1"))
        .one(&db)
        .await
        .unwrap();
    assert!(queued.is_none());
}
