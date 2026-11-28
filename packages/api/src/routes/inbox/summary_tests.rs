use super::{
    Reader,
    deletion::delete_in,
    entries::{mark_read_in, send_in},
    fixture::{at, database, group, outgoing, text, user},
    members::add_in,
    stored,
    summary::{newest_sql, unread_sql},
    view::conversations_for,
};
use crate::entity::{conversation_member, inbox_entry};
use flow_like_types::inbox::{ConversationEncryption, InboxEntryKind};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Statement,
    sea_query::Expr,
};
use serde_json::json;

/// Every summarized membership against a live count of its principal's copy.
async fn assert_summaries_match(db: &DatabaseConnection) {
    let sql = format!(
        "SELECT {} AS unread, {} AS newest",
        unread_sql("$1", "$2", "$3", "$4"),
        newest_sql("$1", "$2", "$3")
    );
    let members = conversation_member::Entity::find().all(db).await.unwrap();
    for member in members
        .iter()
        .filter(|member| member.unread_count.is_some())
    {
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                sql.clone(),
                [
                    member.principal_id.as_str().into(),
                    member.conversation_id.as_str().into(),
                    stored(InboxEntryKind::Message.as_str()).into(),
                    member.last_read_at.into(),
                ],
            ))
            .await
            .unwrap()
            .unwrap();
        let unread: i64 = row.try_get("", "unread").unwrap();
        let newest: Option<String> = row.try_get("", "newest").unwrap();
        assert_eq!(
            (
                member.unread_count.map(i64::from),
                member.last_entry_id.clone()
            ),
            (Some(unread), newest),
            "{} in {}",
            member.principal_id,
            member.conversation_id
        );
    }
}

async fn member(db: &DatabaseConnection, principal: &str) -> conversation_member::Model {
    conversation_member::Entity::find_by_id(("room".to_string(), principal.to_string()))
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

/// What `reader`'s list shows for the room: unread count and newest message id.
async fn listed(db: &DatabaseConnection, reader: &str) -> (u64, Option<String>) {
    let listed = conversations_for(db, &Reader::own(reader), Some("room"))
        .await
        .unwrap();
    let room = &listed[0];
    (
        room.unread_count,
        room.last_entry
            .as_ref()
            .map(|entry| entry.message_id.clone()),
    )
}

async fn room(db: &DatabaseConnection) {
    let others = [user("user-b"), user("user-c")];
    group(db, "room", "user-a", &others, ConversationEncryption::Plain).await;
}

#[tokio::test]
async fn summaries_follow_sends_reads_clears_and_rejoins() {
    let db = database().await;
    room(&db).await;
    assert_eq!(member(&db, "user-a").await.unread_count, None);

    send_in(&db, "room", "user-b", &text("b_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "room", "user-c", &text("c_1"), at(11))
        .await
        .unwrap();
    send_in(&db, "room", "user-a", &text("a_1"), at(12))
        .await
        .unwrap();
    let reaction = json!({ "target": "a_1", "emoji": "🎉", "op": "add" });
    send_in(
        &db,
        "room",
        "user-c",
        &outgoing("c_react", InboxEntryKind::Reaction, reaction),
        at(13),
    )
    .await
    .unwrap();
    mark_read_in(&db, "room", "user-b", None, at(11))
        .await
        .unwrap();
    assert_summaries_match(&db).await;
    assert_eq!(listed(&db, "user-a").await, (2, Some("a_1".into())));
    assert_eq!(listed(&db, "user-b").await, (1, Some("a_1".into())));
    assert_eq!(listed(&db, "user-c").await, (2, Some("a_1".into())));

    delete_in(&db, "room", "user-c", at(14)).await.unwrap();
    assert_eq!(
        (
            member(&db, "user-c").await.unread_count,
            member(&db, "user-c").await.last_entry_id
        ),
        (Some(0), None)
    );
    send_in(&db, "room", "user-b", &text("b_2"), at(15))
        .await
        .unwrap();
    add_in(&db, "room", "user-a", &[user("user-c")], at(16))
        .await
        .unwrap();
    assert_eq!(member(&db, "user-c").await.unread_count, None);
    assert_eq!(listed(&db, "user-c").await, (0, None));

    send_in(&db, "room", "user-b", &text("b_3"), at(17))
        .await
        .unwrap();
    mark_read_in(&db, "room", "user-a", None, at(15))
        .await
        .unwrap();
    assert_summaries_match(&db).await;
    assert_eq!(listed(&db, "user-a").await, (1, Some("b_3".into())));
    assert_eq!(listed(&db, "user-b").await, (1, Some("b_3".into())));
    assert_eq!(listed(&db, "user-c").await, (1, Some("b_3".into())));
}

#[tokio::test]
async fn the_list_reads_stored_summaries_and_counts_unsummarized_rows_live() {
    let db = database().await;
    room(&db).await;
    send_in(&db, "room", "user-b", &text("b_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "room", "user-b", &text("b_2"), at(11))
        .await
        .unwrap();
    let first = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq("user-a"))
        .filter(inbox_entry::Column::MessageId.eq("b_1"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let set = |count: Option<i32>, entry: Option<String>| {
        conversation_member::Entity::update_many()
            .col_expr(conversation_member::Column::UnreadCount, Expr::value(count))
            .col_expr(conversation_member::Column::LastEntryId, Expr::value(entry))
            .filter(conversation_member::Column::PrincipalId.eq("user-a"))
            .exec(&db)
    };

    set(Some(42), Some(first.id.clone())).await.unwrap();
    assert_eq!(listed(&db, "user-a").await, (42, Some("b_1".into())));

    set(None, Some(first.id)).await.unwrap();
    assert_eq!(listed(&db, "user-a").await, (2, Some("b_2".into())));
}
