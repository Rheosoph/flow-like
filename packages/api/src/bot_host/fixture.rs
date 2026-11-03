//! SQLite tests of the ledger and the worker's building blocks: the inbox module's tables and
//! helpers plus the `User` table the history's sender names read, and builders for the
//! deliveries, conversations and messages the turns answer.

pub(super) use crate::routes::inbox::fixture::{assert_code, at, delete_bot, insert_bot, says};
use crate::{
    entity::{conversation, conversation_member, inbox_entry},
    routes::inbox::delivery::{BotDelivery, BotRecipient},
};
use flow_like_types::{
    bot::BotLocation,
    inbox::{ConversationKind, InboxEntryKind},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, Schema,
    sea_query::Table,
};
use serde_json::{Value, json};

pub(super) async fn database() -> DatabaseConnection {
    let db = crate::routes::inbox::fixture::database().await;
    let schema = Schema::new(db.get_database_backend());
    let source = schema.create_table_from_entity(crate::entity::user::Entity);
    let mut table = Table::create();
    table.table(source.get_table_name().unwrap().clone());
    for column in source.get_columns() {
        table.col(column.clone());
    }
    db.execute(&table).await.unwrap();
    db.execute_unprepared(
        r#"CREATE TABLE public."Notification" (
            "id" TEXT PRIMARY KEY, "userId" TEXT NOT NULL, "appId" TEXT, "title" TEXT NOT NULL,
            "description" TEXT, "icon" TEXT, "link" TEXT, "read" INTEGER NOT NULL,
            "sourceRunId" TEXT, "sourceNodeId" TEXT, "createdAt" TEXT NOT NULL, "readAt" TEXT,
            "type" TEXT NOT NULL)"#,
    )
    .await
    .unwrap();
    db
}

/// A conversation with these active members (`user-…` ids are people, `bot_…` ids are bots).
pub(super) async fn conversation(
    db: &DatabaseConnection,
    id: &str,
    kind: &str,
    encryption: &str,
    members: &[&str],
) {
    conversation::ActiveModel {
        id: Set(id.into()),
        kind: Set(kind.into()),
        title: Set(None),
        encryption: Set(encryption.into()),
        direct_key: Set(None),
        created_by: Set(members[0].into()),
        last_activity_at: Set(at(0)),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        import_key: Set(None),
        last_seq: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
    for member in members {
        conversation_member::ActiveModel {
            conversation_id: Set(id.into()),
            principal_id: Set((*member).into()),
            principal_kind: Set(if member.starts_with("bot_") {
                "BOT"
            } else {
                "USER"
            }
            .into()),
            added_by: Set(None),
            joined_at: Set(at(0)),
            left_at: Set(None),
            last_read_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }
}

/// One message as every listed inbox holds it, at position `seq`.
pub(super) async fn message(
    db: &DatabaseConnection,
    conversation_id: &str,
    message_id: &str,
    sender: &str,
    seq: i64,
    body: Value,
    inboxes: &[&str],
) {
    for inbox in inboxes {
        inbox_entry::ActiveModel {
            id: Set(format!("e-{inbox}-{message_id}")),
            inbox_id: Set((*inbox).into()),
            conversation_id: Set(conversation_id.into()),
            message_id: Set(message_id.into()),
            sender_id: Set(sender.into()),
            kind: Set("MESSAGE".into()),
            envelope: Set(json!({ "v": 1, "enc": "none", "body": body })),
            created_at: Set(at(seq)),
            seq: Set(Some(seq)),
        }
        .insert(db)
        .await
        .unwrap();
    }
}

pub(super) fn recipient(bot_id: &str, owner_id: &str, location: BotLocation) -> BotRecipient {
    BotRecipient {
        bot_id: bot_id.into(),
        owner_id: owner_id.into(),
        location,
        device_id: None,
        mentioned: false,
        replied_to: false,
        conversation_id: "c1".into(),
        trigger_message_id: "m1".into(),
        hops: 0,
        interactive: false,
    }
}

/// A person's message in `c1` that reached `recipients`.
pub(super) fn delivery(kind: ConversationKind, recipients: Vec<BotRecipient>) -> BotDelivery {
    BotDelivery {
        conversation_kind: kind,
        sender_id: "user-a".into(),
        kind: InboxEntryKind::Message,
        seq: 1,
        recipients,
    }
}
