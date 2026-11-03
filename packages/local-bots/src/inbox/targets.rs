//! What reactions, edits and deletions act on. A deletion leaves a tombstone: the message and
//! its edits keep their entry, position, sender and time and lose their content, so a read by
//! position shows the gap as deleted instead of the deleted text. A message takes at most
//! [`MAX_EDITS_PER_MESSAGE`] edits, which keeps a conversation within what one import call
//! rewrites when a deletion lands in the account.

use crate::{
    error::{Error, Result, codes},
    model::InboxEntry,
    store::Json,
};
use flow_like_types::inbox::{InboxEntryKind, body_target, is_tombstone, tombstone_envelope};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

/// Edits one message takes at most.
pub(super) const MAX_EDITS_PER_MESSAGE: usize = 40;

fn target_not_found(message: String) -> Error {
    Error::new(codes::INBOX_TARGET_NOT_FOUND, message)
}

/// The sender of the message `target` names, unless it was deleted.
fn live_message_author(db: &Connection, conversation_id: &str, target: &str) -> Result<String> {
    let (author, content): (String, Json<Value>) = db
        .prepare_cached(
            "SELECT sender_id, envelope FROM entries
             WHERE conversation_id = ?1 AND message_id = ?2 AND kind = ?3",
        )?
        .query_row(
            params![conversation_id, target, InboxEntryKind::Message.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            target_not_found(format!("'{target}' is not a message in this conversation"))
        })?;
    if is_tombstone(&content.0) {
        return Err(target_not_found(format!("'{target}' was deleted")));
    }
    Ok(author)
}

/// A reaction acts on any live message in the conversation; an edit or deletion only on a live
/// message the sender wrote. A deleted message takes nothing more, and a message no more than
/// [`MAX_EDITS_PER_MESSAGE`] edits.
pub(super) fn check_target(
    db: &Connection,
    conversation_id: &str,
    sender_id: &str,
    kind: InboxEntryKind,
    envelope: &Value,
) -> Result<()> {
    let target = body_target(envelope).unwrap_or_default();
    let author = live_message_author(db, conversation_id, target)?;
    if kind.requires_own_target() && author != sender_id {
        return Err(Error::new(
            codes::INBOX_NOT_YOUR_MESSAGE,
            format!(
                "You can only {} messages you sent; '{target}' was sent by someone else",
                kind.as_str()
            ),
        ));
    }
    if kind == InboxEntryKind::Edit {
        check_edit_room(db, conversation_id, sender_id, target)?;
    }
    Ok(())
}

fn check_edit_room(
    db: &Connection,
    conversation_id: &str,
    sender_id: &str,
    target: &str,
) -> Result<()> {
    if edits_of(db, conversation_id, sender_id, target)?.len() < MAX_EDITS_PER_MESSAGE {
        return Ok(());
    }
    Err(Error::new(
        codes::INBOX_EDIT_LIMIT,
        format!(
            "'{target}' was edited {MAX_EDITS_PER_MESSAGE} times, the most a message takes; send a new message instead"
        ),
    ))
}

/// The message ids of the live edits `sender_id` made to `target`.
fn edits_of(
    db: &Connection,
    conversation_id: &str,
    sender_id: &str,
    target: &str,
) -> Result<Vec<String>> {
    let mut statement = db.prepare_cached(
        "SELECT message_id FROM entries
         WHERE conversation_id = ?1 AND kind = ?2 AND sender_id = ?3
           AND json_extract(envelope, '$.body.target') = ?4",
    )?;
    let ids = statement
        .query_map(
            params![
                conversation_id,
                InboxEntryKind::Edit.as_str(),
                sender_id,
                target
            ],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Rewrites the message a plain deletion names, and every edit of it, to the tombstone. The
/// caller checked that `deletion`'s sender wrote the message.
pub(super) fn tombstone_target(db: &Connection, deletion: &InboxEntry) -> Result<()> {
    let target = body_target(&deletion.envelope).unwrap_or_default();
    let mut message_ids = edits_of(db, &deletion.conversation_id, &deletion.sender_id, target)?;
    message_ids.push(target.to_string());
    let tombstone = serde_json::to_string(&tombstone_envelope())?;
    let mut rewrite = db.prepare_cached(
        "UPDATE entries SET envelope = ?4
         WHERE conversation_id = ?1 AND sender_id = ?2 AND message_id = ?3",
    )?;
    for message_id in message_ids {
        rewrite.execute(params![
            deletion.conversation_id,
            deletion.sender_id,
            message_id,
            tombstone
        ])?;
    }
    Ok(())
}
