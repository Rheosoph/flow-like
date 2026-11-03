use super::{
    fixture::{at, copies, database, insert_bot},
    mail_threads::{
        Direction, holds_message, link_message_in, normalize_message_id, start_thread_in,
        thread_of_any, touch_thread_in, write_outbound_in, write_trigger_in,
    },
};
use crate::{
    entity::{bot_mail_message, bot_mail_thread, bot_turn, conversation_member},
    routes::bots::mail::mail_item,
};
use flow_like_types::inbox::{TriggerBody, TriggerSource};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Schema,
    sea_query::Table,
};
use serde_json::json;

async fn setup() -> DatabaseConnection {
    let db = database().await;
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(bot_mail_thread::Entity),
        schema.create_table_from_entity(bot_mail_message::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    db.execute_unprepared(
        r#"CREATE UNIQUE INDEX public."BotMailMessage_botId_messageId_key" ON "BotMailMessage"("botId", "messageId")"#,
    )
    .await
    .unwrap();
    insert_bot(&db, "bot_m", "user-a", "SERVER").await;
    db
}

fn trigger(id: &str) -> TriggerBody {
    TriggerBody {
        source: TriggerSource::Email,
        ref_id: id.into(),
        title: "Invoice".into(),
        text: "Untrusted email".into(),
        payload: Some(json!({
            "subject": "Invoice", "text": "hello", "authenticated": true,
            "from": [{"email": "ann@example.com"}], "to": [], "cc": [],
            "attachments": [{"name": "a.pdf", "content_type": "application/pdf", "size": 3}],
        })),
        payload_handle: None,
        bot_id: Some("bot_m".into()),
    }
}

async fn thread(db: &DatabaseConnection) -> String {
    start_thread_in(
        db,
        "user-a",
        "bot_m",
        "Invoice",
        &["ann@example.com".to_string()],
        None,
        at(1),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_thread_is_a_group_of_the_owner_and_the_bot_marked_as_mail() {
    let db = setup().await;
    let id = thread(&db).await;
    let members = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(id.as_str()))
        .all(&db)
        .await
        .unwrap();
    let mut ids: Vec<_> = members.iter().map(|m| m.principal_id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["bot_m", "user-a"]);
    let row = bot_mail_thread::Entity::find_by_id(id.as_str())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.bot_id.as_str(), row.owner_id.as_str()),
        ("bot_m", "user-a")
    );
    assert_eq!(row.subject, "Invoice");
}

#[tokio::test]
async fn an_inbound_mail_is_a_trigger_from_the_owner_that_queues_the_bots_turn() {
    let db = setup().await;
    let id = thread(&db).await;
    let sent = write_trigger_in(&db, &id, "user-a", "mail-1", &trigger("m1"), at(2))
        .await
        .unwrap();
    assert_eq!(sent.entry.kind, "trigger");
    assert_eq!(sent.entry.sender_id, "user-a");
    assert_eq!(sent.bots.len(), 1);
    let turns = bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq("bot_m"))
        .filter(bot_turn::Column::TriggerMessageId.eq("mail-1"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].hops, 0);
    assert_eq!(turns[0].conversation_id, id);
    assert_eq!(copies(&db, "mail-1").await.len(), 2);
}

#[tokio::test]
async fn a_repeated_delivery_writes_the_trigger_once() {
    let db = setup().await;
    let id = thread(&db).await;
    write_trigger_in(&db, &id, "user-a", "mail-1", &trigger("m1"), at(2))
        .await
        .unwrap();
    let again = write_trigger_in(&db, &id, "user-a", "mail-1", &trigger("m1"), at(3))
        .await
        .unwrap();
    assert!(again.bots.is_empty());
    assert_eq!(copies(&db, "mail-1").await.len(), 2);
}

