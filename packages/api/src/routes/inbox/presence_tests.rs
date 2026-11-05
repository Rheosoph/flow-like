use super::presence::{active_turns_in, claims_for, room_key_in, room_member};
use crate::{
    entity::{bot_turn, conversation_member},
    middleware::jwt::{AppUser, BotUser, OpenIDUser},
    routes::inbox::model::ReaderQuery,
    routes::inbox::{
        conversations::{NewConversation, start_group},
        fixture::{at, bot_principal, database, insert_bot, running_turn, user},
    },
};
use axum::http::StatusCode;
use chrono::NaiveDate;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, sea_query::Expr};

const MASTER: [u8; 32] = [7; 32];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

async fn team() -> DatabaseConnection {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "DESKTOP").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("team".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    let others = [user("user-b"), bot_principal("bot_1")];
    start_group(&db, "TEAM", "user-a", &plan, &others, at(0))
        .await
        .unwrap();
    db
}

async fn set_status(db: &DatabaseConnection, id: &str, status: &str) {
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::Status, Expr::value(status))
        .filter(bot_turn::Column::Id.eq(id))
        .exec(db)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_member_lists_only_unfinished_turns_oldest_first() {
    let db = team().await;
    for id in ["bt_1", "bt_2", "bt_3", "bt_4"] {
        running_turn(&db, id, "bot_1", "TEAM", "host", at(900)).await;
    }
    bot_turn::Entity::update_many()
        .col_expr(bot_turn::Column::CreatedAt, Expr::value(at(5)))
        .filter(bot_turn::Column::Id.eq("bt_1"))
        .exec(&db)
        .await
        .unwrap();
    set_status(&db, "bt_2", "QUEUED").await;
    set_status(&db, "bt_3", "DONE").await;
    set_status(&db, "bt_4", "WAITING").await;

    let turns = active_turns_in(&db, "TEAM", "user-b").await.unwrap();
    let seen: Vec<_> = turns
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();
    assert_eq!(
        seen,
        [("bt_2", "queued"), ("bt_4", "waiting"), ("bt_1", "running")]
    );

    let wire = serde_json::to_value(&turns[0]).unwrap();
    let mut keys: Vec<_> = wire.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "bot_id",
            "created_at",
            "id",
            "status",
            "trigger_message_id",
            "updated_at"
        ]
    );
}

#[tokio::test]
async fn a_non_member_gets_the_missing_conversation_404() {
    let db = team().await;
    running_turn(&db, "bt_1", "bot_1", "TEAM", "host", at(900)).await;
    let error = active_turns_in(&db, "TEAM", "user-x").await.unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    let error = room_key_in(&db, &MASTER, "TEAM", "user-x", day(4))
        .await
        .err()
        .unwrap();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_member_who_left_is_refused_both() {
    let db = team().await;
    conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::LeftAt, Expr::value(at(10)))
        .filter(conversation_member::Column::PrincipalId.eq("user-b"))
        .exec(&db)
        .await
        .unwrap();
    assert!(active_turns_in(&db, "TEAM", "user-b").await.is_err());
    assert!(
        room_key_in(&db, &MASTER, "TEAM", "user-b", day(4))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_key_id_is_stable_for_the_same_members_and_changes_with_them() {
    let db = team().await;
    let first = room_key_in(&db, &MASTER, "TEAM", "user-a", day(4))
        .await
        .unwrap();
    let again = room_key_in(&db, &MASTER, "TEAM", "user-b", day(4))
        .await
        .unwrap();
    assert_eq!(first.key_id, again.key_id);
    assert_eq!(first.encryption_key, again.encryption_key);
    let (date, digest) = first.key_id.split_once('.').unwrap();
    assert_eq!(date, "2026-11-04");
    assert_eq!(digest.len(), 16);
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));

    let tomorrow = room_key_in(&db, &MASTER, "TEAM", "user-a", day(5))
        .await
        .unwrap();
    assert_ne!(tomorrow.key_id, first.key_id);
    assert_ne!(tomorrow.encryption_key, first.encryption_key);

    let other_master = room_key_in(&db, &[8; 32], "TEAM", "user-a", day(4))
        .await
        .unwrap();
    assert_eq!(other_master.key_id, first.key_id);
    assert_ne!(other_master.encryption_key, first.encryption_key);

    conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::LeftAt, Expr::value(at(10)))
        .filter(conversation_member::Column::PrincipalId.eq("user-b"))
        .exec(&db)
        .await
        .unwrap();
    let after = room_key_in(&db, &MASTER, "TEAM", "user-a", day(4))
        .await
        .unwrap();
    assert_ne!(after.key_id, first.key_id);
    assert_ne!(after.encryption_key, first.encryption_key);
}

#[test]
fn the_claims_name_the_conversation_and_no_board() {
    let claims = claims_for("user-a".into(), Some("Ann".into()), "TEAM", "jti".into());
    let wire = serde_json::to_value(&claims).unwrap();
    assert_eq!(wire["conversation_id"], "TEAM");
    assert_eq!(wire["scope"], "realtime.read");
    assert_eq!(wire["typ"], "realtime");
    assert!(wire.get("app_id").is_none());
    assert!(wire.get("board_id").is_none());
    assert!(wire.get("board_format_version").is_none());
}

#[test]
fn only_a_person_without_as_gets_room_access() {
    let person = AppUser::OpenID(OpenIDUser {
        sub: "user-a".into(),
        access_token: String::new(),
    });
    let own = ReaderQuery::default();
    assert_eq!(room_member(&person, &own).unwrap(), "user-a");

    let as_bot = ReaderQuery {
        as_bot: Some("bot_1".into()),
    };
    assert_eq!(
        room_member(&person, &as_bot).unwrap_err().status(),
        StatusCode::BAD_REQUEST
    );

    let bot = AppUser::Bot(BotUser {
        bot_id: "bot_1".into(),
        turn_id: "bt_1".into(),
        conv_id: Some("TEAM".into()),
    });
    assert_eq!(
        room_member(&bot, &own).unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
    assert!(room_member(&AppUser::Unauthorized, &own).is_err());
}
