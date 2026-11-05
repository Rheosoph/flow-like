use super::{
    approvals::GrantRows,
    fixture::{at, database, insert_bot},
    ledger::{self, enqueue_in, next_head, recover_in},
    talk::{
        ERROR_ASK_ANSWERED, ERROR_ASK_EXPIRED, ERROR_ASK_QUEUED, Question, REPLY_BY_DAYS,
        WAIT_SECS, answer_from, answered_ask, ask_body, ask_result_transcript, asked_by,
        end_lapsed_in, grant_scope, granted_in, headless_verdict, roster_in, turn_conversation,
    },
};
use crate::{
    entity::{bot_turn, inbox_entry},
    routes::{
        bots::fire::{FiredTurn, TurnReply},
        inbox::{
            activity::{activity_message_id, bot_ask_targets, write_activity_in},
            approvals::open_conversation,
            conversations::start_exchange,
            exchange::{message_outgoing, post_as_bot_in},
            now_millis,
        },
    },
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like::flow::copilot::{
    bot_talk::{BotPresence, CrewMember, HeadlessPermission, PendingAnswer},
    tool_grants::{GrantStore, record_card_answer},
};
use flow_like_types::{
    bot_turn::{STORED_CANCELLED, STORED_QUEUED, turn_id},
    inbox::{ActivityBody, InboxEntryKind},
    tool_grant::ApprovalDecision,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde_json::json;

const OWNER: &str = "user-a";
const SCOUT: &str = "bot_scout";
const LEDGER: &str = "bot_ledger";

async fn talk_db() -> DatabaseConnection {
    let db = database().await;
    for statement in [
        r#"CREATE TABLE public."BotPermission" (
            "id" TEXT PRIMARY KEY, "ownerId" TEXT NOT NULL, "fromBotId" TEXT NOT NULL,
            "toBotId" TEXT NOT NULL, "grantedAt" TEXT NOT NULL, "grantedBy" TEXT NOT NULL,
            UNIQUE ("fromBotId", "toBotId"))"#,
        r#"CREATE TABLE public."BotHostHeartbeat" (
            "botId" TEXT NOT NULL, "hostId" TEXT NOT NULL, "hostKind" TEXT NOT NULL,
            "lastSeenAt" TEXT NOT NULL, PRIMARY KEY ("botId", "hostId"))"#,
        r#"CREATE TABLE public."BotToolGrant" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
            "tool" TEXT NOT NULL, "scope" TEXT NOT NULL, "grantedAt" TEXT NOT NULL,
            "grantedBy" TEXT NOT NULL, UNIQUE ("botId", "tool", "scope"))"#,
    ] {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

async fn heartbeat(db: &DatabaseConnection, bot: &str, seen: DateTime<FixedOffset>) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."BotHostHeartbeat" VALUES ('{bot}', 'h1', 'DESKTOP', '{}')"#,
        seen.format("%Y-%m-%d %H:%M:%S%.f%:z")
    ))
    .await
    .unwrap();
}

fn member(id: &str, name: &str) -> CrewMember {
    CrewMember {
        id: id.into(),
        name: name.into(),
        presence: BotPresence::Online,
        ..Default::default()
    }
}

async fn pair(scout: &str, ledger_location: &str) -> (DatabaseConnection, String) {
    let db = talk_db().await;
    insert_bot(&db, SCOUT, OWNER, scout).await;
    insert_bot(&db, LEDGER, OWNER, ledger_location).await;
    let id = start_exchange(&db, OWNER, SCOUT, LEDGER, Some("Scout".into()), at(0))
        .await
        .unwrap();
    (db, id)
}

async fn ask(db: &DatabaseConnection, conversation: &str, ask_id: &str, hops: u32, wait: u32) {
    ask_round(db, conversation, ask_id, hops, wait, 0).await;
}

async fn ask_round(
    db: &DatabaseConnection,
    conversation: &str,
    ask_id: &str,
    hops: u32,
    wait: u32,
    round: u32,
) {
    let chain = vec![SCOUT.to_string()];
    let question = Question {
        text: "What is the total?",
        callee_id: LEDGER,
        ask_id,
        origin_conversation: "origin",
        hops,
        chain: &chain,
        wait_secs: wait,
        round,
    };
    let outgoing = message_outgoing(ask_id, ask_body(&question, now_millis())).unwrap();
    post_as_bot_in(db, conversation, SCOUT, hops, &outgoing, now_millis())
        .await
        .unwrap();
}

