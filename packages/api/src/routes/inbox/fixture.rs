//! The SQLite inbox the history and bot tests share: every table the module reads, the bot and
//! membership tables from their entities, and helpers that fill them. The conversation lock needs
//! PostgreSQL, so tests call the `*_in` functions the handlers run under it.

use super::{
    entries::{Outgoing, validate_send},
    members::Principal,
    model::SendEntryRequest,
};
use crate::{
    entity::{bot, bot_turn, inbox_entry, membership},
    error::ApiError,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{InboxEntryKind, PrincipalKind};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database,
    DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Schema, sea_query::Table,
};
use serde_json::{Value, json};

const TABLES: [&str; 8] = [
    r#"CREATE TABLE public."Conversation" (
        "id" TEXT PRIMARY KEY, "kind" TEXT NOT NULL, "title" TEXT, "encryption" TEXT NOT NULL,
        "directKey" TEXT UNIQUE, "createdBy" TEXT NOT NULL, "lastActivityAt" TEXT NOT NULL,
        "createdAt" TEXT NOT NULL, "updatedAt" TEXT NOT NULL, "importKey" TEXT, "lastSeq" INTEGER,
        UNIQUE ("createdBy", "importKey"))"#,
    r#"CREATE TABLE public."ConversationMember" (
        "conversationId" TEXT NOT NULL, "principalId" TEXT NOT NULL,
        "principalKind" TEXT NOT NULL, "addedBy" TEXT, "joinedAt" TEXT NOT NULL,
        "leftAt" TEXT, "lastReadAt" TEXT,
        "status" TEXT NOT NULL DEFAULT 'ACTIVE', "limits" TEXT, "invitedBy" TEXT, "invitedAt" TEXT,
        "acceptedAt" TEXT, PRIMARY KEY ("conversationId", "principalId"))"#,
    r#"CREATE TABLE public."InboxEntry" (
        "id" TEXT PRIMARY KEY, "inboxId" TEXT NOT NULL, "conversationId" TEXT NOT NULL,
        "messageId" TEXT NOT NULL, "senderId" TEXT NOT NULL, "kind" TEXT NOT NULL,
        "envelope" TEXT NOT NULL, "createdAt" TEXT NOT NULL, "seq" INTEGER,
        UNIQUE ("inboxId", "messageId"), UNIQUE ("inboxId", "conversationId", "seq"))"#,
    r#"CREATE TABLE public."App" ("id" TEXT PRIMARY KEY, "botId" TEXT UNIQUE)"#,
    r#"CREATE TABLE public."User" (
        "id" TEXT PRIMARY KEY, "name" TEXT, "username" TEXT, "preferredUsername" TEXT)"#,
    r#"CREATE TABLE public."BotTurn" (
        "id" TEXT PRIMARY KEY, "botId" TEXT NOT NULL, "ownerId" TEXT NOT NULL,
        "conversationId" TEXT NOT NULL, "triggerMessageId" TEXT NOT NULL,
        "hostKind" TEXT NOT NULL, "status" TEXT NOT NULL, "attempts" INTEGER NOT NULL,
        "hostId" TEXT, "leaseUntil" TEXT, "operationId" TEXT, "replyMessageId" TEXT,
        "errorCode" TEXT, "hops" INTEGER NOT NULL, "requesterId" TEXT,
        "audience" TEXT NOT NULL DEFAULT 'OWNER', "createdAt" TEXT NOT NULL,
        "updatedAt" TEXT NOT NULL, UNIQUE ("botId", "triggerMessageId"))"#,
    r#"CREATE TABLE public."ConversationSegment" (
        "id" TEXT PRIMARY KEY, "conversationId" TEXT NOT NULL, "authorId" TEXT NOT NULL,
        "level" INTEGER NOT NULL, "seqFrom" INTEGER NOT NULL, "seqTo" INTEGER NOT NULL,
        "tFrom" TEXT NOT NULL, "tTo" TEXT NOT NULL, "title" TEXT, "summary" TEXT NOT NULL,
        "parentId" TEXT, "version" INTEGER NOT NULL, "createdAt" TEXT NOT NULL)"#,
    r#"CREATE TABLE public."ConversationFile" (
        "id" TEXT PRIMARY KEY, "conversationId" TEXT NOT NULL, "uploaderId" TEXT NOT NULL,
        "name" TEXT NOT NULL, "mime" TEXT NOT NULL, "bytes" INTEGER NOT NULL, "sha256" TEXT,
        "status" TEXT NOT NULL DEFAULT 'PENDING', "messageId" TEXT, "deletedAt" TEXT,
        "createdAt" TEXT NOT NULL)"#,
];

pub(crate) async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
        .await
        .unwrap();
    let schema = Schema::new(db.get_database_backend());
    for source in [
        schema.create_table_from_entity(bot::Entity),
        schema.create_table_from_entity(membership::Entity),
    ] {
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
    }
    for statement in TABLES.iter().chain(&super::marks::TEST_TABLES) {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

/// Test times lie in the past: deleting a bot stamps its leave entries with the real now.
pub(crate) fn at(seconds: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_700_000_000 + seconds, 0)
        .unwrap()
        .fixed_offset()
}

pub(crate) fn user(id: &str) -> Principal {
    Principal {
        id: id.into(),
        kind: PrincipalKind::User,
    }
}

