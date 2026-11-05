//! What pinned and starred messages share: they name a message by its `messageId`, the same in
//! every copy, show the reader's own copy with the sender's newest edit, and go when the message
//! is deleted.

use super::{INBOX_TARGET_NOT_FOUND, model::InboxEntry, stored};
use crate::{
    entity::{inbox_entry, pinned_message, starred_message},
    error::ApiError,
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{InboxEntryKind, body_target, is_tombstone, is_valid_message_id};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, Statement};
use std::collections::HashMap;

fn target_not_found(message: String) -> ApiError {
    ApiError::coded(
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
        message,
    )
}

/// The reader's copy of the live message `message_id`, the only kind of entry a pin or star names.
pub(super) async fn live_copy<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    message_id: &str,
) -> Result<inbox_entry::Model, ApiError> {
    if !is_valid_message_id(message_id) {
        return Err(ApiError::bad_request(format!(
            "'{}' is not a message id: 1 to 64 characters of [A-Za-z0-9_-]",
            message_id.chars().take(80).collect::<String>()
        )));
    }
    let copy = copies_of(db, reader, &[message_id.to_string()])
        .await?
        .into_iter()
        .find(|copy| copy.conversation_id == conversation_id)
        .ok_or_else(|| {
            target_not_found(format!(
                "'{message_id}' is not a message in this conversation"
            ))
        })?;
    if is_tombstone(&copy.envelope) {
        return Err(target_not_found(format!("'{message_id}' was deleted")));
    }
    Ok(copy)
}

/// The reader's copies of these messages, deleted ones included.
pub(super) async fn copies_of<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    message_ids: &[String],
) -> Result<Vec<inbox_entry::Model>, DbErr> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    inbox_entry::Entity::find()
        .filter(super::deletion::visible_entries())
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::MessageId.is_in(message_ids.iter().cloned()))
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
        .all(db)
        .await
}

/// The reader's plain edits that may target these messages: one statement over the reader's
/// entries of their conversations from the oldest message on. `messages` is not empty.
fn edits_statement(
    db: &impl ConnectionTrait,
    reader: &str,
    oldest: DateTime<FixedOffset>,
    messages: &[&inbox_entry::Model],
) -> Statement {
    let mut conversations: Vec<&str> = messages
        .iter()
        .map(|message| message.conversation_id.as_str())
        .collect();
    conversations.sort_unstable();
    conversations.dedup();
    let mut values: Vec<sea_orm::Value> = vec![
        reader.into(),
        stored(InboxEntryKind::Edit.as_str()).into(),
        oldest.into(),
    ];
    let mut placeholders = |items: Vec<sea_orm::Value>| {
        let start = values.len();
        let marks: Vec<String> = (1..=items.len())
            .map(|offset| format!("${}", start + offset))
            .collect();
        values.extend(items);
        marks.join(", ")
    };
    let in_conversations = placeholders(conversations.iter().map(|id| (*id).into()).collect());
    let in_targets = placeholders(
        messages
            .iter()
            .map(|message| message.message_id.as_str().into())
            .collect(),
    );
    let visible = super::deletion::visible_entries_sql("\"InboxEntry\"");
    let sql = format!(
        r#"SELECT * FROM "InboxEntry" WHERE {visible} AND "inboxId" = $1 AND "kind" = $2 AND "createdAt" >= $3 AND "conversationId" IN ({in_conversations}) AND ("envelope" -> 'body' ->> 'target') IN ({in_targets})"#
    );
    Statement::from_sql_and_values(db.get_database_backend(), sql, values)
}

/// The sender's newest plain edit of each message in the reader's copy, by message id.
pub(super) async fn latest_edits<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    messages: &[&inbox_entry::Model],
) -> Result<HashMap<String, InboxEntry>, DbErr> {
    let Some(oldest) = messages.iter().map(|message| message.created_at).min() else {
        return Ok(HashMap::new());
    };
    let edits = inbox_entry::Entity::find()
        .from_raw_sql(edits_statement(db, reader, oldest, messages))
        .all(db)
        .await?;
    let by_id: HashMap<&str, &inbox_entry::Model> = messages
        .iter()
        .map(|message| (message.message_id.as_str(), *message))
        .collect();
    let mut newest: HashMap<String, inbox_entry::Model> = HashMap::new();
    for edit in edits {
        let Some(target) = body_target(&edit.envelope).map(str::to_string) else {
            continue;
        };
        let Some(message) = by_id.get(target.as_str()) else {
            continue;
        };
        if edit.sender_id != message.sender_id || edit.conversation_id != message.conversation_id {
            continue;
        }
        let newer = newest.get(&target).is_none_or(|current| {
            (edit.created_at, edit.id.as_str()) > (current.created_at, current.id.as_str())
        });
        if newer {
            newest.insert(target, edit);
        }
    }
    Ok(newest
        .into_iter()
        .map(|(target, edit)| (target, edit.into()))
        .collect())
}

/// Takes a deleted message off its conversation's pins and out of every member's stars, in the
/// deletion's transaction.
pub(super) async fn forget_message<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    message_id: &str,
) -> Result<(), DbErr> {
    pinned_message::Entity::delete_many()
        .filter(pinned_message::Column::ConversationId.eq(conversation_id))
        .filter(pinned_message::Column::MessageId.eq(message_id))
        .exec(txn)
        .await?;
    starred_message::Entity::delete_many()
        .filter(starred_message::Column::ConversationId.eq(conversation_id))
        .filter(starred_message::Column::MessageId.eq(message_id))
        .exec(txn)
        .await?;
    Ok(())
}

/// The pin and star tables for the SQLite test harnesses, which build every table by hand.
#[cfg(test)]
pub(crate) const TEST_TABLES: [&str; 3] = [
    r#"CREATE TABLE public."PinnedConversation" (
        "principalId" TEXT NOT NULL, "conversationId" TEXT NOT NULL, "pinnedAt" TEXT NOT NULL,
        PRIMARY KEY ("principalId", "conversationId"))"#,
    r#"CREATE TABLE public."PinnedMessage" (
        "conversationId" TEXT NOT NULL, "messageId" TEXT NOT NULL, "pinnedBy" TEXT NOT NULL,
        "pinnedAt" TEXT NOT NULL, PRIMARY KEY ("conversationId", "messageId"))"#,
    r#"CREATE TABLE public."StarredMessage" (
        "principalId" TEXT NOT NULL, "messageId" TEXT NOT NULL, "conversationId" TEXT NOT NULL,
        "starredAt" TEXT NOT NULL, PRIMARY KEY ("principalId", "messageId"))"#,
];