#[tokio::test]
async fn later_mails_find_their_thread_by_message_id_headers() {
    let db = setup().await;
    let id = thread(&db).await;
    assert!(
        link_message_in(
            &db,
            &id,
            "bot_m",
            "root@example.com",
            Direction::In,
            "mail-1",
            at(2)
        )
        .await
        .unwrap()
    );
    let found = thread_of_any(
        &db,
        "bot_m",
        &["other@example.com".into(), "root@example.com".into()],
    )
    .await
    .unwrap();
    assert_eq!(found.as_deref(), Some(id.as_str()));
    assert_eq!(
        thread_of_any(&db, "bot_m", &["other@example.com".into()])
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        thread_of_any(&db, "bot_other", &["root@example.com".into()])
            .await
            .unwrap(),
        None,
        "another bot's mail never joins this thread"
    );
    assert_eq!(thread_of_any(&db, "bot_m", &[]).await.unwrap(), None);
}

#[tokio::test]
async fn the_same_message_id_is_accepted_once() {
    let db = setup().await;
    let id = thread(&db).await;
    assert!(
        link_message_in(
            &db,
            &id,
            "bot_m",
            "dup@example.com",
            Direction::In,
            "mail-1",
            at(2)
        )
        .await
        .unwrap()
    );
    assert!(
        !link_message_in(
            &db,
            &id,
            "bot_m",
            "dup@example.com",
            Direction::In,
            "mail-2",
            at(3)
        )
        .await
        .unwrap()
    );
    assert!(
        holds_message(&db, "bot_m", "dup@example.com")
            .await
            .unwrap()
    );
    assert!(
        !holds_message(&db, "bot_m", "new@example.com")
            .await
            .unwrap()
    );
}

#[test]
fn message_ids_are_normalized_and_hostile_ones_refused() {
    assert_eq!(
        normalize_message_id(" <Abc@Example.COM> ").as_deref(),
        Some("abc@example.com")
    );
    assert_eq!(normalize_message_id("<>"), None);
    assert_eq!(normalize_message_id("a b"), None);
    assert_eq!(normalize_message_id(&"x".repeat(600)), None);
}

#[tokio::test]
async fn a_sent_mail_is_recorded_for_the_owner_and_wakes_no_turn() {
    let db = setup().await;
    let id = thread(&db).await;
    let sent = write_outbound_in(
        &db,
        &id,
        "bot_m",
        "mailout-1",
        "Thanks, paid.",
        json!({"direction": "out", "from": "bot@flow-like.me", "to": ["ann@example.com"],
               "cc": [], "subject": "Re: Invoice", "attachments": []}),
        at(4),
    )
    .await
    .unwrap();
    assert_eq!(sent.entry.sender_id, "bot_m");
    assert!(sent.bots.is_empty(), "the sender never wakes itself");
    let owner_copy = copies(&db, "mailout-1")
        .await
        .into_iter()
        .find(|entry| entry.inbox_id == "user-a")
        .unwrap();
    let item = mail_item(&owner_copy).unwrap();
    assert_eq!(item.direction, "out");
    assert_eq!(item.text, "Thanks, paid.");
    assert_eq!(item.subject, "Re: Invoice");
    assert!(item.authenticated);
}

#[tokio::test]
async fn the_thread_view_reads_inbound_triggers_and_ignores_other_chatter() {
    let db = setup().await;
    let id = thread(&db).await;
    write_trigger_in(&db, &id, "user-a", "mail-1", &trigger("m1"), at(2))
        .await
        .unwrap();
    let owner_copy = copies(&db, "mail-1")
        .await
        .into_iter()
        .find(|entry| entry.inbox_id == "user-a")
        .unwrap();
    let item = mail_item(&owner_copy).unwrap();
    assert_eq!(item.direction, "in");
    assert_eq!(item.subject, "Invoice");
    assert!(item.authenticated);
    assert_eq!(item.attachments.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn touching_a_thread_counts_mails_and_merges_correspondents_once() {
    let db = setup().await;
    let id = thread(&db).await;
    touch_thread_in(
        &db,
        &id,
        &["Ann@Example.com".into(), "bob@example.com".into()],
        "line one\n  line two",
        at(5),
    )
    .await
    .unwrap();
    let row = bot_mail_thread::Entity::find_by_id(id.as_str())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.message_count, 1);
    let people: Vec<String> = serde_json::from_str(&row.participants).unwrap();
    assert_eq!(people, ["ann@example.com", "bob@example.com"]);
    assert_eq!(row.last_snippet.as_deref(), Some("line one line two"));
}
