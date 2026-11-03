//! Conversations as their reader sees them, in a constant number of statements however many
//! conversations there are: one for the conversations with their unread counts and newest
//! message ids, one for the active members, one for the newest messages.

use super::{
    Reader, active_members, conversation_not_found,
    model::{Conversation, ConversationMember, InboxEntry},
    stored,
};
use crate::{entity::inbox_entry, error::ApiError};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::InboxEntryKind;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter, Statement,
    Value,
};
use std::collections::HashMap;

/// The most conversations one list returns.
pub(super) const MAX_LISTED_CONVERSATIONS: u64 = 200;

#[derive(Debug, FromQueryResult)]
struct ConversationRow {
    id: String,
    kind: String,
    title: Option<String>,
    encryption: String,
    created_by: String,
    created_at: DateTime<FixedOffset>,
    last_activity_at: DateTime<FixedOffset>,
    unread_count: i64,
    last_entry_id: Option<String>,
}

/// The reader's conversations, newest activity first; with `only`, just that one if the reader
/// is an active member. The correlated counts run on the at most 200 rows the inner query keeps,
/// each a range scan of the `(inboxId, conversationId, createdAt)` index.
fn conversations_statement(
    db: &impl ConnectionTrait,
    reader: &Reader,
    only: Option<&str>,
) -> Statement {
    let mut values: Vec<Value> = vec![
        reader.inbox.as_str().into(),
        stored(InboxEntryKind::Message.as_str()).into(),
    ];
    let mut filter = String::new();
    if let Some(conversation_id) = only {
        values.push(conversation_id.into());
        filter.push_str(&format!(r#" AND c."id" = ${}"#, values.len()));
    }
    let sql = format!(
        r#"SELECT p."id" AS id, p."kind" AS kind, p."title" AS title, p."encryption" AS encryption,
       p."createdBy" AS created_by, p."createdAt" AS created_at,
       p."lastActivityAt" AS last_activity_at,
       (SELECT COUNT(*) FROM "InboxEntry" e
         WHERE e."inboxId" = $1 AND e."conversationId" = p."id" AND e."kind" = $2
           AND e."senderId" <> $1
           AND (p."lastReadAt" IS NULL OR e."createdAt" > p."lastReadAt")) AS unread_count,
       (SELECT e."id" FROM "InboxEntry" e
         WHERE e."inboxId" = $1 AND e."conversationId" = p."id" AND e."kind" = $2
         ORDER BY e."createdAt" DESC, e."id" DESC LIMIT 1) AS last_entry_id
FROM (
    SELECT c."id", c."kind", c."title", c."encryption", c."createdBy", c."createdAt",
           c."lastActivityAt", m."lastReadAt"
    FROM "ConversationMember" m
    JOIN "Conversation" c ON c."id" = m."conversationId"
    WHERE m."principalId" = $1 AND m."leftAt" IS NULL{filter}
    ORDER BY c."lastActivityAt" DESC, c."id" DESC
    LIMIT {MAX_LISTED_CONVERSATIONS}
) p
ORDER BY p."lastActivityAt" DESC, p."id" DESC"#
    );
    Statement::from_sql_and_values(db.get_database_backend(), sql, values)
}

/// Conversations the reader is an active member of, with active members, the reader's unread
/// count and newest message.
pub(super) async fn conversations_for<C: ConnectionTrait>(
    db: &C,
    reader: &Reader,
    only: Option<&str>,
) -> Result<Vec<Conversation>, DbErr> {
    let rows = ConversationRow::find_by_statement(conversations_statement(db, reader, only))
        .all(db)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let mut members = members_by_conversation(db, &ids).await?;
    let mut last_entries = entries_by_id(db, &reader.inbox, &rows).await?;
    Ok(rows
        .into_iter()
        .map(|row| Conversation {
            members: members.remove(&row.id).unwrap_or_default(),
            unread_count: u64::try_from(row.unread_count).unwrap_or(0),
            last_entry: row
                .last_entry_id
                .as_deref()
                .and_then(|id| last_entries.remove(id)),
            kind: row.kind.to_ascii_lowercase(),
            encryption: row.encryption.to_ascii_lowercase(),
            id: row.id,
            title: row.title,
            created_by: row.created_by,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
        })
        .collect())
}

async fn members_by_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_ids: &[String],
) -> Result<HashMap<String, Vec<ConversationMember>>, DbErr> {
    let mut members: HashMap<String, Vec<ConversationMember>> = HashMap::new();
    for member in active_members(db, conversation_ids).await? {
        members
            .entry(member.conversation_id.clone())
            .or_default()
            .push(member.into());
    }
    Ok(members)
}

/// The newest messages the rows name, from the reader's inbox, by entry id.
async fn entries_by_id<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    rows: &[ConversationRow],
) -> Result<HashMap<String, InboxEntry>, DbErr> {
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row.last_entry_id.clone())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(inbox_entry::Entity::find()
        .filter(inbox_entry::Column::Id.is_in(ids))
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id.clone(), InboxEntry::from(row)))
        .collect())
}

/// One conversation as `reader` sees it, if they are an active member.
pub(super) async fn conversation_for<C: ConnectionTrait>(
    db: &C,
    reader: &Reader,
    conversation_id: &str,
) -> Result<Option<Conversation>, DbErr> {
    Ok(conversations_for(db, reader, Some(conversation_id))
        .await?
        .into_iter()
        .next())
}

/// [`conversation_for`], with a conversation the reader is not in reading as missing.
pub(super) async fn visible_conversation<C: ConnectionTrait>(
    db: &C,
    reader: &Reader,
    conversation_id: &str,
) -> Result<Conversation, ApiError> {
    conversation_for(db, reader, conversation_id)
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))
}
