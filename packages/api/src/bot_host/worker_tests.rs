use super::{
    fixture::{at, conversation, database, insert_bot, message},
    history::{HISTORY_ENTRIES, HistoryError, transcript},
    ledger::{self, ClaimRequest, enqueue_in},
    worker::{
        CODE_TIMEOUT, Failure, commit_reply_in, memory_written, refuse_tool, reply_request, retries,
    },
};
use crate::{
    entity::{bot_turn, inbox_entry},
    routes::inbox::{
        delivery::BotRecipient, entries::validate_send, model::TurnClaim, now_millis,
        turns::reply_in,
    },
};
use flow_like::flow::copilot::ChatRole;
use flow_like_types::{
    bot::BotLocation,
    bot_message::MessageBody,
    bot_turn::{
        MAX_HOPS, STORED_DONE, STORED_FAILED, STORED_QUEUED, TurnHostKind, failure_message_id,
        reply_message_id, turn_id,
    },
    inbox::{ConversationKind, tombstone_envelope},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

async fn lone_message_history(db: &DatabaseConnection) {
    insert_bot(db, "bot_1", "user-a", "SERVER").await;
    conversation(db, "c1", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
    for (n, (id, sender, text)) in [
        ("m1", "user-a", "first"),
        ("m2", "bot_1", "second"),
        ("m3", "user-a", "third"),
    ]
    .into_iter()
    .enumerate()
    {
        message(
            db,
            "c1",
            id,
            sender,
            n as i64 + 1,
            json!({ "text": text }),
            &["user-a", "bot_1"],
        )
        .await;
    }
}

#[tokio::test]
async fn the_transcript_ends_at_the_answered_message_and_marks_who_spoke() {
    let db = database().await;
    lone_message_history(&db).await;
    message(
        &db,
        "c1",
        "m4",
        "user-a",
        4,
        json!({ "text": "later" }),
        &["bot_1"],
    )
    .await;
    let transcript = transcript(&db, "bot_1", "c1", "m3", ConversationKind::Direct)
        .await
        .unwrap();
    assert_eq!(transcript.prompt, "third");
    let history: Vec<_> = transcript
        .history
        .iter()
        .map(|message| {
            (
                message.role == ChatRole::Assistant,
                message.content.as_str(),
            )
        })
        .collect();
    assert_eq!(history, [(false, "first"), (true, "second")]);
}

#[tokio::test]
async fn a_group_transcript_names_the_senders_but_not_the_bot_itself() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_2", "user-b", "SERVER").await;
    conversation(
        &db,
        "g1",
        "GROUP",
        "PLAIN",
        &["user-a", "user-c", "bot_1", "bot_2"],
    )
    .await;
    let inboxes = ["user-a", "bot_1"];
    message(
        &db,
        "g1",
        "m1",
        "user-c",
        1,
        json!({ "text": "hello" }),
        &inboxes,
    )
    .await;
    message(
        &db,
        "g1",
        "m2",
        "bot_1",
        2,
        json!({ "text": "hi" }),
        &inboxes,
    )
    .await;
    message(
        &db,
        "g1",
        "m3",
        "bot_2",
        3,
        json!({ "text": "agreed" }),
        &inboxes,
    )
    .await;
    message(
        &db,
        "g1",
        "m4",
        "user-a",
        4,
        json!({ "text": "@bot_1 summarize" }),
        &inboxes,
    )
    .await;
    let transcript = transcript(&db, "bot_1", "g1", "m4", ConversationKind::Group)
        .await
        .unwrap();
    assert_eq!(transcript.prompt, "user-a: @bot_1 summarize");
    let said: Vec<_> = transcript
        .history
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(said, ["user-c: hello", "hi", "bot_2: agreed"]);
}

#[tokio::test]
async fn the_transcript_holds_the_last_entries_only() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    conversation(&db, "c1", "DIRECT", "PLAIN", &["user-a", "bot_1"]).await;
    for n in 1..=40 {
        message(
            &db,
            "c1",
            &format!("m{n}"),
            "user-a",
            n,
            json!({ "text": format!("t{n}") }),
            &["bot_1"],
        )
        .await;
    }
    let transcript = transcript(&db, "bot_1", "c1", "m40", ConversationKind::Direct)
        .await
        .unwrap();
    assert_eq!(transcript.history.len() as u64, HISTORY_ENTRIES - 1);
    assert_eq!(transcript.history[0].content, "t9");
    assert_eq!(transcript.prompt, "t40");
}

