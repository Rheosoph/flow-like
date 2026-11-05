use super::*;
use crate::{
    entity::bot_turn,
    routes::inbox::{
        exchange::post_as_bot_in,
        fixture::{at, copies, database as inbox_database, delete_bot, insert_bot},
    },
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, PaginatorTrait};
use serde_json::json;
use std::sync::Mutex;

#[derive(Default)]
struct RecordingSink {
    sent: Mutex<Vec<(String, DispatchNotificationInput)>>,
    db: Mutex<Option<DatabaseConnection>>,
}

#[async_trait]
impl NoticeSink for RecordingSink {
    async fn send(&self, id: &str, input: DispatchNotificationInput) -> Result<()> {
        self.sent
            .lock()
            .unwrap()
            .push((id.to_string(), input.clone()));
        let db = self.db.lock().unwrap().clone().expect("db");
        notification::ActiveModel {
            id: Set(id.to_string()),
            user_id: Set(input.user_id),
            app_id: Set(None),
            title: Set(input.title),
            description: Set(input.description),
            icon: Set(None),
            link: Set(input.link),
            r#type: Set(input.notification_type),
            read: Set(false),
            source_run_id: Set(input.source_run_id),
            source_node_id: Set(input.source_node_id),
            created_at: Set(chrono::Utc::now().fixed_offset()),
            read_at: Set(None),
        }
        .insert(&db)
        .await?;
        Ok(())
    }
}

impl RecordingSink {
    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

async fn setup() -> (DatabaseConnection, RecordingSink) {
    let db = inbox_database().await;
    db.execute_unprepared(
        r#"CREATE TABLE public."Notification" (
            "id" TEXT PRIMARY KEY, "userId" TEXT NOT NULL, "appId" TEXT, "title" TEXT NOT NULL,
            "description" TEXT, "icon" TEXT, "link" TEXT, "read" INTEGER NOT NULL,
            "sourceRunId" TEXT, "sourceNodeId" TEXT, "createdAt" TEXT NOT NULL, "readAt" TEXT,
            "type" TEXT NOT NULL)"#,
    )
    .await
    .unwrap();
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    let sink = RecordingSink::default();
    *sink.db.lock().unwrap() = Some(db.clone());
    (db, sink)
}

async fn turn(db: &DatabaseConnection, id: &str, trigger_kind: &str, hops: i32) {
    let trigger = format!("trigger-{id}");
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."InboxEntry"
        ("id","inboxId","conversationId","messageId","senderId","kind","envelope","createdAt","seq")
        VALUES ('e-{id}','user-a','c1','{trigger}','user-a','{trigger_kind}','{{}}','2023-11-14 22:13:20+00:00',(SELECT COUNT(*) + 1 FROM public."InboxEntry"))"#
    ))
    .await
    .unwrap();
    bot_turn::ActiveModel {
        id: Set(id.into()),
        bot_id: Set("bot_1".into()),
        owner_id: Set("user-a".into()),
        conversation_id: Set("c1".into()),
        trigger_message_id: Set(trigger),
        host_kind: Set("SERVER".into()),
        status: Set("DONE".into()),
        attempts: Set(1),
        host_id: Set(None),
        lease_until: Set(None),
        operation_id: Set(None),
        reply_message_id: Set(None),
        error_code: Set(None),
        hops: Set(hops),
        requester_id: Set(None),
        audience: Set("OWNER".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
    }
    .insert(db)
    .await
    .unwrap();
}

fn reply<'a>(turn_id: &'a str, message_id: &'a str, text: &'a str) -> Reply<'a> {
    Reply {
        turn_id,
        bot_id: "bot_1",
        message_id,
        text,
        resumed: false,
    }
}

async fn mute(db: &DatabaseConnection) {
    db.execute_unprepared(
        r#"UPDATE "Bot" SET "appearance" = '{"kind":"pip","mute_notifications":true}' WHERE "id" = 'bot_1'"#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_reply_to_a_person_message_is_asked_for_and_does_not_notify() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "MESSAGE", 0).await;
    let sent = notify_for_reply(&db, &sink, &reply("t1", "re-1", "Done"))
        .await
        .unwrap();
    assert!(!sent);
    assert_eq!(sink.count(), 0);
}

#[tokio::test]
async fn a_reply_to_a_trigger_notifies_under_the_bot_and_opens_its_chat() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    let sent = notify_for_reply(&db, &sink, &reply("t1", "re-1", "Weekly report\nis ready"))
        .await
        .unwrap();
    assert!(sent);
    let (id, input) = sink.sent.lock().unwrap()[0].clone();
    assert!(id.starts_with("bn-"));
    assert_eq!(input.user_id, "user-a");
    assert_eq!(input.title, "bot_1");
    assert_eq!(input.description.as_deref(), Some("Weekly report is ready"));
    assert_eq!(input.link.as_deref(), Some("/bots?bot=bot_1"));
    assert_eq!(input.source_run_id.as_deref(), Some("bot:bot_1"));
    assert_eq!(input.source_node_id.as_deref(), Some("re-1"));
    assert_eq!(input.app_id, None);
}

