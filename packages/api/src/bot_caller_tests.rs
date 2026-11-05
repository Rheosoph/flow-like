use super::*;
use crate::middleware::jwt::OpenIDUser;
use axum::http::StatusCode;

fn bot(conv: Option<&str>) -> BotUser {
    BotUser {
        bot_id: "bot_1".into(),
        turn_id: "bt_1".into(),
        conv_id: conv.map(str::to_owned),
    }
}

fn status(result: Result<(), ApiError>) -> Option<StatusCode> {
    result.err().map(|error| error.status())
}

#[test]
fn as_must_be_absent_or_the_tokens_bot() {
    let bot = bot(None);
    assert_eq!(status(ensure_as_self(&bot, None)), None);
    assert_eq!(status(ensure_as_self(&bot, Some("  "))), None);
    assert_eq!(status(ensure_as_self(&bot, Some("bot_1"))), None);
    assert_eq!(status(ensure_as_self(&bot, Some(" bot_1 "))), None);
    assert_eq!(
        status(ensure_as_self(&bot, Some("bot_2"))),
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        status(ensure_as_self(&bot, Some("user-a"))),
        Some(StatusCode::FORBIDDEN)
    );
}

#[test]
fn a_turn_route_names_the_tokens_turn() {
    let bot = bot(None);
    assert_eq!(status(ensure_turn(&bot, "bt_1")), None);
    assert_eq!(
        status(ensure_turn(&bot, "bt_2")),
        Some(StatusCode::FORBIDDEN)
    );
}

#[test]
fn a_room_token_reaches_only_its_room() {
    let room = bot(Some("ROOM-1"));
    assert_eq!(status(ensure_room(&room, "room-1")), None);
    assert_eq!(
        status(ensure_room(&room, "room-2")),
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(status(ensure_room(&bot(None), "room-2")), None);
}

#[test]
fn check_applies_every_name() {
    let room = bot(Some("room-1"));
    let names = Names {
        as_bot: Some("bot_1"),
        conversation: Some("room-1"),
        turn: Some("bt_1"),
    };
    assert_eq!(status(check(&room, &names)), None);
    for wrong in [
        Names {
            as_bot: Some("bot_2"),
            ..names
        },
        Names {
            conversation: Some("room-9"),
            ..names
        },
        Names {
            turn: Some("bt_9"),
            ..names
        },
    ] {
        assert_eq!(status(check(&room, &wrong)), Some(StatusCode::FORBIDDEN));
    }
}

#[test]
fn a_bot_write_names_the_tokens_bot_never_as() {
    let token = AppUser::Bot(bot(None));
    assert_eq!(bot_named(&token, Some("bot_2")), Some("bot_1"));
    assert_eq!(bot_named(&token, None), Some("bot_1"));
    let person = AppUser::OpenID(OpenIDUser {
        sub: "user-a".into(),
        access_token: String::new(),
    });
    assert_eq!(bot_named(&person, Some(" bot_2 ")), Some("bot_2"));
    assert_eq!(bot_named(&person, None), None);
}

#[tokio::test]
async fn model_usage_requires_live_bot_and_active_group_membership() {
    use crate::entity::conversation_member;
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ConnectOptions, Database, Schema, sea_query::Table,
    };
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(bot::Entity),
        schema.create_table_from_entity(conversation_member::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    let now = chrono::Utc::now().fixed_offset();
    bot::ActiveModel {
        id: Set("bot_1".into()),
        owner_id: Set("owner".into()),
        appearance: Set(serde_json::json!({})),
        model_policy: Set(serde_json::json!({})),
        pii: Set(false),
        capabilities: Set(vec![].into()),
        location: Set("SERVER".into()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let direct = crate::routes::bots::runtime_token::bot_user_for_direct(&db, "owner", "bot_1")
        .await
        .unwrap();
    assert!(direct.turn_id.starts_with("direct:"));
    assert_eq!(direct.bot_id, "bot_1");
    assert!(direct.conv_id.is_none());
    assert!(
        crate::routes::bots::runtime_token::bot_user_for_direct(&db, "other", "bot_1")
            .await
            .is_err()
    );
    assert_eq!(
        usage_identity(&db, &bot(None)).await.unwrap(),
        ("owner".into(), None)
    );
    assert!(usage_identity(&db, &bot(Some("group"))).await.is_err());
    conversation_member::ActiveModel {
        conversation_id: Set("group".into()),
        principal_id: Set("bot_1".into()),
        principal_kind: Set("BOT".into()),
        status: Set("ACTIVE".into()),
        joined_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    assert_eq!(
        usage_identity(&db, &bot(Some("group"))).await.unwrap(),
        ("owner".into(), Some("group".into()))
    );
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::LeftAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .exec(&db)
        .await
        .unwrap();
    assert!(usage_identity(&db, &bot(Some("group"))).await.is_err());
    bot::Entity::update_many()
        .col_expr(bot::Column::DeletedAt, sea_orm::sea_query::Expr::value(now))
        .exec(&db)
        .await
        .unwrap();
    assert!(usage_identity(&db, &bot(None)).await.is_err());
}

#[test]
fn direct_bot_app_tokens_cannot_read_or_write_inbox_conversations() {
    let bot = BotUser {
        turn_id: "direct:request".into(),
        ..bot(None)
    };
    for room in ["owner-chat", "group"] {
        assert_eq!(status(ensure_room(&bot, room)), Some(StatusCode::FORBIDDEN));
    }
    assert_eq!(
        status(ensure_turn(&bot, "bt_1")),
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(status(check(&bot, &Names::default())), None);
}
