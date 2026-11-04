use super::{
    INBOX_PIN_LIMIT, INBOX_TARGET_NOT_FOUND, Reader,
    conversations::{NewConversation, start_group},
    entries::send_in,
    fixture::assert_code,
    fixture::{at, bot_principal, database, insert_bot, outgoing, text, user},
    members::remove_in,
    pins::{MAX_PINNED_MESSAGES, pin_message_in, pins_for, set_pinned_in, unpin_message_in},
    stars::{StarsQuery, star_message_in, stars_for, unstar_message_in},
    view::conversations_for,
};
use crate::entity::pinned_message;
use axum::http::StatusCode;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind, InboxEntryKind};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use serde_json::json;

async fn room(db: &DatabaseConnection, id: &str, second: i64) {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(
        db,
        id,
        "user-a",
        &plan,
        &[user("user-b"), bot_principal("bot_1")],
        at(second),
    )
    .await
    .unwrap();
}

async fn world() -> DatabaseConnection {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    room(&db, "old", 1).await;
    room(&db, "mid", 2).await;
    room(&db, "new", 3).await;
    db
}

async fn listed(db: &DatabaseConnection, reader: &Reader) -> Vec<String> {
    conversations_for(db, reader, None)
        .await
        .unwrap()
        .into_iter()
        .map(|conversation| conversation.id)
        .collect()
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn as_bot_1() -> Reader {
    Reader {
        inbox: "bot_1".into(),
        owner: Some("user-a".into()),
    }
}

#[tokio::test]
async fn pinned_conversations_list_first_newest_pin_first_for_whoever_pinned_them() {
    let db = world().await;
    assert_eq!(
        listed(&db, &Reader::own("user-a")).await,
        ids(&["new", "mid", "old"])
    );

    let first = set_pinned_in(&db, "old", "user-a", true, at(10))
        .await
        .unwrap();
    assert_eq!(first.pinned_at, Some(at(10)));
    set_pinned_in(&db, "mid", "user-a", true, at(11))
        .await
        .unwrap();
    let again = set_pinned_in(&db, "old", "user-a", true, at(12))
        .await
        .unwrap();
    assert_eq!(
        again.pinned_at,
        Some(at(10)),
        "pinning again keeps the first pin"
    );

    assert_eq!(
        listed(&db, &Reader::own("user-a")).await,
        ids(&["mid", "old", "new"])
    );
    assert_eq!(
        listed(&db, &Reader::own("user-b")).await,
        ids(&["new", "mid", "old"])
    );
    assert_eq!(listed(&db, &as_bot_1()).await, ids(&["new", "mid", "old"]));
    let shown = conversations_for(&db, &Reader::own("user-a"), Some("old"))
        .await
        .unwrap();
    assert_eq!(shown[0].pinned_at, Some(at(10)));
    let as_bot = conversations_for(&db, &as_bot_1(), Some("old"))
        .await
        .unwrap();
    assert_eq!(as_bot[0].pinned_at, None);

    let off = set_pinned_in(&db, "mid", "user-a", false, at(13))
        .await
        .unwrap();
    assert_eq!(off.pinned_at, None);
    assert_eq!(
        listed(&db, &Reader::own("user-a")).await,
        ids(&["old", "new", "mid"])
    );
}

#[tokio::test]
async fn only_active_members_pin_and_star() {
    let db = world().await;
    send_in(&db, "new", "user-a", &text("m1"), at(20))
        .await
        .unwrap();
    for error in [
        set_pinned_in(&db, "new", "user-c", true, at(21))
            .await
            .unwrap_err(),
        pin_message_in(&db, "new", "user-c", "m1", at(21))
            .await
            .unwrap_err(),
        star_message_in(&db, "new", "user-c", "m1", at(21))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }
    remove_in(&db, "new", "user-b", "user-b", at(22))
        .await
        .unwrap();
    let left = pin_message_in(&db, "new", "user-b", "m1", at(23))
        .await
        .unwrap_err();
    assert_eq!(left.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn message_pins_are_shared_show_the_newest_edit_and_stop_at_the_cap() {
    let db = world().await;
    send_in(&db, "new", "user-a", &text("m1"), at(20))
        .await
        .unwrap();
    let pin = pin_message_in(&db, "new", "user-b", "m1", at(21))
        .await
        .unwrap();
    assert_eq!(pin.pinned_by, "user-b");
    assert_eq!(pin.entry.as_ref().unwrap().message_id, "m1");
    let again = pin_message_in(&db, "new", "user-a", "m1", at(22))
        .await
        .unwrap();
    assert_eq!(
        (again.pinned_by.as_str(), again.pinned_at),
        ("user-b", at(21))
    );

    let edit = outgoing(
        "e1",
        InboxEntryKind::Edit,
        json!({ "target": "m1", "text": "fixed" }),
    );
    send_in(&db, "new", "user-a", &edit, at(23)).await.unwrap();
    let seen = pins_for(&db, "new", &Reader::own("user-a")).await.unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].latest_edit.as_ref().unwrap().envelope["body"]["text"],
        "fixed"
    );
    let by_bot = pins_for(&db, "new", &as_bot_1()).await.unwrap();
    assert_eq!(by_bot[0].pinned_by, "user-b");

    let missing = pin_message_in(&db, "new", "user-a", "nope", at(24))
        .await
        .unwrap_err();
    assert_code(
        &missing,
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
    );
    let malformed = pin_message_in(&db, "new", "user-a", "no spaces", at(24))
        .await
        .unwrap_err();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

    for n in 1..MAX_PINNED_MESSAGES {
        pinned_message::ActiveModel {
            conversation_id: Set("new".into()),
            message_id: Set(format!("filler-{n}")),
            pinned_by: Set("user-a".into()),
            pinned_at: Set(at(30)),
        }
        .insert(&db)
        .await
        .unwrap();
    }
    send_in(&db, "new", "user-a", &text("m2"), at(31))
        .await
        .unwrap();
    let full = pin_message_in(&db, "new", "user-a", "m2", at(32))
        .await
        .unwrap_err();
    assert_code(&full, StatusCode::CONFLICT, INBOX_PIN_LIMIT);

    assert!(unpin_message_in(&db, "new", "user-a", "m1").await.unwrap());
    assert!(!unpin_message_in(&db, "new", "user-a", "m1").await.unwrap());
    pin_message_in(&db, "new", "user-a", "m2", at(33))
        .await
        .unwrap();
}

#[tokio::test]
async fn stars_are_personal_newest_first_and_page_back_with_before() {
    let db = world().await;
    for (n, conversation) in [(1, "old"), (2, "new"), (3, "new")] {
        let id = format!("m{n}");
        send_in(&db, conversation, "user-a", &text(&id), at(20 + n))
            .await
            .unwrap();
        let star = star_message_in(&db, conversation, "user-b", &id, at(30 + n))
            .await
            .unwrap();
        assert_eq!(star.entry.message_id, id);
    }
    let again = star_message_in(&db, "new", "user-b", "m3", at(40))
        .await
        .unwrap();
    assert_eq!(
        again.starred_at,
        at(33),
        "starring again keeps the first star"
    );

    let none = stars_for(&db, "user-a", &StarsQuery::default())
        .await
        .unwrap();
    assert!(none.stars.is_empty());

    let query = |before: Option<String>| StarsQuery {
        conversation_id: None,
        before,
        limit: Some(2),
    };
    let first = stars_for(&db, "user-b", &query(None)).await.unwrap();
    let first_ids: Vec<&str> = first
        .stars
        .iter()
        .map(|star| star.message_id.as_str())
        .collect();
    assert_eq!(first_ids, ["m3", "m2"]);
    let rest = stars_for(&db, "user-b", &query(first.next_before.clone()))
        .await
        .unwrap();
    assert_eq!(rest.stars.len(), 1);
    assert_eq!(rest.stars[0].message_id, "m1");
    assert_eq!(rest.next_before, None);

    let in_new = stars_for(
        &db,
        "user-b",
        &StarsQuery {
            conversation_id: Some("new".into()),
            ..StarsQuery::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(in_new.stars.len(), 2);

    assert!(unstar_message_in(&db, "new", "user-b", "m3").await.unwrap());
    assert!(!unstar_message_in(&db, "new", "user-b", "m3").await.unwrap());

    remove_in(&db, "new", "user-b", "user-b", at(50))
        .await
        .unwrap();
    let after_leaving = stars_for(&db, "user-b", &StarsQuery::default())
        .await
        .unwrap();
    let kept: Vec<&str> = after_leaving
        .stars
        .iter()
        .map(|star| star.message_id.as_str())
        .collect();
    assert_eq!(kept, ["m1"]);
    let gone = stars_for(
        &db,
        "user-b",
        &StarsQuery {
            conversation_id: Some("new".into()),
            ..StarsQuery::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_message_takes_it_off_pins_and_stars() {
    let db = world().await;
    send_in(&db, "new", "user-a", &text("m1"), at(20))
        .await
        .unwrap();
    pin_message_in(&db, "new", "user-b", "m1", at(21))
        .await
        .unwrap();
    star_message_in(&db, "new", "user-b", "m1", at(22))
        .await
        .unwrap();
    star_message_in(&db, "new", "user-a", "m1", at(22))
        .await
        .unwrap();

    let delete = outgoing("d1", InboxEntryKind::Delete, json!({ "target": "m1" }));
    send_in(&db, "new", "user-a", &delete, at(23))
        .await
        .unwrap();

    assert!(
        pins_for(&db, "new", &Reader::own("user-b"))
            .await
            .unwrap()
            .is_empty()
    );
    for reader in ["user-a", "user-b"] {
        let stars = stars_for(&db, reader, &StarsQuery::default())
            .await
            .unwrap();
        assert!(stars.stars.is_empty(), "{reader} still has a star");
    }
    let deleted = star_message_in(&db, "new", "user-b", "m1", at(24))
        .await
        .unwrap_err();
    assert_code(
        &deleted,
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
    );
}
