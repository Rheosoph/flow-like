use super::*;
use crate::routes::inbox::fixture::{database, insert_bot, running_turn};
use chrono::Duration;

const BOT: &str = "bot_1";
const TURN: &str = "turn_1";
const HOST: &str = "device:one";

fn principal(turn: &str) -> BotUser {
    BotUser {
        bot_id: BOT.into(),
        turn_id: turn.into(),
        conv_id: Some("room".into()),
    }
}

async fn held_turn(lease_hours: i64) -> sea_orm::DatabaseConnection {
    let db = database().await;
    insert_bot(&db, BOT, "user-a", "DEVICE").await;
    let lease = (chrono::Utc::now() + Duration::hours(lease_hours)).fixed_offset();
    running_turn(&db, TURN, BOT, "room", HOST, lease).await;
    db
}

#[tokio::test]
async fn the_host_holding_the_turn_may_run_an_app_tool() {
    let db = held_turn(1).await;
    let row = held_by_host(&db, &principal(TURN), BOT, TURN, HOST, "list_apps")
        .await
        .unwrap();
    assert_eq!(row.id, BOT);
}

#[tokio::test]
async fn another_host_is_refused_as_a_lost_lease() {
    let db = held_turn(1).await;
    let error = held_by_host(&db, &principal(TURN), BOT, TURN, "device:two", "list_apps")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_expired_lease_is_refused() {
    let db = held_turn(-1).await;
    let error = held_by_host(&db, &principal(TURN), BOT, TURN, HOST, "list_apps")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_tool_that_is_not_an_app_tool_is_refused() {
    let db = held_turn(1).await;
    let error = held_by_host(&db, &principal(TURN), BOT, TURN, HOST, "web_search")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_token_of_another_turn_or_bot_is_refused() {
    let db = held_turn(1).await;
    let other_turn = held_by_host(&db, &principal("turn_2"), BOT, TURN, HOST, "list_apps")
        .await
        .unwrap_err();
    assert_eq!(other_turn.status(), StatusCode::FORBIDDEN);
    let other_bot = held_by_host(&db, &principal(TURN), "bot_2", TURN, HOST, "list_apps")
        .await
        .unwrap_err();
    assert_eq!(other_bot.status(), StatusCode::FORBIDDEN);
}