#[tokio::test]
async fn a_missing_deleted_sealed_or_own_trigger_is_unreadable() {
    let db = database().await;
    lone_message_history(&db).await;
    assert!(matches!(
        transcript(&db, "bot_1", "c1", "nope", ConversationKind::Direct).await,
        Err(HistoryError::TriggerMissing)
    ));
    assert!(matches!(
        transcript(&db, "bot_1", "c1", "m2", ConversationKind::Direct).await,
        Err(HistoryError::TriggerUnreadable)
    ));
    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Envelope,
            sea_orm::sea_query::Expr::value(tombstone_envelope()),
        )
        .filter(inbox_entry::Column::MessageId.eq("m3"))
        .exec(&db)
        .await
        .unwrap();
    assert!(matches!(
        transcript(&db, "bot_1", "c1", "m3", ConversationKind::Direct).await,
        Err(HistoryError::TriggerUnreadable)
    ));
}

#[test]
fn a_reply_answers_the_trigger_one_hop_further_and_a_failure_names_its_code() {
    let turn = bot_turn::Model {
        id: turn_id("bot_1", "m3"),
        bot_id: "bot_1".into(),
        owner_id: "user-a".into(),
        conversation_id: "c1".into(),
        trigger_message_id: "m3".into(),
        host_kind: "SERVER".into(),
        status: "RUNNING".into(),
        attempts: 1,
        host_id: Some("h1".into()),
        lease_until: None,
        operation_id: None,
        reply_message_id: None,
        error_code: None,
        hops: 2,
        created_at: at(0),
        updated_at: at(0),
    };
    let reply = reply_request(&turn, reply_message_id("bot_1", "m3"), "done", None);
    let body = reply.envelope.as_ref().unwrap()["body"].clone();
    let parsed = MessageBody::parse(&body).unwrap();
    assert_eq!(parsed.text, "done");
    assert_eq!(parsed.reply_to.as_deref(), Some("m3"));
    assert_eq!(parsed.hops, Some(3));
    assert!(parsed.error.is_none());
    assert_eq!(reply.message_id, reply_message_id("bot_1", "m3"));

    let failure = reply_request(
        &turn,
        failure_message_id("bot_1", "m3"),
        "I ran out of time",
        Some(CODE_TIMEOUT),
    );
    let parsed = MessageBody::parse(&failure.envelope.as_ref().unwrap()["body"]).unwrap();
    assert_eq!(
        parsed.error.map(|error| error.code),
        Some(CODE_TIMEOUT.to_string())
    );
    assert!(validate_send(failure).is_ok());
}

#[test]
fn the_headless_bridge_refuses_every_tool_and_remembers_side_effects() {
    let mutating = AtomicBool::new(false);
    let refusal: serde_json::Value =
        serde_json::from_str(&refuse_tool("list_apps", &mutating)).unwrap();
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["code"], "bot_tool_unavailable_headless");
    assert!(!mutating.load(Ordering::SeqCst));
    refuse_tool("unknown_tool", &mutating);
    assert!(!mutating.load(Ordering::SeqCst));
    refuse_tool("call_app_event", &mutating);
    assert!(mutating.load(Ordering::SeqCst));
}

