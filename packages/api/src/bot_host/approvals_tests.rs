use super::{
    approvals::{
        BOT_APPROVAL_UNKNOWN, BOT_TURN_NOT_WAITING, Expired, GrantRows, answer_in, expire_in,
        grant_in, list_grants, park_in, revoke_in,
    },
    fixture::{assert_code, at, conversation, database, delivery, insert_bot, recipient},
    ledger::{self, BOT_TURN_LEASE_LOST, ClaimRequest, claim_in, enqueue_in, recover_in},
};
use crate::{
    entity::{bot_tool_grant, bot_turn, inbox_entry},
    routes::inbox::now_millis,
};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset};
use flow_like::flow::copilot::tool_grants::{GrantCheck, GrantStore, approval_request, check_call};
use flow_like_types::{
    bot::BotLocation,
    bot_message::ApprovalRequest,
    bot_turn::{STORED_CANCELLED, STORED_QUEUED, STORED_WAITING, TurnHostKind, turn_id},
    inbox::{ConversationKind, direct_pair_key},
    tool_grant::{
        APPROVAL_WAIT_DAYS, ApprovalAnswer, ApprovalDecision, ERROR_APPROVAL_EXPIRED, ToolGrantKey,
        args_hash,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    sea_query::Expr,
};
use serde_json::{Value, json};
use std::sync::Arc;

const OWNER: &str = "user-a";
const BOT: &str = "bot_1";
const HOST: &str = "server:1";

fn key(tool: &str, scope: &str) -> ToolGrantKey {
    ToolGrantKey {
        tool: tool.into(),
        scope: scope.into(),
    }
}

fn drop_args() -> Value {
    json!({ "index": "notes" })
}

async fn grants_db() -> DatabaseConnection {
    let db = database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."BotToolGrant" (
            "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
            "tool" TEXT NOT NULL, "scope" TEXT NOT NULL, "grantedAt" TEXT NOT NULL,
            "grantedBy" TEXT NOT NULL, UNIQUE ("botId", "tool", "scope"))"#,
    )
    .await
    .unwrap();
    db
}

