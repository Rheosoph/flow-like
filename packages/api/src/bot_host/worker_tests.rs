use super::{
    fixture::{at, conversation, database, entry, insert_bot, message},
    history::{HISTORY_ENTRIES, HistoryError, reaction_turn, transcript},
    ledger::{self, ClaimRequest, enqueue_in},
    worker::{
        AbortOnDrop, Answer, CODE_TIMEOUT, Failure, answer_request, commit_reply_in,
        executor_token, holds_lease_or_retry, memory_written, refuse_tool, reply_request, retries,
        room_of, run_while_notifying, usable_decision_model,
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

#[tokio::test(start_paused = true)]
async fn an_unavailable_status_provider_does_not_delay_completed_work() {
    struct Cancelled<'a>(&'a AtomicBool);
    impl Drop for Cancelled<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let cancelled = AtomicBool::new(false);
    let (started, received) = tokio::sync::oneshot::channel();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run_while_notifying(
            async {
                received.await.unwrap();
                "finished"
            },
            async {
                let _cancelled = Cancelled(&cancelled);
                started.send(()).unwrap();
                std::future::pending().await
            },
        ),
    )
    .await
    .expect("work must finish even while status delivery is pending");
    assert_eq!(result, "finished");
    assert!(cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn status_delivery_finishing_first_keeps_the_work_running() {
    let (sent, received) = tokio::sync::oneshot::channel();
    let result = run_while_notifying(
        async {
            received.await.unwrap();
            tokio::task::yield_now().await;
            "finished"
        },
        async { sent.send(()).unwrap() },
    )
    .await;
    assert_eq!(result, "finished");
}

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
    let transcript = transcript(&db, "bot_1", "c1", "m3", ConversationKind::Direct, "user-a")
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
async fn a_spoken_transcript_reads_as_the_speakers_line_and_a_call_is_left_out() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    conversation(&db, "g1", "GROUP", "PLAIN", &["user-a", "user-c", "bot_1"]).await;
    let inboxes = ["user-a", "user-c", "bot_1"];
    entry(
        &db,
        "g1",
        "call_1",
        "user-c",
        1,
        "CALL",
        json!({ "op": "started" }),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "g1",
        "t1",
        "user-c",
        2,
        "TRANSCRIPT",
        json!({ "call": "call_1", "text": "ship it friday", "offset_ms": 1000, "duration_ms": 2000, "mentions": [] }),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "g1",
        "t2",
        "user-a",
        3,
        "TRANSCRIPT",
        json!({ "call": "call_1", "text": "@bot_1 note that", "offset_ms": 4000, "duration_ms": 1500, "mentions": ["bot_1"] }),
        &inboxes,
    )
    .await;
    let transcript = transcript(&db, "bot_1", "g1", "t2", ConversationKind::Group, "user-a")
        .await
        .unwrap();
    assert!(transcript.addressed);
    assert_eq!(
        transcript.prompt,
        "user-a (your owner) wrote: (spoken in call) @bot_1 note that"
    );
    let said: Vec<_> = transcript
        .history
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        said,
        ["user-c (member, not your owner) wrote: (spoken in call) ship it friday"]
    );
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
    let transcript = transcript(&db, "bot_1", "g1", "m4", ConversationKind::Group, "user-a")
        .await
        .unwrap();
    assert_eq!(
        transcript.prompt,
        "user-a (your owner) wrote: @bot_1 summarize"
    );
    let said: Vec<_> = transcript
        .history
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        said,
        [
            "user-c (member, not your owner) wrote: hello",
            "hi",
            "bot_2 (member, not your owner) wrote: agreed"
        ]
    );
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
    let transcript = transcript(
        &db,
        "bot_1",
        "c1",
        "m40",
        ConversationKind::Direct,
        "user-a",
    )
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
        transcript(
            &db,
            "bot_1",
            "c1",
            "nope",
            ConversationKind::Direct,
            "user-a"
        )
        .await,
        Err(HistoryError::TriggerMissing)
    ));
    assert!(matches!(
        transcript(&db, "bot_1", "c1", "m2", ConversationKind::Direct, "user-a").await,
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
        transcript(&db, "bot_1", "c1", "m3", ConversationKind::Direct, "user-a").await,
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
        requester_id: Some("user-a".into()),
        audience: "OWNER".into(),
        status_text: None,
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
        hands_off: false,
        mentioned: false,
        replied_to: false,
        read_along: false,
        reacted: false,
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
            hands_off: false,
            mentioned: false,
            replied_to: false,
            read_along: false,
            reacted: false,
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
            hands_off: false,
            mentioned: false,
            replied_to: false,
            read_along: false,
            reacted: false,
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

#[test]
fn a_turn_token_names_the_bot_and_keeps_the_owner_as_payer() {
    crate::backend_jwt::init_for_tests();
    let direct = executor_token("owner", "bot-1", None, "turn-1", Some("ws"), 600).unwrap();
    let claims = crate::execution::verify_execution_jwt(&direct).unwrap();
    assert_eq!(claims.sub, "owner");
    assert_eq!(claims.bot_id.as_deref(), Some("bot-1"));
    assert_eq!(claims.conv_id, None);
    assert_eq!(claims.run_id, "turn-1");
    assert!(claims.unattended);

    let room = room_of(ConversationKind::Group, "conv-1");
    let token = executor_token("owner", "bot-1", room, "turn-1", Some("ws"), 600).unwrap();
    let claims = crate::execution::verify_execution_jwt(&token).unwrap();
    assert_eq!(claims.conv_id.as_deref(), Some("conv-1"));
    assert_eq!(room_of(ConversationKind::Direct, "conv-1"), None);
}

#[test]
fn only_a_spoken_answer_posts_a_reply() {
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
        hops: 0,
        requester_id: None,
        audience: "ROOM".into(),
        status_text: None,
        created_at: now_millis(),
        updated_at: now_millis(),
    };
    let silent = Answer {
        acknowledgement: false,
        text: String::new(),
        resumed: false,
        silent: true,
        completed: false,
        event_delivery: None,
        note: None,
        tool_calls: Vec::new(),
        files: Vec::new(),
    };
    assert!(answer_request(&turn, &silent).is_none());
    let mut spoken = Answer {
        acknowledgement: false,
        text: "hello".into(),
        resumed: false,
        silent: false,
        completed: false,
        event_delivery: None,
        note: None,
        tool_calls: Vec::new(),
        files: Vec::new(),
    };
    assert!(answer_request(&turn, &spoken).is_some());
    spoken.acknowledgement = true;
    assert_eq!(
        answer_request(&turn, &spoken).unwrap().envelope.unwrap()["body"]["system_one_acknowledgement"],
        true
    );
    spoken.acknowledgement = false;
    spoken.event_delivery = Some(flow_like::flow::copilot::decision_event::EventDecision::Respond);
    let request = answer_request(&turn, &spoken).unwrap();
    assert_eq!(
        request.envelope.unwrap()["body"]["system_one_delivery"],
        "respond"
    );
    spoken.event_delivery = Some(flow_like::flow::copilot::decision_event::EventDecision::Notify);
    assert!(
        answer_request(&turn, &spoken).unwrap().envelope.unwrap()["body"]
            .get("system_one_delivery")
            .is_none()
    );
}

#[tokio::test]
async fn a_silent_turn_ends_done_with_the_silent_code_and_posts_nothing() {
    let db = database().await;
    lone_message_history(&db).await;
    let recipient = super::fixture::recipient("bot_1", "user-a", BotLocation::Server);
    let mut recipient = recipient;
    recipient.trigger_message_id = "m3".into();
    recipient.read_along = true;
    let delivery = super::fixture::delivery(ConversationKind::Group, vec![recipient]);
    enqueue_in(&db, &delivery, now_millis()).await.unwrap();
    let turn = claim_now(&db, "bot_1", "m3", "h1").await;
    let before = inbox_entry::Entity::find().all(&db).await.unwrap().len();

    assert!(
        ledger::complete_silent_in(&db, &turn.id, "h1", now_millis())
            .await
            .unwrap()
    );
    assert!(
        !ledger::complete_silent_in(&db, &turn.id, "h1", now_millis())
            .await
            .unwrap(),
        "a finished turn is not completed twice"
    );
    let done = bot_turn::Entity::find_by_id(&turn.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, STORED_DONE);
    assert_eq!(
        done.error_code.as_deref(),
        Some(flow_like_types::bot_turn::ERROR_SILENT)
    );
    assert!(done.reply_message_id.is_none() && done.lease_until.is_none());
    assert_eq!(
        inbox_entry::Entity::find().all(&db).await.unwrap().len(),
        before
    );
}

#[tokio::test]
async fn another_host_cannot_complete_a_turn_silently() {
    let db = database().await;
    lone_message_history(&db).await;
    let mut recipient = super::fixture::recipient("bot_1", "user-a", BotLocation::Server);
    recipient.trigger_message_id = "m3".into();
    let delivery = super::fixture::delivery(ConversationKind::Direct, vec![recipient]);
    enqueue_in(&db, &delivery, now_millis()).await.unwrap();
    let turn = claim_now(&db, "bot_1", "m3", "h1").await;
    assert!(
        !ledger::complete_silent_in(&db, &turn.id, "h2", now_millis())
            .await
            .unwrap()
    );
}

async fn room_with_reactions(db: &DatabaseConnection) {
    insert_bot(db, "bot_1", "user-a", "SERVER").await;
    conversation(db, "c1", "GROUP", "PLAIN", &["user-a", "user-b", "bot_1"]).await;
    let inboxes = ["user-a", "user-b", "bot_1"];
    message(
        db,
        "c1",
        "m1",
        "user-a",
        1,
        json!({ "text": "plan?" }),
        &inboxes,
    )
    .await;
    message(
        db,
        "c1",
        "m2",
        "user-b",
        2,
        json!({ "text": "hm" }),
        &inboxes,
    )
    .await;
    message(
        db,
        "c1",
        "b1",
        "bot_1",
        3,
        json!({ "text": "Ship on Friday" }),
        &inboxes,
    )
    .await;
}

fn reaction(target: &str, emoji: &str, op: &str) -> serde_json::Value {
    json!({ "target": target, "emoji": emoji, "op": op })
}

#[tokio::test]
async fn a_reaction_turn_lists_the_reactions_in_place_on_the_bots_own_messages_only() {
    let db = database().await;
    room_with_reactions(&db).await;
    let inboxes = ["user-a", "user-b", "bot_1"];
    entry(
        &db,
        "c1",
        "r1",
        "user-a",
        4,
        "REACTION",
        reaction("b1", "👎", "add"),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "c1",
        "r2",
        "user-b",
        5,
        "REACTION",
        reaction("b1", "👍", "add"),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "c1",
        "r3",
        "user-b",
        6,
        "REACTION",
        reaction("b1", "👍", "remove"),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "c1",
        "r4",
        "user-a",
        7,
        "REACTION",
        reaction("m2", "❓", "add"),
        &inboxes,
    )
    .await;

    let answered: Vec<String> = ["r1", "r2", "r4"].iter().map(|id| id.to_string()).collect();
    let turn = reaction_turn(
        &db,
        "bot_1",
        "c1",
        &answered,
        ConversationKind::Group,
        "user-a",
    )
    .await
    .unwrap();
    assert_eq!(
        turn.reactions.len(),
        1,
        "removed and foreign-message reactions drop out"
    );
    assert_eq!(turn.reactions[0].emoji, "👎");
    assert_eq!(turn.reactions[0].excerpt, "Ship on Friday");
    assert!(turn.transcript.addressed);
    assert!(
        turn.transcript
            .prompt
            .contains("reacted 👎 to your message")
    );
}

#[tokio::test]
async fn a_reaction_removed_before_the_turn_starts_leaves_nothing_to_answer() {
    let db = database().await;
    room_with_reactions(&db).await;
    let inboxes = ["user-a", "user-b", "bot_1"];
    entry(
        &db,
        "c1",
        "r1",
        "user-a",
        4,
        "REACTION",
        reaction("b1", "👎", "add"),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "c1",
        "r2",
        "user-a",
        5,
        "REACTION",
        reaction("b1", "👎", "remove"),
        &inboxes,
    )
    .await;
    let answered = vec!["r1".to_string()];
    let turn = reaction_turn(
        &db,
        "bot_1",
        "c1",
        &answered,
        ConversationKind::Group,
        "user-a",
    )
    .await
    .unwrap();
    assert!(turn.reactions.is_empty());
}

#[tokio::test]
async fn several_reactions_in_a_row_make_one_turn_that_names_each() {
    let db = database().await;
    room_with_reactions(&db).await;
    let inboxes = ["user-a", "user-b", "bot_1"];
    entry(
        &db,
        "c1",
        "r1",
        "user-a",
        4,
        "REACTION",
        reaction("b1", "👎", "add"),
        &inboxes,
    )
    .await;
    entry(
        &db,
        "c1",
        "r2",
        "user-b",
        5,
        "REACTION",
        reaction("b1", "❓", "add"),
        &inboxes,
    )
    .await;
    let answered = vec!["r2".to_string(), "r1".to_string()];
    let turn = reaction_turn(
        &db,
        "bot_1",
        "c1",
        &answered,
        ConversationKind::Group,
        "user-a",
    )
    .await
    .unwrap();
    let emojis: Vec<_> = turn
        .reactions
        .iter()
        .map(|note| note.emoji.as_str())
        .collect();
    assert_eq!(emojis, ["👎", "❓"]);
}

#[test]
fn an_unavailable_decision_model_is_skipped_not_failed() {
    use crate::routes::ai::global_chat::ProfileModelAccess;
    let none = ProfileModelAccess::default();
    assert_eq!(usable_decision_model(&none, Some("bit"), "t1"), None);
    assert_eq!(usable_decision_model(&none, None, "t1"), None);
    let covered = ProfileModelAccess {
        profile_models: 1,
        allowed_models: 1,
        ..Default::default()
    };
    assert_eq!(
        usable_decision_model(&covered, Some("bit"), "t1"),
        Some("bit")
    );
}

#[tokio::test]
async fn a_lease_check_error_fails_closed_after_retrying_and_a_held_lease_passes() {
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
            hands_off: false,
            mentioned: false,
            replied_to: false,
            read_along: false,
            reacted: false,
            conversation_id: "c1".into(),
            trigger_message_id: "m3".into(),
            hops: 0,
            interactive: false,
        }],
    );
    enqueue_in(&db, &delivery, now_millis()).await.unwrap();
    let turn = claim_now(&db, "bot_1", "m3", "h1").await;
    assert!(holds_lease_or_retry(&db, &turn.id, "h1").await);
    assert!(!holds_lease_or_retry(&db, &turn.id, "other").await);
    db.execute_unprepared(r#"DROP TABLE public."BotTurn""#)
        .await
        .unwrap();
    assert!(!holds_lease_or_retry(&db, &turn.id, "h1").await);
}

#[tokio::test]
async fn dropping_the_heartbeat_guard_aborts_the_task() {
    let task = tokio::spawn(std::future::pending::<()>());
    let probe = task.abort_handle();
    drop(AbortOnDrop(task));
    for _ in 0..50 {
        if probe.is_finished() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the heartbeat task kept running after its guard was dropped");
}