async fn claim_now(
    db: &DatabaseConnection,
    bot_id: &str,
    trigger: &str,
    host: &str,
) -> bot_turn::Model {
    ledger::claim_in(
        db,
        &ClaimRequest {
            turn_id: turn_id(bot_id, trigger),
            owner_id: None,
            host_kind: TurnHostKind::Server,
            host_id: host.into(),
            now: now_millis(),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .turn
}

#[tokio::test]
async fn the_reply_completes_the_turn_once_and_a_repeat_returns_the_first_copy() {
    let db = database().await;
    lone_message_history(&db).await;
    db.execute_unprepared(r#"UPDATE public."Conversation" SET "lastSeq" = 3"#)
        .await
        .unwrap();
    let recipient = BotRecipient {
        bot_id: "bot_1".into(),
        owner_id: "user-a".into(),
        location: BotLocation::Server,
        device_id: None,
        mentioned: false,
        replied_to: false,
        conversation_id: "c1".into(),
        trigger_message_id: "m3".into(),
        hops: 0,
        interactive: false,
    };
    let delivery = super::fixture::delivery(ConversationKind::Direct, vec![recipient]);
    enqueue_in(&db, &delivery, now_millis()).await.unwrap();
    let turn = claim_now(&db, "bot_1", "m3", "h1").await;

    let request = reply_request(&turn, reply_message_id("bot_1", "m3"), "the answer", None);
    let outgoing = validate_send(request).unwrap();
    let claim = TurnClaim {
        id: turn.id.clone(),
        host_id: "h1".into(),
    };
    let first = reply_in(&db, "c1", "bot_1", &claim, &outgoing, now_millis())
        .await
        .unwrap();
    let again = reply_in(&db, "c1", "bot_1", &claim, &outgoing, now_millis())
        .await
        .unwrap();
    assert_eq!(first.entry.id, again.entry.id);
    assert_eq!(first.entry.seq, Some(4));
    assert!(again.wake.is_empty() && again.bots.is_empty());

    let copies = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(reply_message_id("bot_1", "m3")))
        .order_by_asc(inbox_entry::Column::InboxId)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(copies.len(), 2, "one copy per member, once");
    let done = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, STORED_DONE);
    assert_eq!(done.reply_message_id, Some(reply_message_id("bot_1", "m3")));
    assert!(done.lease_until.is_none());
    assert_eq!(first.entry.envelope["body"]["reply_to"], json!("m3"));
}

#[tokio::test]
async fn a_failed_turn_posts_its_error_and_ends_failed_in_one_step() {
    let db = database().await;
    lone_message_history(&db).await;
    db.execute_unprepared(r#"UPDATE public."Conversation" SET "lastSeq" = 3"#)
        .await
        .unwrap();
    let delivery = super::fixture::delivery(
        ConversationKind::Direct,
        vec![BotRecipient {
            bot_id: "bot_1".into(),
            owner_id: "user-a".into(),
            location: BotLocation::Server,
            device_id: None,
            mentioned: false,
            replied_to: false,
            conversation_id: "c1".into(),
            trigger_message_id: "m3".into(),
            hops: 0,
            interactive: false,
        }],
    );
    enqueue_in(&db, &delivery, now_millis()).await.unwrap();
    let turn = claim_now(&db, "bot_1", "m3", "h1").await;
    let sent = commit_reply_in(
        &db,
        &turn,
        "h1",
        reply_request(
            &turn,
            failure_message_id("bot_1", "m3"),
            "Out of time",
            Some(CODE_TIMEOUT),
        ),
        Some(CODE_TIMEOUT),
    )
    .await
    .unwrap();
    assert_eq!(
        sent.entry.envelope["body"]["error"]["code"],
        json!(CODE_TIMEOUT)
    );
    let failed = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            failed.status.as_str(),
            failed.error_code.as_deref(),
            failed.reply_message_id
        ),
        (
            STORED_FAILED,
            Some(CODE_TIMEOUT),
            Some(failure_message_id("bot_1", "m3"))
        )
    );
}