/// The owner's direct conversation with a bot whose turn for `m1` runs under `HOST`.
async fn running() -> (DatabaseConnection, bot_turn::Model) {
    let db = grants_db().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    conversation(&db, "c1", "DIRECT", "PLAIN", &[OWNER, BOT]).await;
    db.execute_unprepared(&format!(
        r#"UPDATE public."Conversation" SET "directKey" = '{}' WHERE "id" = 'c1'"#,
        direct_pair_key(OWNER, BOT)
    ))
    .await
    .unwrap();
    let mut target = recipient(BOT, OWNER, BotLocation::Server);
    target.conversation_id = "c1".into();
    target.trigger_message_id = "m1".into();
    enqueue_in(
        &db,
        &delivery(ConversationKind::Direct, vec![target]),
        at(1),
    )
    .await
    .unwrap();
    let now = now_millis();
    claim_in(
        &db,
        &ClaimRequest {
            turn_id: turn_id(BOT, "m1"),
            owner_id: None,
            host_kind: TurnHostKind::Server,
            host_id: HOST.into(),
            now,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let turn = ledger::find(&db, &turn_id(BOT, "m1"), None)
        .await
        .unwrap()
        .unwrap();
    (db, turn)
}

fn card(turn: &bot_turn::Model) -> ApprovalRequest {
    approval_request(
        &turn.id,
        "knowledge_drop",
        &drop_args(),
        None,
        "Drops 'notes'.",
    )
}

async fn parked() -> (DatabaseConnection, bot_turn::Model, ApprovalRequest) {
    let (db, turn) = running().await;
    let card = card(&turn);
    park_in(&db, &turn, HOST, &card, "c1", now_millis())
        .await
        .unwrap();
    let turn = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    (db, turn, card)
}

fn answer(card: &ApprovalRequest, decision: ApprovalDecision) -> ApprovalAnswer {
    ApprovalAnswer {
        approval_id: card.id.clone(),
        decision,
    }
}

async fn resolve(
    db: &DatabaseConnection,
    turn: &bot_turn::Model,
    card: &ApprovalRequest,
    decision: ApprovalDecision,
) -> Result<bot_turn::Model, crate::error::ApiError> {
    answer_in(
        db,
        OWNER,
        BOT,
        &turn.id,
        &answer(card, decision),
        "c1",
        now_millis(),
    )
    .await
    .map(|(turn, _)| turn)
}

fn rows(db: &DatabaseConnection) -> GrantRows<Arc<DatabaseConnection>> {
    GrantRows::new(Arc::new(db.clone()), BOT, OWNER)
}

#[tokio::test]
async fn a_grant_is_the_owners_alone_idempotent_and_listed_without_tokens() {
    let db = grants_db().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    insert_bot(&db, "bot_other", "user-b", "SERVER").await;
    let first = grant_in(&db, OWNER, BOT, &key("code_run", "host:a.com"), at(5))
        .await
        .unwrap();
    let again = grant_in(&db, OWNER, BOT, &key("code_run", "host:a.com"), at(900))
        .await
        .unwrap();
    assert_eq!(first, again);
    rows(&db).put("code_run", "once:abc").await.unwrap();
    assert_eq!(list_grants(&db, OWNER, BOT, false).await.unwrap().len(), 1);
    assert_eq!(list_grants(&db, OWNER, BOT, true).await.unwrap().len(), 2);

    for stranger in ["user-b", "user-c"] {
        assert_code(
            &list_grants(&db, stranger, BOT, false).await.unwrap_err(),
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
        );
        assert!(
            grant_in(&db, stranger, BOT, &key("code_run", "*"), at(1))
                .await
                .is_err()
        );
        assert!(
            revoke_in(&db, stranger, BOT, &key("code_run", "host:a.com"))
                .await
                .is_err()
        );
    }
    assert!(list_grants(&db, OWNER, "bot_other", false).await.is_err());
    assert_eq!(bot_tool_grant::Entity::find().count(&db).await.unwrap(), 2);

    assert!(
        revoke_in(&db, OWNER, BOT, &key("code_run", "host:a.com"))
            .await
            .unwrap()
    );
    assert!(
        !revoke_in(&db, OWNER, BOT, &key("code_run", "host:a.com"))
            .await
            .unwrap()
    );
    for bad in [key("code run", "*"), key("code_run", "bogus")] {
        assert_eq!(
            grant_in(&db, OWNER, BOT, &bad, at(1))
                .await
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn a_grant_covers_exactly_its_scope_for_the_core_lookup() {
    let db = grants_db().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    let store = rows(&db);
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Missing
    );
    grant_in(
        &db,
        OWNER,
        BOT,
        &key("knowledge_drop", "table:notes"),
        at(1),
    )
    .await
    .unwrap();
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Granted
    );
    assert_eq!(
        check_call(&store, "knowledge_drop", &json!({ "index": "other" }), None)
            .await
            .unwrap(),
        GrantCheck::Missing
    );
    let other_bot = GrantRows::new(Arc::new(db.clone()), "bot_x", OWNER);
    assert_eq!(
        check_call(&other_bot, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Missing
    );
}

#[tokio::test]
async fn parking_posts_the_card_waits_without_a_lease_and_the_host_can_do_it_once() {
    let (db, turn) = running().await;
    let card = card(&turn);
    let sent = park_in(&db, &turn, HOST, &card, "c1", now_millis())
        .await
        .unwrap();
    assert_eq!(sent.entry.sender_id, BOT);
    let parked = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    assert_eq!(parked.status, STORED_WAITING);
    assert_eq!(parked.reply_message_id.as_deref(), Some(card.id.as_str()));
    assert!(parked.lease_until.is_none() && parked.host_id.is_none());
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(card.id.as_str()))
        .filter(inbox_entry::Column::InboxId.eq(OWNER))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.envelope["body"]["approval"]["tool"], "knowledge_drop");
    assert_eq!(entry.envelope["body"]["approval"]["scope"], "table:notes");
    assert_code(
        &park_in(&db, &turn, HOST, &card, "c1", now_millis())
            .await
            .unwrap_err(),
        StatusCode::CONFLICT,
        BOT_TURN_LEASE_LOST,
    );
}

#[tokio::test]
async fn a_host_that_lost_its_lease_cannot_park() {
    let (db, turn) = running().await;
    let card = card(&turn);
    assert_code(
        &park_in(&db, &turn, "server:other", &card, "c1", now_millis())
            .await
            .unwrap_err(),
        StatusCode::CONFLICT,
        BOT_TURN_LEASE_LOST,
    );
    let still = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    assert_eq!(still.status, "RUNNING");
    assert_eq!(inbox_entry::Entity::find().count(&db).await.unwrap(), 0);
}

#[tokio::test]
async fn once_leaves_a_token_queues_the_turn_and_wakes_no_new_turn() {
    let (db, turn, card) = parked().await;
    let queued = resolve(&db, &turn, &card, ApprovalDecision::Once)
        .await
        .unwrap();
    assert_eq!(queued.status, STORED_QUEUED);
    assert!(queued.reply_message_id.is_none() && queued.lease_until.is_none());
    assert_eq!(bot_turn::Entity::find().count(&db).await.unwrap(), 1);
    assert!(
        list_grants(&db, OWNER, BOT, false)
            .await
            .unwrap()
            .is_empty()
    );
    let store = rows(&db);
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Once
    );
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Missing
    );
    let response = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(OWNER))
        .filter(inbox_entry::Column::SenderId.eq(OWNER))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response.envelope["body"]["approval_response"]["decision"],
        "once"
    );
    assert_eq!(response.envelope["body"]["reply_to"], card.id.as_str());
}