pub(crate) fn bot_principal(id: &str) -> Principal {
    Principal {
        id: id.into(),
        kind: PrincipalKind::Bot,
    }
}

pub(crate) fn assert_code(error: &ApiError, status: StatusCode, code: &str) {
    assert_eq!(
        (error.status(), error.public_code()),
        (status, code),
        "{error:?}"
    );
}

pub(crate) fn says(error: &ApiError, text: &str) -> bool {
    error
        .public_message()
        .is_some_and(|message| message.contains(text))
}

/// A live bot of `owner_id` running at `location` (`SERVER`, `DESKTOP` or `DEVICE`).
pub(crate) async fn insert_bot(db: &DatabaseConnection, id: &str, owner_id: &str, location: &str) {
    bot::ActiveModel {
        id: Set(id.into()),
        owner_id: Set(owner_id.into()),
        system_key: Set(None),
        preset_id: Set(None),
        name: Set(Some(id.into())),
        description: Set(None),
        instructions: Set(None),
        appearance: Set(json!({ "kind": "pip" })),
        model_policy: Set(json!({})),
        classification: Set(None),
        pii: Set(false),
        capabilities: Set(Default::default()),
        profile_id: Set(None),
        location: Set(location.into()),
        device_id: Set((location == "DEVICE").then(|| format!("device-of-{id}"))),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
        deleted_at: Set(None),
        import_key: Set(None),
        embedding_model: Set(None),
        stt_model: Set(None),
        tts_model: Set(None),
        image_model: Set(None),
        vision_model: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

/// The owner's Flux on the server: its row stores no capability, its system key gives it
/// `flowpilot`.
pub(crate) async fn insert_flux(db: &DatabaseConnection, id: &str, owner_id: &str) {
    insert_bot(db, id, owner_id, "SERVER").await;
    bot::ActiveModel {
        id: Set(id.into()),
        system_key: Set(Some("flux".into())),
        ..Default::default()
    }
    .update(db)
    .await
    .unwrap();
}

pub(crate) async fn delete_bot(db: &DatabaseConnection, id: &str) {
    bot::Entity::update_many()
        .col_expr(
            bot::Column::DeletedAt,
            sea_orm::sea_query::Expr::value(at(1)),
        )
        .filter(bot::Column::Id.eq(id))
        .exec(db)
        .await
        .unwrap();
}

/// Marks `app_id` as the workspace of `bot_id`.
pub(crate) async fn insert_workspace(db: &DatabaseConnection, app_id: &str, bot_id: &str) {
    db.execute_unprepared(&format!(
        r#"INSERT INTO public."App" ("id", "botId") VALUES ('{app_id}', '{bot_id}')"#
    ))
    .await
    .unwrap();
}

/// A RUNNING turn of `bot_id` in the conversation, claimed by `host_id` until `lease_until`.
pub(crate) async fn running_turn(
    db: &DatabaseConnection,
    id: &str,
    bot_id: &str,
    conversation_id: &str,
    host_id: &str,
    lease_until: DateTime<FixedOffset>,
) {
    bot_turn::ActiveModel {
        id: Set(id.into()),
        bot_id: Set(bot_id.into()),
        owner_id: Set("user-a".into()),
        conversation_id: Set(conversation_id.into()),
        trigger_message_id: Set(format!("trigger-{id}")),
        host_kind: Set("DESKTOP".into()),
        status: Set("RUNNING".into()),
        attempts: Set(1),
        host_id: Set(Some(host_id.into())),
        lease_until: Set(Some(lease_until)),
        operation_id: Set(None),
        reply_message_id: Set(None),
        error_code: Set(None),
        hops: Set(1),
        requester_id: Set(None),
        audience: Set("OWNER".into()),
        created_at: Set(at(0)),
        updated_at: Set(at(0)),
    }
    .insert(db)
    .await
    .unwrap();
}

pub(crate) async fn turn(db: &DatabaseConnection, id: &str) -> bot_turn::Model {
    bot_turn::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

pub(crate) fn plain(body: Value) -> Option<Value> {
    Some(json!({ "v": 1, "enc": "none", "body": body }))
}

pub(crate) fn request(message_id: &str, kind: InboxEntryKind, body: Value) -> SendEntryRequest {
    SendEntryRequest {
        message_id: message_id.into(),
        kind,
        envelope: plain(body),
        envelopes: None,
        turn: None,
    }
}

pub(crate) fn outgoing(message_id: &str, kind: InboxEntryKind, body: Value) -> Outgoing {
    validate_send(request(message_id, kind, body)).unwrap()
}

pub(crate) fn text(message_id: &str) -> Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({ "text": message_id }),
    )
}

/// Every copy of the event, by inbox.
pub(crate) async fn copies(db: &DatabaseConnection, message_id: &str) -> Vec<inbox_entry::Model> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .order_by_asc(inbox_entry::Column::InboxId)
        .all(db)
        .await
        .unwrap()
}

/// The positions `inbox` holds in the conversation, in order.
pub(crate) async fn positions(
    db: &DatabaseConnection,
    inbox: &str,
    conversation_id: &str,
) -> Vec<(i64, String)> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(inbox))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .order_by_asc(inbox_entry::Column::Seq)
        .all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| (entry.seq.unwrap(), entry.message_id))
        .collect()
}
