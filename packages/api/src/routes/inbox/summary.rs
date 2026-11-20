//! Each member's unread count and newest message, stored on their membership so the conversation
//! list reads two columns instead of counting entries on every request. Every write that changes
//! what a member counts or sees keeps them: one delivered message bumps them, anything else
//! recomputes them from the member's copy. A NULL `unreadCount` means not computed yet; the list
//! counts live for those rows, and the next write on the row computes it in full.

use super::{MEMBER_ACTIVE, stored};
use crate::entity::{conversation_member, inbox_entry};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::InboxEntryKind;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, QuerySelect, Statement, Value};
use std::collections::{BTreeMap, BTreeSet};

const INBOX: &str = r#""ConversationMember"."principalId""#;
const CONVERSATION: &str = r#""ConversationMember"."conversationId""#;
const READ_MARKER: &str = r#""ConversationMember"."lastReadAt""#;

/// Visible messages in `inbox`'s copy of `conversation` sent by someone else after `after`.
pub(super) fn unread_sql(inbox: &str, conversation: &str, kind: &str, after: &str) -> String {
    let visible = super::deletion::visible_entries_sql("e");
    format!(
        r#"(SELECT COUNT(*) FROM "InboxEntry" e
         WHERE e."inboxId" = {inbox} AND e."conversationId" = {conversation} AND e."kind" = {kind} AND {visible}
           AND e."senderId" <> {inbox}
           AND ({after} IS NULL OR e."createdAt" > {after}))"#
    )
}

/// The id of the newest visible message in `inbox`'s copy of `conversation`.
pub(super) fn newest_sql(inbox: &str, conversation: &str, kind: &str) -> String {
    let visible = super::deletion::visible_entries_sql("e");
    format!(
        r#"(SELECT e."id" FROM "InboxEntry" e
         WHERE e."inboxId" = {inbox} AND e."conversationId" = {conversation} AND e."kind" = {kind} AND {visible}
         ORDER BY e."createdAt" DESC, e."id" DESC LIMIT 1)"#
    )
}

fn message_kind() -> String {
    stored(InboxEntryKind::Message.as_str())
}

/// Appends `principals` as placeholders after the values already bound.
fn bind_principals(values: &mut Vec<Value>, principals: &[String]) -> String {
    principals
        .iter()
        .map(|principal| {
            values.push(principal.as_str().into());
            format!("${}", values.len())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn execute<C: ConnectionTrait>(
    db: &C,
    sql: String,
    values: Vec<Value>,
) -> Result<u64, DbErr> {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .await
    .map(|result| result.rows_affected())
}

/// Recomputes the summaries of `principals` in `conversation_id` from their copies.
pub(super) async fn refresh<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principals: &[String],
) -> Result<(), DbErr> {
    if principals.is_empty() {
        return Ok(());
    }
    let unread = unread_sql(INBOX, CONVERSATION, "$1", READ_MARKER);
    let newest = newest_sql(INBOX, CONVERSATION, "$1");
    let mut values: Vec<Value> = vec![message_kind().into(), conversation_id.into()];
    let members = bind_principals(&mut values, principals);
    let sql = format!(
        r#"UPDATE "ConversationMember" SET "unreadCount" = {unread}, "lastEntryId" = {newest}
WHERE "conversationId" = $2 AND "principalId" IN ({members})"#
    );
    execute(db, sql, values).await.map(|_| ())
}

/// One message delivered to `recipients`: their newest message is their new copy, and their count
/// grows unless they sent it or already read past it. A summary not yet computed is computed in full.
async fn record_message<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    message: &DeliveredMessage,
) -> Result<(), DbErr> {
    let unread = unread_sql(INBOX, CONVERSATION, "$1", READ_MARKER);
    let newest = newest_sql(INBOX, CONVERSATION, "$1");
    let mut values: Vec<Value> = vec![
        message_kind().into(),
        message.sender.as_str().into(),
        message.created_at.into(),
        conversation_id.into(),
    ];
    let members = bind_principals(&mut values, &message.recipients);
    let sql = format!(
        r#"UPDATE "ConversationMember" SET
  "unreadCount" = CASE
    WHEN "unreadCount" IS NULL THEN {unread}
    WHEN "principalId" <> $2 AND ("lastReadAt" IS NULL OR "lastReadAt" < $3) THEN "unreadCount" + 1
    ELSE "unreadCount" END,
  "lastEntryId" = {newest}
WHERE "conversationId" = $4 AND "principalId" IN ({members})"#
    );
    execute(db, sql, values).await.map(|_| ())
}

/// Moves `reader`'s active read marker to `read_at` unless it already is there or later, and
/// recounts what is unread after it; whether it moved. The row is locked first, so on PostgreSQL a
/// send that is committing meanwhile is finished, and counted, before the recount starts.
pub(super) async fn move_marker<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    read_at: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    conversation_member::Entity::find_by_id((conversation_id.to_string(), reader.to_string()))
        .lock_exclusive()
        .one(db)
        .await?;
    let unread = unread_sql(INBOX, CONVERSATION, "$2", "$1");
    let newest = newest_sql(INBOX, CONVERSATION, "$2");
    let values: Vec<Value> = vec![
        read_at.into(),
        message_kind().into(),
        conversation_id.into(),
        reader.into(),
        MEMBER_ACTIVE.into(),
    ];
    let sql = format!(
        r#"UPDATE "ConversationMember" SET "lastReadAt" = $1, "unreadCount" = {unread}, "lastEntryId" = {newest}
WHERE "conversationId" = $3 AND "principalId" = $4 AND "leftAt" IS NULL AND "status" = $5
  AND ("lastReadAt" IS NULL OR "lastReadAt" < $1)"#
    );
    Ok(execute(db, sql, values).await? == 1)
}

