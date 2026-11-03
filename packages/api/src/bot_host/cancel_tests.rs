use super::{
    fixture::{assert_code, at, database, delivery, insert_bot, recipient},
    ledger::{
        BOT_TURN_LEASE_LOST, BOT_TURN_NOT_CANCELLABLE, ClaimRequest, ERROR_OWNER_CANCELLED,
        Refusal, cancel_owned_in, claim_in, enqueue_in, heartbeat_in, holds_lease,
    },
};
use crate::entity::bot_turn;
use axum::http::StatusCode;
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{STORED_CANCELLED, STORED_DONE, STORED_QUEUED, TurnHostKind, turn_id},
    inbox::ConversationKind,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr};

async fn queued(db: &DatabaseConnection) -> String {
    insert_bot(db, "bot_1", "user-a", "DESKTOP").await;
    enqueue_in(
        db,
        &delivery(
            ConversationKind::Direct,
            vec![recipient("bot_1", "user-a", BotLocation::Desktop)],
        ),
        at(1),
    )
    .await
    .unwrap();
    turn_id("bot_1", "m1")
}

fn claim_request(turn: &str, host: &str) -> ClaimRequest {
    ClaimRequest {
        turn_id: turn.into(),
        owner_id: Some("user-a".into()),
        host_kind: TurnHostKind::Desktop,
        host_id: host.into(),
        now: at(10),
    }
}

async fn stored(db: &DatabaseConnection, turn: &str) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(turn)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cancelling_a_queued_turn_ends_it_before_any_host_claims_it() {
    let db = database().await;
    let turn = queued(&db).await;
    let cancelled = cancel_owned_in(&db, "user-a", "bot_1", &turn, at(5))
        .await
        .unwrap();
    assert_eq!(cancelled.status, STORED_CANCELLED);
    assert_eq!(cancelled.error_code.as_deref(), Some(ERROR_OWNER_CANCELLED));

    let refusal = claim_in(&db, &claim_request(&turn, "desk-1"))
        .await
        .unwrap();
    assert!(matches!(refusal, Err(Refusal::NotQueued(_))));
}

#[tokio::test]
async fn cancelling_a_running_turn_revokes_the_lease() {
    let db = database().await;
    let turn = queued(&db).await;
    claim_in(&db, &claim_request(&turn, "desk-1"))
        .await
        .unwrap()
        .unwrap();
    assert!(holds_lease(&db, &turn, "desk-1", at(20)).await.unwrap());
    heartbeat_in(&db, &turn, Some("user-a"), "desk-1", at(20))
        .await
        .unwrap();

    let cancelled = cancel_owned_in(&db, "user-a", "bot_1", &turn, at(30))
        .await
        .unwrap();
    assert_eq!(cancelled.status, STORED_CANCELLED);
    assert_eq!(stored(&db, &turn).await.lease_until, None);
    assert!(!holds_lease(&db, &turn, "desk-1", at(31)).await.unwrap());

    let error = heartbeat_in(&db, &turn, Some("user-a"), "desk-1", at(32))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, BOT_TURN_LEASE_LOST);
}

#[tokio::test]
async fn a_finished_turn_cannot_be_cancelled() {
    let db = database().await;
    let turn = queued(&db).await;
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(STORED_DONE))
        .filter(bot_turn::Column::Id.eq(turn.as_str()))
        .exec(&db)
        .await
        .unwrap();
    let error = cancel_owned_in(&db, "user-a", "bot_1", &turn, at(5))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, BOT_TURN_NOT_CANCELLABLE);
    assert_eq!(stored(&db, &turn).await.status, STORED_DONE);

    let again = cancel_owned_in(&db, "user-a", "bot_1", &turn, at(5))
        .await
        .unwrap_err();
    assert_code(&again, StatusCode::CONFLICT, BOT_TURN_NOT_CANCELLABLE);
}

#[tokio::test]
async fn only_the_owner_of_that_bot_cancels_its_turn() {
    let db = database().await;
    let turn = queued(&db).await;
    insert_bot(&db, "bot_2", "user-a", "DESKTOP").await;

    let stranger = cancel_owned_in(&db, "user-b", "bot_1", &turn, at(5))
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);
    let other_bot = cancel_owned_in(&db, "user-a", "bot_2", &turn, at(5))
        .await
        .unwrap_err();
    assert_eq!(other_bot.status(), StatusCode::NOT_FOUND);
    let missing = cancel_owned_in(&db, "user-a", "bot_1", "bt_missing", at(5))
        .await
        .unwrap_err();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(stored(&db, &turn).await.status, STORED_QUEUED);
}
