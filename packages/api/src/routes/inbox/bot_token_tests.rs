use super::{
    acting_reader,
    conversations::{NewConversation, start_group},
    entries::{SendAs, ensure_bot_sends_own_turn},
    fixture::{at, bot_principal, database, delete_bot, insert_bot, user},
    model::TurnClaim,
};
use crate::{
    bot_caller::{self, Names},
    middleware::jwt::{AppUser, BotUser, OpenIDUser},
};
use axum::http::StatusCode;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind};
use sea_orm::DatabaseConnection;

fn token(bot_id: &str, turn_id: &str, room: Option<&str>) -> AppUser {
    AppUser::Bot(BotUser {
        bot_id: bot_id.into(),
        turn_id: turn_id.into(),
        conv_id: room.map(str::to_owned),
    })
}

fn person(sub: &str) -> AppUser {
    AppUser::OpenID(OpenIDUser {
        sub: sub.into(),
        access_token: String::new(),
    })
}

async fn team() -> DatabaseConnection {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "DESKTOP").await;
    insert_bot(&db, "bot_2", "user-b", "DESKTOP").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some("team".into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    let others = [
        user("user-b"),
        bot_principal("bot_1"),
        bot_principal("bot_2"),
    ];
    start_group(&db, "TEAM", "user-a", &plan, &others, at(0))
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn a_bot_token_reads_its_own_inbox_and_resolves_its_owner_server_side() {
    let db = team().await;
    for as_bot in [None, Some("bot_1")] {
        let reader = acting_reader(&db, &token("bot_1", "bt_1", None), as_bot, "TEAM")
            .await
            .unwrap();
        assert_eq!(reader.inbox, "bot_1");
        assert_eq!(reader.owner.as_deref(), Some("user-a"));
    }
}

#[tokio::test]
async fn a_bot_token_refuses_a_foreign_bot_or_a_person_as() {
    let db = team().await;
    for as_bot in ["bot_2", "user-a", "user-b"] {
        let error = acting_reader(&db, &token("bot_1", "bt_1", None), Some(as_bot), "TEAM")
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN, "as {as_bot}");
    }
}

#[tokio::test]
async fn a_room_token_refuses_another_conversation() {
    let db = team().await;
    let room = token("bot_1", "bt_1", Some("TEAM"));
    assert!(acting_reader(&db, &room, None, "team").await.is_ok());
    let error = acting_reader(&db, &room, None, "OTHER").await.unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_deleted_bot_token_is_refused() {
    let db = team().await;
    delete_bot(&db, "bot_1").await;
    let error = acting_reader(&db, &token("bot_1", "bt_1", None), None, "TEAM")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_person_still_reads_only_their_own_bots() {
    let db = team().await;
    let own = acting_reader(&db, &person("user-a"), Some("bot_1"), "TEAM")
        .await
        .unwrap();
    assert_eq!(
        (own.inbox.as_str(), own.owner.as_deref()),
        ("bot_1", Some("user-a"))
    );
    let error = acting_reader(&db, &person("user-a"), Some("bot_2"), "TEAM")
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn acting_owner_is_the_owner_for_the_tokens_bot_only() {
    let db = team().await;
    let names = |turn| Names {
        as_bot: Some("bot_1"),
        conversation: Some("TEAM"),
        turn,
    };
    let bot = token("bot_1", "bt_1", Some("TEAM"));
    assert_eq!(
        bot_caller::acting_owner(&db, &bot, &names(Some("bt_1")))
            .await
            .unwrap(),
        "user-a"
    );
    let other_turn = bot_caller::acting_owner(&db, &bot, &names(Some("bt_2"))).await;
    assert_eq!(other_turn.unwrap_err().status(), StatusCode::FORBIDDEN);
    let foreign = Names {
        as_bot: Some("bot_2"),
        ..names(None)
    };
    assert_eq!(
        bot_caller::acting_owner(&db, &bot, &foreign)
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        bot_caller::acting_owner(&db, &person("user-b"), &names(None))
            .await
            .unwrap(),
        "user-b"
    );
}

fn claim(turn: &str) -> TurnClaim {
    TurnClaim {
        id: turn.into(),
        host_id: "host".into(),
    }
}

#[test]
fn a_bot_token_sends_only_as_itself_for_its_own_turn() {
    let bot = token("bot_1", "bt_1", None);
    let own = SendAs::Bot {
        bot_id: "bot_1".into(),
        claim: claim("bt_1"),
    };
    assert!(ensure_bot_sends_own_turn(&bot, &own).is_ok());
    let other_turn = SendAs::Bot {
        bot_id: "bot_1".into(),
        claim: claim("bt_2"),
    };
    let foreign = SendAs::Bot {
        bot_id: "bot_2".into(),
        claim: claim("bt_1"),
    };
    for refused in [&other_turn, &foreign, &SendAs::Person] {
        let error = ensure_bot_sends_own_turn(&bot, refused).unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }
    assert!(ensure_bot_sends_own_turn(&person("user-a"), &SendAs::Person).is_ok());
}