struct DeliveredMessage {
    sender: String,
    created_at: DateTime<FixedOffset>,
    recipients: Vec<String>,
}

enum Change {
    Message(DeliveredMessage),
    /// Several messages, or copies a recipient does not see: recount those recipients.
    Refresh(Vec<String>),
}

/// What a batch of copies about to be inserted changes in their members' summaries.
pub(super) struct Delivered(BTreeMap<String, Change>);

#[derive(Default)]
struct Batch<'a> {
    messages: BTreeSet<&'a str>,
    origins: BTreeSet<(&'a str, DateTime<FixedOffset>)>,
    recipients: Vec<String>,
    seen: BTreeSet<&'a str>,
    repeated: bool,
    irregular: bool,
}

impl Delivered {
    pub(super) fn of(copies: &[inbox_entry::Model]) -> Self {
        let kind = message_kind();
        let mut batches: BTreeMap<&str, Batch> = BTreeMap::new();
        for copy in copies.iter().filter(|copy| copy.kind == kind) {
            let batch = batches.entry(copy.conversation_id.as_str()).or_default();
            batch.messages.insert(copy.message_id.as_str());
            batch
                .origins
                .insert((copy.sender_id.as_str(), copy.created_at));
            batch.irregular |= copy.seq.is_none() || copy.cleared != Some(false);
            if batch.seen.insert(copy.inbox_id.as_str()) {
                batch.recipients.push(copy.inbox_id.clone());
            } else {
                batch.repeated = true;
            }
        }
        Self(
            batches
                .into_iter()
                .map(|(conversation_id, batch)| {
                    let single = batch.messages.len() == 1
                        && batch.origins.len() == 1
                        && !batch.repeated
                        && !batch.irregular;
                    let change = match batch.origins.first() {
                        Some((sender, created_at)) if single => Change::Message(DeliveredMessage {
                            sender: (*sender).to_string(),
                            created_at: *created_at,
                            recipients: batch.recipients,
                        }),
                        _ => Change::Refresh(batch.recipients),
                    };
                    (conversation_id.to_string(), change)
                })
                .collect(),
        )
    }

    /// Applies the changes once the copies are written, in the same transaction.
    pub(super) async fn apply<C: ConnectionTrait>(self, db: &C) -> Result<(), DbErr> {
        for (conversation_id, change) in self.0 {
            match change {
                Change::Message(message) => record_message(db, &conversation_id, &message).await?,
                Change::Refresh(principals) => refresh(db, &conversation_id, &principals).await?,
            }
        }
        Ok(())
    }
}