#[tokio::test]
async fn always_stores_a_grant_that_covers_the_next_call() {
    let (db, turn, card) = parked().await;
    resolve(&db, &turn, &card, ApprovalDecision::Always)
        .await
        .unwrap();
    let grants = list_grants(&db, OWNER, BOT, false).await.unwrap();
    assert_eq!(
        grants
            .iter()
            .map(|g| (g.tool.as_str(), g.scope.as_str()))
            .collect::<Vec<_>>(),
        [("knowledge_drop", "table:notes")]
    );
    let store = rows(&db);
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Granted
    );
}

#[tokio::test]
async fn deny_leaves_a_refusal_the_resumed_run_is_told() {
    let (db, turn, card) = parked().await;
    let queued = resolve(&db, &turn, &card, ApprovalDecision::Deny)
        .await
        .unwrap();
    assert_eq!(queued.status, STORED_QUEUED);
    assert!(
        list_grants(&db, OWNER, BOT, false)
            .await
            .unwrap()
            .is_empty()
    );
    let store = rows(&db);
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Denied
    );
}

#[tokio::test]
async fn an_answer_needs_the_waiting_turn_its_card_and_its_owner() {
    let (db, turn, card) = parked().await;
    let wrong = ApprovalAnswer {
        approval_id: "ap-other".into(),
        decision: ApprovalDecision::Always,
    };
    let refused = answer_in(&db, OWNER, BOT, &turn.id, &wrong, "c1", now_millis())
        .await
        .unwrap_err();
    assert_code(&refused, StatusCode::CONFLICT, BOT_APPROVAL_UNKNOWN);
    assert!(
        answer_in(
            &db,
            "user-b",
            BOT,
            &turn.id,
            &answer(&card, ApprovalDecision::Always),
            "c1",
            now_millis()
        )
        .await
        .is_err()
    );
    assert!(list_grants(&db, OWNER, BOT, true).await.unwrap().is_empty());

    resolve(&db, &turn, &card, ApprovalDecision::Once)
        .await
        .unwrap();
    let again = resolve(&db, &turn, &card, ApprovalDecision::Always)
        .await
        .unwrap_err();
    assert_code(&again, StatusCode::CONFLICT, BOT_TURN_NOT_WAITING);
    assert!(
        list_grants(&db, OWNER, BOT, false)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn recovery_leaves_a_waiting_turn_alone_and_seven_days_cancel_it_with_a_notice() {
    let (db, turn, _) = parked().await;
    let long_after = now_millis() + Duration::days(30);
    recover_in(&db, long_after).await.unwrap();
    let waiting = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    assert_eq!(waiting.status, STORED_WAITING);

    let now = now_millis();
    assert!(
        expire_in(&db, &waiting, Some("c1"), now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ledger::find(&db, &turn.id, None)
            .await
            .unwrap()
            .unwrap()
            .status,
        STORED_WAITING
    );
    let eight_days_ago: DateTime<FixedOffset> = now - Duration::days(APPROVAL_WAIT_DAYS + 1);
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::UpdatedAt, Expr::value(eight_days_ago))
        .filter(bot_turn::Column::Id.eq(turn.id.as_str()))
        .exec(&db)
        .await
        .unwrap();
    let stale = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    let notice = expire_in(&db, &stale, Some("c1"), now)
        .await
        .unwrap()
        .unwrap();
    assert!(
        notice.entry.envelope["body"]["text"]
            .as_str()
            .unwrap()
            .contains("seven days")
    );
    let ended = ledger::find(&db, &turn.id, None).await.unwrap().unwrap();
    assert_eq!(
        (ended.status.as_str(), ended.error_code.as_deref()),
        (STORED_CANCELLED, Some(ERROR_APPROVAL_EXPIRED))
    );
    let _ = Expired::default();
}

#[tokio::test]
async fn answer_tokens_older_than_a_day_are_ignored_and_swept() {
    let db = grants_db().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    let store = rows(&db);
    let hash = args_hash("knowledge_drop", &drop_args());
    store
        .put("knowledge_drop", &format!("once:{hash}"))
        .await
        .unwrap();
    store.put("knowledge_drop", "table:old").await.unwrap();
    let old: DateTime<FixedOffset> = now_millis() - Duration::hours(25);
    bot_tool_grant::Entity::update_many()
        .col_expr(bot_tool_grant::Column::GrantedAt, Expr::value(old))
        .exec(&db)
        .await
        .unwrap();
    let listed = store.list().await.unwrap();
    assert_eq!(
        listed.iter().map(|k| k.scope.as_str()).collect::<Vec<_>>(),
        ["table:old"]
    );
    assert_eq!(
        check_call(&store, "knowledge_drop", &drop_args(), None)
            .await
            .unwrap(),
        GrantCheck::Missing
    );
}

#[tokio::test]
async fn the_parked_card_names_who_woke_the_turn() {
    let (db, mut turn) = running().await;
    turn.requester_id = Some("user-c".into());
    let card = card(&turn);
    park_in(&db, &turn, HOST, &card, "c1", now_millis())
        .await
        .unwrap();
    let posted = crate::routes::inbox::approvals::card_in(&db, OWNER, "c1", &card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(posted.requester_id.as_deref(), Some("user-c"));
    assert_eq!(posted.room, None);
}
