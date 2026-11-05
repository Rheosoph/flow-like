//! What reactions, edits and deletions act on. A plain deletion leaves a tombstone: every copy of
//! the message and of its edits keeps its entry, position, sender and time and loses its content,
//! so a read by position shows the gap as deleted instead of the deleted text. A message takes at
//! most [`MAX_EDITS_PER_MESSAGE`] edits, which bounds the copies a deletion rewrites.

use super::{INBOX_EDIT_LIMIT, INBOX_NOT_YOUR_MESSAGE, INBOX_TARGET_NOT_FOUND, stored};
use crate::{
    entity::{conversation_member, inbox_entry},
    error::ApiError,
};
use axum::http::StatusCode;
use flow_like_types::{
    Value,
    inbox::{
        InboxEntryKind, MAX_CONVERSATION_MEMBERS, body_target, is_tombstone, tombstone_envelope,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect, Statement,
    sea_query::Expr,
};

/// Edits one message takes at most.
pub(super) const MAX_EDITS_PER_MESSAGE: usize = 40;

// A deletion inserts its copies, moves the position and touches the lock, rewrites every copy of
// the message and its edits, and drops the message's pin and every member's star of it: far below
// DSQL's 3,000 rows per transaction.
const _: () = assert!(MAX_CONVERSATION_MEMBERS * (MAX_EDITS_PER_MESSAGE + 3) + 3 <= 2_500);

const EDITS_OF_SQL: &str = r#"SELECT "messageId" FROM "InboxEntry" WHERE "inboxId" = $1 AND "conversationId" = $2 AND "kind" = $3 AND "senderId" = $4 AND ("envelope" -> 'body' ->> 'target') = $5"#;

fn target_not_found(message: String) -> ApiError {
    ApiError::coded(
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
        message,
    )
}

/// A plain reaction may act on any message in the conversation the sender received; an edit or
/// deletion only on a message they sent. A deleted message takes nothing more, and a message no
/// more than [`MAX_EDITS_PER_MESSAGE`] edits.
pub(super) async fn check_target<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sender: &str,
    kind: InboxEntryKind,
    envelope: &Value,
) -> Result<(), ApiError> {
    let target = body_target(envelope).unwrap_or_default();
    let message = live_message(db, conversation_id, sender, target).await?;
    if kind.requires_own_target() && message.sender_id != sender {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            INBOX_NOT_YOUR_MESSAGE,
            format!(
                "You can only {} messages you sent; '{target}' was sent by someone else",
                kind.as_str()
            ),
        ));
    }
    if kind == InboxEntryKind::Edit {
        check_edit_room(db, conversation_id, sender, target).await?;
    }
    Ok(())
}

/// The message `target` of `reader`'s copy unless it was deleted.
async fn live_message<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    target: &str,
) -> Result<inbox_entry::Model, ApiError> {
    let message = inbox_entry::Entity::find()
        .filter(super::deletion::visible_entries())
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::MessageId.eq(target))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
        .one(db)
        .await?
        .ok_or_else(|| {
            target_not_found(format!("'{target}' is not a message in this conversation"))
        })?;
    if is_tombstone(&message.envelope) {
        return Err(target_not_found(format!("'{target}' was deleted")));
    }
    Ok(message)
}

async fn check_edit_room<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sender: &str,
    target: &str,
) -> Result<(), ApiError> {
    let edits = edits_of(db, conversation_id, sender, sender, target).await?;
    if edits.len() < MAX_EDITS_PER_MESSAGE {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_EDIT_LIMIT,
        format!(
            "'{target}' was edited {MAX_EDITS_PER_MESSAGE} times, the most a message takes; send a new message instead"
        ),
    ))
}

/// The message ids of the plain edits `sender` made to `target`, read from `witness`'s copy,
/// which holds every one: the sender's own (they receive what they send), or on an import the
/// caller's.
pub(super) async fn edits_of<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    witness: &str,
    sender: &str,
    target: &str,
) -> Result<Vec<String>, DbErr> {
    db.query_all_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        EDITS_OF_SQL,
        [
            witness.into(),
            conversation_id.into(),
            stored(InboxEntryKind::Edit.as_str()).into(),
            sender.into(),
            target.into(),
        ],
    ))
    .await?
    .iter()
    .map(|row| row.try_get::<String>("", "messageId"))
    .collect()
}

/// Rewrites every copy of `target` and of its `edits`, in the inbox of everyone who was ever a
/// member, to the tombstone, and takes it off pins and stars. The caller checked that `sender`
/// sent the plain message.
pub(super) async fn tombstone<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    target: &str,
    edits: Vec<String>,
) -> Result<(), DbErr> {
    super::marks::forget_message(txn, conversation_id, target).await?;
    let mut message_ids = edits;
    message_ids.push(target.to_string());
    let holders: Vec<String> = conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::PrincipalId)
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .into_tuple()
        .all(txn)
        .await?;
    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Envelope,
            Expr::value(tombstone_envelope()),
        )
        .filter(inbox_entry::Column::InboxId.is_in(holders))
        .filter(inbox_entry::Column::MessageId.is_in(message_ids))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::SenderId.eq(sender))
        .exec(txn)
        .await
        .map(|_| ())
}