#[tokio::test]
async fn a_reply_to_another_bots_message_notifies() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "MESSAGE", 1).await;
    assert!(
        notify_for_reply(&db, &sink, &reply("t1", "re-1", "Result"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn the_same_reply_notifies_once() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    assert!(
        notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
            .await
            .unwrap()
    );
    assert!(
        !notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
            .await
            .unwrap()
    );
    assert_eq!(sink.count(), 1);
    turn(&db, "t2", "TRIGGER", 0).await;
    assert!(
        notify_for_reply(&db, &sink, &reply("t2", "re-2", "Hi"))
            .await
            .unwrap()
    );
    assert_eq!(sink.count(), 2);
}

#[tokio::test]
async fn a_muted_or_deleted_bot_stays_quiet() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    mute(&db).await;
    assert!(
        !notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
            .await
            .unwrap()
    );
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    delete_bot(&db, "bot_1").await;
    assert!(
        !notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
            .await
            .unwrap()
    );
    assert_eq!(sink.count(), 0);
}

#[tokio::test]
async fn failure_reports_and_foreign_turns_do_not_notify() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    assert!(
        !notify_for_reply(&db, &sink, &reply("t1", "rf-1", "I couldn't"))
            .await
            .unwrap()
    );
    let other = Reply {
        bot_id: "bot_2",
        ..reply("t1", "re-1", "Hi")
    };
    assert!(!notify_for_reply(&db, &sink, &other).await.unwrap());
    assert_eq!(sink.count(), 0);
}

#[tokio::test]
async fn an_empty_reply_still_has_a_body() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    notify_for_reply(&db, &sink, &reply("t1", "re-1", "  "))
        .await
        .unwrap();
    let input = sink.sent.lock().unwrap()[0].1.clone();
    assert_eq!(input.description.as_deref(), Some(NO_PREVIEW));
}

#[test]
fn the_text_comes_from_a_plain_envelope_only() {
    let plain = json!({ "v": 1, "enc": "none", "body": { "text": "Hello" } });
    assert_eq!(text_of(Some(&plain)), "Hello");
    assert_eq!(
        text_of(Some(&json!({ "v": 1, "enc": "x25519", "ct": "…" }))),
        ""
    );
    assert_eq!(text_of(None), "");
}

struct DbChat<'a>(&'a DatabaseConnection);

#[async_trait]
impl OwnerChat for DbChat<'_> {
    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let outgoing = message_outgoing(message_id, json!({ "text": text, "hops": 0 }))
            .map_err(|error| anyhow!("{error}"))?;
        post_as_bot_in(self.0, conversation_id, bot_id, 0, &outgoing, now_millis())
            .await
            .map(|_| ())
            .map_err(|error| anyhow!("{error}"))
    }
}

fn notice(title: Option<&str>, text: &str) -> OwnerNotice {
    OwnerNotice {
        title: title.map(str::to_string),
        text: text.to_string(),
    }
}

async fn queued_turns(db: &DatabaseConnection) -> u64 {
    bot_turn::Entity::find().count(db).await.unwrap()
}

async fn owner_chat(db: &DatabaseConnection) -> String {
    open_conversation(db, "user-a", "bot_1", "", now_millis())
        .await
        .unwrap()
        .expect("plain chat")
}