async fn answer(db: &DatabaseConnection, conversation: &str, ask_id: &str, hops: u32) -> String {
    let id = format!("re-{ask_id}");
    let body = json!({ "text": "42", "reply_to": ask_id, "hops": hops });
    let outgoing = message_outgoing(&id, body).unwrap();
    post_as_bot_in(db, conversation, LEDGER, hops, &outgoing, now_millis())
        .await
        .unwrap();
    id
}

async fn turn(db: &DatabaseConnection, bot: &str, message: &str) -> Option<bot_turn::Model> {
    bot_turn::Entity::find_by_id(turn_id(bot, message))
        .one(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_roster_lists_the_other_bots_with_their_presence() {
    let (db, _) = pair("SERVER", "DESKTOP").await;
    insert_bot(&db, "bot_dev", OWNER, "DEVICE").await;
    insert_bot(&db, "bot_stranger", "user-b", "SERVER").await;
    heartbeat(&db, LEDGER, now_millis() - Duration::seconds(30)).await;
    heartbeat(&db, "bot_dev", now_millis() - Duration::seconds(600)).await;
    let roster = roster_in(&db, OWNER, SCOUT, now_millis()).await.unwrap();
    let presence: Vec<_> = roster
        .iter()
        .map(|member| (member.id.as_str(), member.presence))
        .collect();
    assert_eq!(
        presence,
        [
            ("bot_dev", BotPresence::Offline),
            (LEDGER, BotPresence::Online)
        ]
    );
}

#[tokio::test]
async fn a_card_answer_becomes_a_stored_permission() {
    let (db, _) = pair("SERVER", "SERVER").await;
    assert!(
        !granted_in(&db, OWNER, SCOUT, LEDGER, now_millis())
            .await
            .unwrap()
    );
    GrantRows::new(&db, SCOUT, OWNER)
        .put("ask_bot", &grant_scope(LEDGER))
        .await
        .unwrap();
    assert!(
        granted_in(&db, OWNER, SCOUT, LEDGER, now_millis())
            .await
            .unwrap()
    );
    let left = GrantRows::new(&db, SCOUT, OWNER).list().await.unwrap();
    assert!(left.is_empty());
    assert!(
        !granted_in(&db, OWNER, LEDGER, SCOUT, now_millis())
            .await
            .unwrap()
    );
    assert!(
        granted_in(&db, OWNER, SCOUT, LEDGER, now_millis())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_headless_ask_parks_then_runs_on_the_owners_answer() {
    let db = talk_db().await;
    insert_bot(&db, SCOUT, OWNER, "SERVER").await;
    insert_bot(&db, LEDGER, OWNER, "SERVER").await;
    let store = GrantRows::new(&db, SCOUT, OWNER);
    let (scout, ledger_bot) = (member(SCOUT, "Scout"), member(LEDGER, "Ledger"));
    let (verdict, card) =
        headless_verdict(&store, "bt_1", &scout, &ledger_bot, "What is the total?").await;
    assert_eq!(verdict, HeadlessPermission::Parked);
    let card = card.unwrap();
    assert_eq!(card.tool, "ask_bot");
    assert_eq!(card.scope, "bot:bot_ledger");
    assert_eq!(
        card.summary,
        "Scout wants to ask Ledger: What is the total?"
    );

    record_card_answer(&store, &card, ApprovalDecision::Once)
        .await
        .unwrap();
    let (verdict, _) =
        headless_verdict(&store, "bt_1", &scout, &ledger_bot, "What is the total?").await;
    assert_eq!(verdict, HeadlessPermission::Allowed);
    let (again, _) =
        headless_verdict(&store, "bt_1", &scout, &ledger_bot, "What is the total?").await;
    assert_eq!(
        again,
        HeadlessPermission::Parked,
        "a one-time answer is spent"
    );

    record_card_answer(&store, &card, ApprovalDecision::Deny)
        .await
        .unwrap();
    let (verdict, _) =
        headless_verdict(&store, "bt_1", &scout, &ledger_bot, "What is the total?").await;
    assert_eq!(verdict, HeadlessPermission::Denied);

    record_card_answer(&store, &card, ApprovalDecision::Always)
        .await
        .unwrap();
    assert!(
        granted_in(&db, OWNER, SCOUT, LEDGER, now_millis())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn one_pair_has_one_exchange() {
    let (db, id) = pair("SERVER", "SERVER").await;
    let again = start_exchange(&db, OWNER, LEDGER, SCOUT, None, at(5))
        .await
        .unwrap();
    assert_eq!(id, again);
}

#[tokio::test]
async fn a_server_to_server_ask_queues_the_callees_turn_and_its_answer_reaches_the_asker() {
    let (db, conversation) = pair("SERVER", "SERVER").await;
    ask(&db, &conversation, "ask-1", 1, WAIT_SECS).await;
    let queued = turn(&db, LEDGER, "ask-1").await.expect("callee turn");
    assert_eq!(queued.status, STORED_QUEUED);
    assert_eq!(queued.hops, 1);
    assert_eq!(queued.error_code.as_deref(), Some(ERROR_ASK_QUEUED));
    assert_eq!(queued.host_kind, "SERVER");
    let chain = asked_by(&db, &queued).await.unwrap();
    assert_eq!(
        chain.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        [SCOUT]
    );

    let reply = answer(&db, &conversation, "ask-1", 2).await;
    let follow_up = turn(&db, SCOUT, &reply)
        .await
        .expect("asker follow-up turn");
    let held = follow_up.lease_until.expect("held while the asker waits");
    assert!(held > now_millis() + Duration::seconds(i64::from(WAIT_SECS) - 5));
    assert!(
        next_head(
            &db,
            SCOUT,
            &conversation,
            flow_like_types::bot_turn::TurnHostKind::Server,
            now_millis()
        )
        .await
        .unwrap()
        .is_none()
    );
    let later = now_millis() + Duration::seconds(i64::from(WAIT_SECS) + 30);
    assert!(
        next_head(
            &db,
            SCOUT,
            &conversation,
            flow_like_types::bot_turn::TurnHostKind::Server,
            later
        )
        .await
        .unwrap()
        .is_some()
    );

    ledger::cancel_in(&db, &follow_up.id, ERROR_ASK_ANSWERED, now_millis())
        .await
        .unwrap();
    let settled = turn(&db, SCOUT, &reply).await.unwrap();
    assert_eq!(settled.status, STORED_CANCELLED);
}

#[tokio::test]
async fn a_late_answer_wakes_the_asker_in_the_conversation_it_asked_from() {
    let (db, conversation) = pair("DESKTOP", "DESKTOP").await;
    ask(&db, &conversation, "ask-2", 1, WAIT_SECS).await;
    let reply = answer(&db, &conversation, "ask-2", 2).await;
    let follow_up = turn(&db, SCOUT, &reply).await.expect("follow-up");
    assert_eq!(follow_up.host_kind, "DESKTOP");
    let answered = answered_ask(&db, &follow_up)
        .await
        .unwrap()
        .expect("answers an ask");
    assert_eq!(answered.origin_conversation, "origin");
    assert_eq!(answered.question, "What is the total?");
    assert_eq!(answered.answer, "42");
    assert_eq!(answered.callee_id, LEDGER);
    assert!(!answered.failed);
    let plain = turn(&db, LEDGER, "ask-2").await.unwrap();
    assert!(answered_ask(&db, &plain).await.unwrap().is_none());
    let transcript = ask_result_transcript(&db, &follow_up, &answered).await;
    let prompt = transcript.map(|t| t.prompt).unwrap_or_default();
    assert!(prompt.contains(" answered:\n42"), "{prompt}");
}

#[tokio::test]
async fn a_desktop_hosts_reply_to_a_late_answer_turn_goes_to_the_origin_conversation() {
    use crate::routes::inbox::{
        entries::{Outgoing, answer_origin},
        model::TurnClaim,
    };
    let (db, conversation) = pair("DESKTOP", "DESKTOP").await;
    ask(&db, &conversation, "ask-4", 1, WAIT_SECS).await;
    let reply = answer(&db, &conversation, "ask-4", 2).await;
    let follow_up = turn(&db, SCOUT, &reply).await.expect("follow-up");
    let claim = TurnClaim {
        id: follow_up.id.clone(),
        host_id: "h1".into(),
    };
    let outgoing = |message_id: String| -> Outgoing {
        message_outgoing(&message_id, json!({ "text": "Ledger says 42", "hops": 3 })).unwrap()
    };
    let success = outgoing(flow_like_types::bot_turn::reply_message_id(
        SCOUT,
        &follow_up.trigger_message_id,
    ));
    let (routed, origin) = answer_origin(&db, SCOUT, &claim, &success)
        .await
        .unwrap()
        .expect("a late-answer reply is redirected");
    assert_eq!(routed.id, follow_up.id);
    assert_eq!(origin, "origin");

    let failure = outgoing(flow_like_types::bot_turn::failure_message_id(
        SCOUT,
        &follow_up.trigger_message_id,
    ));
    assert!(
        answer_origin(&db, SCOUT, &claim, &failure)
            .await
            .unwrap()
            .is_none()
    );

    let plain = turn(&db, LEDGER, "ask-4").await.unwrap();
    let plain_claim = TurnClaim {
        id: plain.id.clone(),
        host_id: "h1".into(),
    };
    let plain_reply = outgoing(flow_like_types::bot_turn::reply_message_id(
        LEDGER,
        &plain.trigger_message_id,
    ));
    assert!(
        answer_origin(&db, LEDGER, &plain_claim, &plain_reply)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_offline_callee_leaves_no_hold_and_its_question_waits_seven_days() {
    let (db, conversation) = pair("SERVER", "DESKTOP").await;
    ask(&db, &conversation, "ask-3", 1, 0).await;
    let reply = answer(&db, &conversation, "ask-3", 2).await;
    let follow_up = turn(&db, SCOUT, &reply).await.unwrap();
    assert!(
        follow_up.lease_until.is_none(),
        "an asker that did not wait is woken at once"
    );

    let queued = turn(&db, LEDGER, "ask-3").await.unwrap();
    assert_eq!(queued.status, STORED_QUEUED);
    let after_a_day = now_millis() + Duration::days(1);
    recover_in(&db, after_a_day).await.unwrap();
    assert_eq!(
        turn(&db, LEDGER, "ask-3").await.unwrap().status,
        STORED_QUEUED
    );
    assert!(end_lapsed_in(&db, after_a_day).await.unwrap().is_empty());

    let after_eight = now_millis() + Duration::days(REPLY_BY_DAYS + 1);
    let lapsed = end_lapsed_in(&db, after_eight).await.unwrap();
    assert_eq!(lapsed.len(), 1);
    assert_eq!(lapsed[0].asker, SCOUT);
    assert_eq!(lapsed[0].origin_conversation, "origin");
    assert!(lapsed[0].text.contains("lapsed after 7 days"));
    let ended = turn(&db, LEDGER, "ask-3").await.unwrap();
    assert_eq!(ended.status, STORED_CANCELLED);
    assert_eq!(ended.error_code.as_deref(), Some(ERROR_ASK_EXPIRED));
}

#[tokio::test]
async fn a_question_past_the_hop_limit_queues_no_turn() {
    let (db, conversation) = pair("SERVER", "SERVER").await;
    ask(&db, &conversation, "ask-deep", 4, WAIT_SECS).await;
    assert!(turn(&db, LEDGER, "ask-deep").await.is_none());
    ask(&db, &conversation, "ask-edge", 3, WAIT_SECS).await;
    assert!(turn(&db, LEDGER, "ask-edge").await.is_some());
}

fn fired(status: &str, reply: Option<&str>, code: Option<&str>) -> FiredTurn {
    FiredTurn {
        turn_id: "bt_x".into(),
        status: status.into(),
        reply: reply.map(|text| TurnReply {
            message_id: "re-x".into(),
            text: text.into(),
        }),
        error_code: code.map(str::to_string),
    }
}

#[test]
fn a_settled_turn_answers_and_an_unsettled_one_is_pending() {
    let callee = member(LEDGER, "Ledger");
    let (id, text) = answer_from(&fired("done", Some("42"), None), &callee).unwrap();
    assert_eq!((id.as_str(), text.as_str()), ("re-x", "42"));
    let pending = answer_from(&fired("queued", None, None), &callee).unwrap_err();
    assert!(pending.downcast_ref::<PendingAnswer>().is_some());
    let running = answer_from(&fired("running", None, None), &callee).unwrap_err();
    assert!(running.downcast_ref::<PendingAnswer>().is_some());
    let failed = answer_from(
        &fired("failed", Some("sorry"), Some("BOT_TURN_FAILED")),
        &callee,
    )
    .unwrap_err();
    assert!(failed.downcast_ref::<PendingAnswer>().is_none());
    let cancelled = answer_from(&fired("cancelled", None, Some("BOT_ASK_EXPIRED")), &callee);
    assert!(cancelled.is_err());
}

#[test]
fn the_ask_body_carries_the_chain_and_only_waits_when_asked_to() {
    let chain = vec!["a".to_string(), "b".to_string()];
    let question = Question {
        text: "Hi",
        callee_id: "c",
        ask_id: "ask-9",
        origin_conversation: "conv",
        hops: 2,
        chain: &chain,
        wait_secs: 0,
        round: 0,
    };
    let body = ask_body(&question, at(0));
    assert_eq!(body["mentions"], json!(["c"]));
    assert_eq!(body["ask"]["chain"], json!(["a", "b"]));
    assert!(body["ask"].get("wait_secs").is_none());
    assert!(body["ask"].get("round").is_none());
    assert_eq!(body["ask"]["hops"], 2);
    let waiting = ask_body(
        &Question {
            wait_secs: 90,
            round: 3,
            ..question
        },
        at(0),
    );
    assert_eq!(waiting["ask"]["wait_secs"], 90);
    assert_eq!(waiting["ask"]["round"], 3);
}

#[tokio::test]
async fn an_answer_to_a_counted_question_wakes_the_asker_at_its_own_hops() {
    let (db, conversation) = pair("SERVER", "SERVER").await;
    ask_round(&db, &conversation, "ask-r1", 1, WAIT_SECS, 1).await;
    let reply = answer(&db, &conversation, "ask-r1", 2).await;
    let woken = turn(&db, SCOUT, &reply)
        .await
        .expect("asker follow-up turn");
    assert_eq!(woken.hops, 1);
    let answered = answered_ask(&db, &woken)
        .await
        .unwrap()
        .expect("answered ask");
    assert_eq!(answered.round, 1);
    assert_eq!(answered.callee_id, LEDGER);

    ask_round(&db, &conversation, "ask-r2", 3, WAIT_SECS, 2).await;
    let deep = answer(&db, &conversation, "ask-r2", 4).await;
    let woken = turn(&db, SCOUT, &deep)
        .await
        .expect("a follow-up past the hop limit still reaches the asker");
    assert_eq!(woken.hops, 2);
}

#[tokio::test]
async fn an_answer_to_a_question_without_a_round_counts_its_hops_as_before() {
    let (db, conversation) = pair("SERVER", "SERVER").await;
    ask(&db, &conversation, "ask-l1", 1, WAIT_SECS).await;
    let reply = answer(&db, &conversation, "ask-l1", 2).await;
    let woken = turn(&db, SCOUT, &reply)
        .await
        .expect("asker follow-up turn");
    assert_eq!(woken.hops, 2);
    assert_eq!(answered_ask(&db, &woken).await.unwrap().unwrap().round, 0);

    ask(&db, &conversation, "ask-l2", 3, WAIT_SECS).await;
    let past = answer(&db, &conversation, "ask-l2", 4).await;
    assert!(turn(&db, SCOUT, &past).await.is_none());
}

#[tokio::test]
async fn a_desktop_turns_late_answer_returns_to_the_conversation_the_turn_runs_in() {
    let (db, conversation) = pair("DESKTOP", "DESKTOP").await;
    ask(&db, &conversation, "ask-7", 1, WAIT_SECS).await;
    let queued = turn(&db, LEDGER, "ask-7").await.expect("queued turn");
    assert_eq!(
        turn_conversation(&db, OWNER, LEDGER, &queued.id)
            .await
            .unwrap()
            .as_deref(),
        Some(conversation.as_str())
    );
    for (owner, bot, run) in [
        ("someone-else", LEDGER, queued.id.as_str()),
        (OWNER, SCOUT, queued.id.as_str()),
        (OWNER, LEDGER, "a-chat-run"),
        (OWNER, LEDGER, ""),
    ] {
        assert_eq!(
            turn_conversation(&db, owner, bot, run).await.unwrap(),
            None,
            "{owner} {bot} {run}"
        );
    }
}

async fn activity_rows(db: &DatabaseConnection, inbox: &str) -> Vec<inbox_entry::Model> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox))
        .filter(inbox_entry::Column::Kind.eq(crate::routes::inbox::stored(
            InboxEntryKind::Activity.as_str(),
        )))
        .all(db)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_ask_leaves_one_activity_in_the_askers_chat_and_the_callees_chat_and_wakes_no_bot() {
    let (db, exchange) = pair("SERVER", "SERVER").await;
    let scout_chat = open_conversation(&db, OWNER, SCOUT, "", now_millis())
        .await
        .unwrap()
        .unwrap();
    let ledger_chat = open_conversation(&db, OWNER, LEDGER, "", now_millis())
        .await
        .unwrap()
        .unwrap();
    let question = "q".repeat(500);
    let body = ActivityBody::bot_ask("ask-9", SCOUT, LEDGER, &exchange, &question);

    for _ in 0..2 {
        let targets = bot_ask_targets(&db, OWNER, &scout_chat, &body)
            .await
            .unwrap();
        assert_eq!(
            targets,
            [
                (scout_chat.clone(), SCOUT.to_string()),
                (ledger_chat.clone(), LEDGER.to_string())
            ]
        );
        for (conversation, sender) in targets {
            let id = activity_message_id("ask-9", &conversation);
            let sent = write_activity_in(&db, &conversation, &sender, &id, &body, now_millis())
                .await
                .unwrap();
            assert!(sent.bots.is_empty());
        }
    }

    for (chat, sender) in [(&scout_chat, SCOUT), (&ledger_chat, LEDGER)] {
        let rows = activity_rows(&db, OWNER).await;
        let row = rows
            .iter()
            .find(|row| &row.conversation_id == chat)
            .unwrap();
        assert_eq!(row.sender_id, sender);
        assert_eq!(row.message_id, activity_message_id("ask-9", chat));
        let parsed = ActivityBody::parse(&row.envelope["body"]).unwrap();
        assert_eq!(parsed, body);
        let ActivityBody::BotAsk { question, .. } = parsed;
        assert_eq!(question.chars().count(), 300);
        assert_eq!(activity_rows(&db, sender).await.len(), 1);
    }
    assert_eq!(activity_rows(&db, OWNER).await.len(), 2);
    assert!(
        turn(&db, LEDGER, &activity_message_id("ask-9", &ledger_chat))
            .await
            .is_none()
    );
    assert!(
        turn(&db, SCOUT, &activity_message_id("ask-9", &scout_chat))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn an_ask_does_not_start_the_callees_chat_and_writes_once_when_both_chats_are_one() {
    let (db, exchange) = pair("SERVER", "SERVER").await;
    let scout_chat = open_conversation(&db, OWNER, SCOUT, "", now_millis())
        .await
        .unwrap()
        .unwrap();
    let body = ActivityBody::bot_ask("ask-8", SCOUT, LEDGER, &exchange, "hi");
    let targets = bot_ask_targets(&db, OWNER, &scout_chat, &body)
        .await
        .unwrap();
    assert_eq!(targets, [(scout_chat, SCOUT.to_string())]);
    let ledger_chat = open_conversation(&db, OWNER, LEDGER, "", now_millis())
        .await
        .unwrap()
        .unwrap();
    let from_ledger_chat = bot_ask_targets(&db, OWNER, &ledger_chat, &body)
        .await
        .unwrap();
    assert_eq!(from_ledger_chat.len(), 1);
}
