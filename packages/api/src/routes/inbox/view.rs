//! Conversations as their reader sees them, in a constant number of statements however many
//! conversations there are: one for the conversations with their unread counts and newest
//! message ids, one for the active members, one for the newest messages.

use super::{
    MEMBER_DECLINED, MEMBER_PENDING, Reader, active_members, conversation_not_found,
    model::{Conversation, ConversationMember, InboxEntry, PendingMember},
    stored,
};
use crate::{
    entity::{bot, conversation_member, inbox_entry},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::InboxEntryKind;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Statement, Value,
};
use std::collections::{HashMap, HashSet};

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
    WHERE m."principalId" = $1 AND m."leftAt" IS NULL AND m."status" = 'ACTIVE'{filter}
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
    reveal_own_bot_limits(db, reader, &mut members).await?;
    let mut pending = pending_by_conversation(db, &ids).await?;
    let mut last_entries = entries_by_id(db, &reader.inbox, &rows).await?;
    Ok(rows
        .into_iter()
        .map(|row| Conversation {
            members: members.remove(&row.id).unwrap_or_default(),
            pending: pending.remove(&row.id).unwrap_or_default(),
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

async fn pending_by_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_ids: &[String],
) -> Result<HashMap<String, Vec<PendingMember>>, DbErr> {
    let mut pending: HashMap<String, Vec<PendingMember>> = HashMap::new();
    for member in conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.is_in(conversation_ids.iter().cloned()))
        .filter(
            Condition::any()
                .add(conversation_member::Column::Status.eq(MEMBER_PENDING))
                .add(conversation_member::Column::Status.eq(MEMBER_DECLINED)),
        )
        .order_by_asc(conversation_member::Column::InvitedAt)
        .order_by_asc(conversation_member::Column::PrincipalId)
        .all(db)
        .await?
    {
        pending
            .entry(member.conversation_id.clone())
            .or_default()
            .push(member.into());
    }
    Ok(pending)
}

/// Room limits show only to the owner of the bot they limit; everyone else sees none.
async fn reveal_own_bot_limits<C: ConnectionTrait>(
    db: &C,
    reader: &Reader,
    members: &mut HashMap<String, Vec<ConversationMember>>,
) -> Result<(), DbErr> {
    let owner = reader.owner.as_deref().unwrap_or(&reader.inbox);
    let limited: Vec<String> = members
        .values()
        .flatten()
        .filter(|member| member.limits.is_some())
        .map(|member| member.principal_id.clone())
        .collect();
    let owned: HashSet<String> = if limited.is_empty() {
        HashSet::new()
    } else {
        bot::Entity::find()
            .select_only()
            .column(bot::Column::Id)
            .filter(bot::Column::Id.is_in(limited))
            .filter(bot::Column::OwnerId.eq(owner))
            .into_tuple::<String>()
            .all(db)
            .await?
            .into_iter()
            .collect()
    };
    for member in members.values_mut().flatten() {
        if !owned.contains(&member.principal_id) {
            member.limits = None;
        }
    }
    Ok(())
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