#[tokio::test]
async fn the_owner_message_is_posted_as_the_bot_pushed_once_and_wakes_nobody() {
    let (db, sink) = setup().await;
    let receipt = deliver_owner_message(
        &db,
        &sink,
        &DbChat(&db),
        "bot_1",
        Some(&TurnSite::Conversation("mail-thread".into())),
        notice(Some("Heads up"), "Mail from Ada about the invoice"),
    )
    .await
    .unwrap();
    assert!(receipt.posted && !receipt.muted && !receipt.in_owner_chat);
    assert!(receipt.id.starts_with(OWNER_MESSAGE_PREFIX));

    let copies = copies(&db, &receipt.id).await;
    assert!(!copies.is_empty());
    assert!(copies.iter().all(|copy| copy.sender_id == "bot_1"));
    assert_eq!(copies[0].conversation_id, owner_chat(&db).await);
    assert_eq!(
        text_of(Some(&copies[0].envelope)),
        "Heads up\n\nMail from Ada about the invoice"
    );
    assert_eq!(queued_turns(&db).await, 0);

    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.title, "bot_1");
    assert_eq!(
        sent[0].1.description.as_deref(),
        Some("Heads up: Mail from Ada about the invoice")
    );
    assert_eq!(sent[0].1.link.as_deref(), Some("/bots?bot=bot_1"));
    assert_eq!(sent[0].1.source_run_id.as_deref(), Some("bot:bot_1"));
}

#[tokio::test]
async fn a_turn_in_the_owner_chat_is_refused_before_anything_is_written() {
    let (db, sink) = setup().await;
    let chat = owner_chat(&db).await;
    let receipt = deliver_owner_message(
        &db,
        &sink,
        &DbChat(&db),
        "bot_1",
        Some(&TurnSite::Conversation(chat)),
        notice(None, "Hello"),
    )
    .await
    .unwrap();
    assert!(receipt.in_owner_chat && !receipt.posted);
    assert_eq!(sink.count(), 0);
    let hour_ago = chrono::Utc::now().timestamp_millis() - 3_600_000;
    assert_eq!(
        count_tool_messages(&db, "bot_1", hour_ago).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn a_chat_the_owner_reads_live_is_refused_before_anything_is_written() {
    let (db, sink) = setup().await;
    let receipt = deliver_owner_message(
        &db,
        &sink,
        &DbChat(&db),
        "bot_1",
        Some(&TurnSite::OwnerLive),
        notice(None, "Hello"),
    )
    .await
    .unwrap();
    assert!(receipt.in_owner_chat && !receipt.posted);
    assert_eq!(sink.count(), 0);
    let hour_ago = chrono::Utc::now().timestamp_millis() - 3_600_000;
    assert_eq!(
        count_tool_messages(&db, "bot_1", hour_ago).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn a_muted_bot_still_posts_but_sends_no_push_and_counts_toward_the_limit() {
    let (db, sink) = setup().await;
    mute(&db).await;
    let receipt = deliver_owner_message(
        &db,
        &sink,
        &DbChat(&db),
        "bot_1",
        None,
        notice(None, "Quiet"),
    )
    .await
    .unwrap();
    assert!(receipt.muted && receipt.posted);
    assert_eq!(sink.count(), 0);
    let hour_ago = chrono::Utc::now().timestamp_millis() - 3_600_000;
    assert_eq!(
        count_tool_messages(&db, "bot_1", hour_ago).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn messages_are_counted_once_per_bot_and_window() {
    let (db, sink) = setup().await;
    for text in ["one", "two"] {
        deliver_owner_message(&db, &sink, &DbChat(&db), "bot_1", None, notice(None, text))
            .await
            .unwrap();
    }
    let hour_ago = chrono::Utc::now().timestamp_millis() - 3_600_000;
    assert_eq!(
        count_tool_messages(&db, "bot_1", hour_ago).await.unwrap(),
        2
    );
    assert_eq!(
        count_tool_messages(&db, "bot_2", hour_ago).await.unwrap(),
        0
    );
    let later = chrono::Utc::now().timestamp_millis() + 60_000;
    assert_eq!(count_tool_messages(&db, "bot_1", later).await.unwrap(), 0);
}

#[tokio::test]
async fn reply_notifications_do_not_count_against_the_tool_limit() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "TRIGGER", 0).await;
    notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
        .await
        .unwrap();
    let hour_ago = chrono::Utc::now().timestamp_millis() - 3_600_000;
    assert_eq!(
        count_tool_messages(&db, "bot_1", hour_ago).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn a_resumed_turn_notifies_even_when_a_person_asked() {
    let (db, sink) = setup().await;
    turn(&db, "t1", "MESSAGE", 0).await;
    assert!(
        !notify_for_reply(&db, &sink, &reply("t1", "re-1", "Hi"))
            .await
            .unwrap()
    );
    let resumed = Reply {
        resumed: true,
        ..reply("t1", "re-2", "Done")
    };
    assert!(notify_for_reply(&db, &sink, &resumed).await.unwrap());
}
