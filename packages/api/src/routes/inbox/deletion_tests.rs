use super::{
    Reader,
    conversations::{NewConversation, start_direct, start_group},
    deletion::delete_in,
    entries::{PageBounds, entries_following, entries_page, send_in},
    fixture::{at, bot_principal, database, insert_bot, outgoing, text, user},
    members::add_in,
    pins::{pin_message_in, pins_for},
    positions::{PositionRead, entries_by_position},
    stars::{StarsQuery, star_message_in, stars_for},
    view::conversations_for,
};
use crate::entity::{conversation_member, inbox_entry};
use flow_like_types::inbox::{
    ConversationEncryption, ConversationKind, EntryCursor, InboxEntryKind,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde_json::json;

fn bounds() -> PageBounds {
    PageBounds {
        before: None,
        after: None,
        limit: 100,
    }
}

#[tokio::test]
async fn deleting_a_direct_chat_clears_only_its_reader_and_preserves_send_idempotency() {
    let db = database().await;
    let id = start_direct(
        &db,
        "user-a",
        &user("user-b"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    let old = send_in(&db, &id, "user-a", &text("old"), at(1))
        .await
        .unwrap();
    send_in(&db, &id, "user-b", &text("reply"), at(2))
        .await
        .unwrap();
    pin_message_in(&db, &id, "user-a", "old", at(3))
        .await
        .unwrap();
    star_message_in(&db, &id, "user-a", "old", at(3))
        .await
        .unwrap();
    delete_in(&db, &id, "user-a", at(4)).await.unwrap();
    assert!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .is_empty()
    );
    let details = conversations_for(&db, &Reader::own("user-a"), Some(&id))
        .await
        .unwrap();
    assert_eq!(details[0].history_cleared_at, Some(at(4)));
    assert!(details[0].last_entry.is_none());
    assert_eq!(details[0].unread_count, 0);
    assert!(
        entries_page(&db, "user-a", &id, &bounds())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        entries_page(&db, "user-b", &id, &bounds())
            .await
            .unwrap()
            .entries
            .len(),
        2
    );
    assert!(
        entries_by_position(
            &db,
            "user-a",
            &id,
            &PositionRead::Range {
                from: Some(1),
                to: None,
                limit: 100
            }
        )
        .await
        .unwrap()
        .entries
        .is_empty()
    );
    let cursor = EntryCursor::parse(&old.entry.cursor).unwrap();
    assert!(
        entries_following(&db, "user-a", &id, Some(&cursor), 100)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        pins_for(&db, &id, &Reader::own("user-a"))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        pins_for(&db, &id, &Reader::own("user-b"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        stars_for(&db, "user-a", &StarsQuery::default())
            .await
            .unwrap()
            .stars
            .is_empty()
    );
    let retry = send_in(&db, &id, "user-a", &text("old"), at(5))
        .await
        .unwrap();
    assert_eq!(retry.entry.seq, old.entry.seq);
    assert_eq!(retry.entry.envelope["body"]["deleted"], true);
    assert!(retry.wake.is_empty());
    assert!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .is_empty()
    );
    let edit = outgoing(
        "edit",
        InboxEntryKind::Edit,
        json!({"target":"reply", "text":"updated"}),
    );
    send_in(&db, &id, "user-b", &edit, at(6)).await.unwrap();
    assert!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .is_empty()
    );
    send_in(&db, &id, "user-b", &text("new"), at(7))
        .await
        .unwrap();
    let visible = conversations_for(&db, &Reader::own("user-a"), None)
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].last_entry.as_ref().unwrap().message_id, "new");
    assert_eq!(visible[0].unread_count, 1);
    assert_eq!(
        entries_page(&db, "user-a", &id, &bounds())
            .await
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.message_id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
}

#[tokio::test]
async fn explicit_reopen_keeps_legacy_and_positioned_history_cleared() {
    let db = database().await;
    let id = start_direct(
        &db,
        "user-a",
        &user("user-b"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    send_in(&db, &id, "user-a", &text("legacy"), at(1))
        .await
        .unwrap();
    inbox_entry::Entity::update_many()
        .col_expr(inbox_entry::Column::Seq, Expr::value(None::<i64>))
        .col_expr(inbox_entry::Column::Cleared, Expr::value(None::<bool>))
        .filter(inbox_entry::Column::MessageId.eq("legacy"))
        .exec(&db)
        .await
        .unwrap();
    delete_in(&db, &id, "user-a", at(2)).await.unwrap();
    let reopened = start_direct(
        &db,
        "user-a",
        &user("user-b"),
        ConversationEncryption::Plain,
        at(3),
    )
    .await
    .unwrap();
    assert_eq!(reopened, id);
    // System helpers ensuring a room exists do not reopen a deleted chat.
    assert!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .is_empty()
    );
    super::deletion::reopen(&db, &id, "user-a").await.unwrap();
    assert_eq!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        entries_page(&db, "user-a", &id, &bounds())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        entries_page(&db, "user-b", &id, &bounds())
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn deleting_group_leaves_with_owned_bots_and_rejoining_retains_cutoff() {
    let db = database().await;
    insert_bot(&db, "bot_a", "user-a", "DESKTOP").await;
    insert_bot(&db, "bot_b", "user-b", "DESKTOP").await;
    start_group(
        &db,
        "room",
        "user-a",
        &NewConversation {
            kind: ConversationKind::Group,
            title: None,
            members: vec![],
            encryption: ConversationEncryption::Plain,
        },
        &[
            user("user-b"),
            bot_principal("bot_a"),
            bot_principal("bot_b"),
        ],
        at(0),
    )
    .await
    .unwrap();
    send_in(&db, "room", "user-a", &text("old"), at(1))
        .await
        .unwrap();
    delete_in(&db, "room", "user-a", at(2)).await.unwrap();
    delete_in(&db, "room", "user-a", at(3)).await.unwrap();
    for (principal, left) in [
        ("user-a", true),
        ("bot_a", true),
        ("user-b", false),
        ("bot_b", false),
    ] {
        let member =
            conversation_member::Entity::find_by_id(("room".to_string(), principal.to_string()))
                .one(&db)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(member.left_at.is_some(), left, "{principal}");
    }
    add_in(&db, "room", "user-b", &[user("user-a")], at(4))
        .await
        .unwrap();
    assert_eq!(
        conversations_for(&db, &Reader::own("user-a"), None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        entries_page(&db, "user-a", "room", &bounds())
            .await
            .unwrap()
            .entries
            .iter()
            .all(|entry| entry.message_id != "old")
    );
    assert!(
        entries_page(&db, "user-b", "room", &bounds())
            .await
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.message_id == "old")
    );
    assert!(delete_in(&db, "room", "stranger", at(5)).await.is_err());
}

#[tokio::test]
async fn repeated_deletions_advance_cache_marker_even_with_the_same_clock_value() {
    let db = database().await;
    let id = start_direct(
        &db,
        "user-a",
        &user("user-b"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    send_in(&db, &id, "user-b", &text("first"), at(1))
        .await
        .unwrap();
    delete_in(&db, &id, "user-a", at(2)).await.unwrap();
    super::deletion::reopen(&db, &id, "user-a").await.unwrap();
    send_in(&db, &id, "user-b", &text("second"), at(2))
        .await
        .unwrap();
    delete_in(&db, &id, "user-a", at(2)).await.unwrap();
    let detail = conversations_for(&db, &Reader::own("user-a"), Some(&id))
        .await
        .unwrap();
    assert!(detail[0].history_cleared_at.unwrap() > at(2));
    assert!(
        entries_page(&db, "user-a", &id, &bounds())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn summaries_exclude_cleared_imports_and_deleted_ranges_for_the_reader() {
    let db = database().await;
    insert_bot(&db, "bot_a", "user-a", "DESKTOP").await;
    let id = start_direct(
        &db,
        "user-a",
        &bot_principal("bot_a"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    send_in(&db, &id, "user-a", &text("old"), at(1))
        .await
        .unwrap();
    let summary = super::segments::validate_segment(super::model::WriteSegmentRequest {
        id: "summary".into(),
        level: 0,
        seq_from: 1,
        seq_to: 1,
        title: None,
        summary: "Old message content".into(),
        parent_id: None,
    })
    .unwrap();
    super::segments::write_segment_in(&db, &id, "bot_a", &summary, at(2))
        .await
        .unwrap();
    let page = super::segments::SegmentPage {
        level: None,
        covering: None,
        after: None,
        limit: 100,
    };
    inbox_entry::Entity::update_many()
        .col_expr(inbox_entry::Column::Cleared, Expr::value(true))
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .filter(inbox_entry::Column::MessageId.eq("old"))
        .exec(&db)
        .await
        .unwrap();
    assert!(
        super::segments::segments_page_after(&db, &id, None, &page, None, Some("user-a"))
            .await
            .unwrap()
            .segments
            .is_empty()
    );
    assert_eq!(
        super::segments::segments_page_after(&db, &id, None, &page, None, Some("bot_a"))
            .await
            .unwrap()
            .segments
            .len(),
        1
    );
    delete_in(&db, &id, "user-a", at(3)).await.unwrap();
    assert!(
        super::segments::segments_page_after(&db, &id, None, &page, Some(1), Some("user-a"))
            .await
            .unwrap()
            .segments
            .is_empty()
    );
}