#[tokio::test]
async fn bots_answering_each_other_stop_at_the_hop_cap() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    insert_bot(&db, "bot_2", "user-b", "SERVER").await;
    conversation(&db, "c1", "DIRECT", "PLAIN", &["bot_1", "bot_2"]).await;
    message(
        &db,
        "c1",
        "m1",
        "bot_2",
        1,
        json!({ "text": "start" }),
        &["bot_1", "bot_2"],
    )
    .await;
    db.execute_unprepared(r#"UPDATE public."Conversation" SET "lastSeq" = 1"#)
        .await
        .unwrap();
    let wake_first = super::fixture::delivery(
        ConversationKind::Direct,
        vec![BotRecipient {
            bot_id: "bot_1".into(),
            owner_id: "user-a".into(),
            location: BotLocation::Server,
            device_id: None,
            mentioned: false,
            replied_to: false,
            conversation_id: "c1".into(),
            trigger_message_id: "m1".into(),
            hops: 0,
            interactive: false,
        }],
    );
    enqueue_in(&db, &wake_first, now_millis()).await.unwrap();

    let (mut bot, mut trigger) = ("bot_1", "m1".to_string());
    let mut ran = Vec::new();
    loop {
        let turn = claim_now(&db, bot, &trigger, "h1").await;
        ran.push((bot, turn.hops));
        let reply_id = reply_message_id(bot, &trigger);
        let outgoing = validate_send(reply_request(&turn, reply_id.clone(), "ok", None)).unwrap();
        let claim = TurnClaim {
            id: turn.id.clone(),
            host_id: "h1".into(),
        };
        let sent = reply_in(&db, "c1", bot, &claim, &outgoing, now_millis())
            .await
            .unwrap();
        let Some(next) = sent.bots.first() else { break };
        assert_eq!(next.hops, turn.hops as u32 + 1);
        let queued_next = bot_turn::Entity::find_by_id(turn_id(&next.bot_id, &reply_id))
            .one(&db)
            .await
            .unwrap();
        if queued_next.is_none() {
            assert_eq!(next.hops, MAX_HOPS + 1);
            break;
        }
        (bot, trigger) = (if bot == "bot_1" { "bot_2" } else { "bot_1" }, reply_id);
    }
    assert_eq!(
        ran,
        [("bot_1", 0), ("bot_2", 1), ("bot_1", 2), ("bot_2", 3)],
        "the turn at the cap runs; its reply (hops {}) wakes no one",
        MAX_HOPS + 1
    );
    let queued = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .all(&db)
        .await
        .unwrap();
    assert!(queued.is_empty(), "{queued:?}");
}

#[test]
fn memory_writes_end_the_retries_but_reads_do_not() {
    let transient = Failure {
        code: CODE_TIMEOUT,
        text: "slow".into(),
        retryable: true,
        post: true,
        park: None,
    };
    for tool in ["_memory_store", "memory_store", "memory_save"] {
        let mutating = AtomicBool::new(false);
        refuse_tool(tool, &mutating);
        assert!(mutating.load(Ordering::SeqCst), "{tool}");
        assert!(
            !retries(&transient, 1, mutating.load(Ordering::SeqCst)),
            "{tool}"
        );
    }
    for tool in ["_memory_search", "memory_search", "list_apps"] {
        let mutating = AtomicBool::new(false);
        refuse_tool(tool, &mutating);
        assert!(!mutating.load(Ordering::SeqCst), "{tool}");
    }
    assert!(retries(&transient, 1, false));
}

#[test]
fn a_changed_or_unreadable_memory_count_counts_as_a_write() {
    assert!(!memory_written(Some(4), Some(4)));
    assert!(memory_written(Some(4), Some(5)));
    assert!(memory_written(Some(4), None));
    assert!(memory_written(None, Some(4)));
}
