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
