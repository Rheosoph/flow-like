//! Pinned conversations and starred messages (personal) and pinned messages (shared by the
//! conversation), shaped like the API's `/pin`, `/pins`, `/stars` routes. The person is the only
//! reader, so every personal row is theirs.

use super::{
    ENTRY_COLUMNS, active_membership, entry_by_message_id, position, read_entry,
    targets::live_message_author,
};
use crate::{
    LOCAL_OWNER_ID,
    error::{Error, Result, codes},
    model::{ConversationPinState, InboxEntry, PinnedMessage, StarredMessage, StarredMessagesPage},
    store::{LocalBotStore, now_micros, timestamp},
};
use flow_like_types::inbox::{EntryCursor, InboxEntryKind};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const MAX_PINNED_MESSAGES: i64 = 50;
const DEFAULT_STARS_PAGE: u64 = 50;
const MAX_STARS_PAGE: u64 = 200;

pub(super) fn conversation_pinned_at(
    db: &Connection,
    conversation_id: &str,
) -> Result<Option<i64>> {
    Ok(db
        .prepare_cached("SELECT pinned_at FROM pinned_conversations WHERE conversation_id = ?1")?
        .query_row([conversation_id], |row| row.get(0))
        .optional()?)
}

/// A deleted message is neither pinned nor starred any more.
pub(super) fn drop_message(db: &Connection, conversation_id: &str, message_id: &str) -> Result<()> {
    db.execute(
        "DELETE FROM pinned_messages WHERE conversation_id = ?1 AND message_id = ?2",
        params![conversation_id, message_id],
    )?;
    db.execute(
        "DELETE FROM starred_messages WHERE conversation_id = ?1 AND message_id = ?2",
        params![conversation_id, message_id],
    )?;
    Ok(())
}

/// The newest edit of `message` by its sender.
fn latest_edit(db: &Connection, message: &InboxEntry) -> Result<Option<InboxEntry>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {ENTRY_COLUMNS} FROM entries
             WHERE conversation_id = ?1 AND kind = ?2 AND sender_id = ?3
               AND json_extract(envelope, '$.body.target') = ?4
             ORDER BY created_at DESC, id DESC LIMIT 1"
        ))?
        .query_row(
            params![
                message.conversation_id,
                InboxEntryKind::Edit.as_str(),
                message.sender_id,
                message.message_id
            ],
            read_entry,
        )
        .optional()?)
}

fn pinned_message(
    db: &Connection,
    message_id: String,
    pinned_by: String,
    pinned_at: i64,
) -> Result<PinnedMessage> {
    let entry = entry_by_message_id(db, &message_id)?;
    let latest_edit = entry
        .as_ref()
        .map(|entry| latest_edit(db, entry))
        .transpose()?
        .flatten();
    Ok(PinnedMessage {
        message_id,
        pinned_by,
        pinned_at: timestamp(pinned_at),
        entry,
        latest_edit,
    })
}

fn pin_row(db: &Connection, conversation_id: &str, message_id: &str) -> Result<PinnedMessage> {
    let (pinned_by, pinned_at): (String, i64) = db.query_row(
        "SELECT pinned_by, pinned_at FROM pinned_messages
         WHERE conversation_id = ?1 AND message_id = ?2",
        params![conversation_id, message_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    pinned_message(db, message_id.to_string(), pinned_by, pinned_at)
}

fn starred_message(
    db: &Connection,
    conversation_id: String,
    message_id: String,
    starred_at: i64,
) -> Result<StarredMessage> {
    let entry = entry_by_message_id(db, &message_id)?.ok_or_else(|| {
        Error::not_found(format!(
            "Message '{message_id}' is not in conversation '{conversation_id}'"
        ))
    })?;
    Ok(StarredMessage {
        latest_edit: latest_edit(db, &entry)?,
        cursor: EntryCursor::new(starred_at, message_id.as_str()).to_string(),
        conversation_id,
        message_id,
        starred_at: timestamp(starred_at),
        entry,
    })
}

fn star_row(db: &Connection, conversation_id: &str, message_id: &str) -> Result<StarredMessage> {
    let starred_at: i64 = db.query_row(
        "SELECT starred_at FROM starred_messages WHERE message_id = ?1",
        [message_id],
        |row| row.get(0),
    )?;
    starred_message(
        db,
        conversation_id.to_string(),
        message_id.to_string(),
        starred_at,
    )
}

fn pin_limit() -> Error {
    Error::new(
        codes::INBOX_PIN_LIMIT,
        format!(
            "A conversation holds at most {MAX_PINNED_MESSAGES} pinned messages; unpin one first"
        ),
    )
}

impl LocalBotStore {
    /// Pins the conversation to the top of the person's list, or unpins it. Pinning one that is
    /// pinned keeps its pin time.
    pub fn set_conversation_pinned(
        &self,
        conversation_id: &str,
        pinned: bool,
    ) -> Result<ConversationPinState> {
        self.write(|tx, changes| {
            active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            let before = conversation_pinned_at(tx, conversation_id)?;
            let pinned_at = if pinned {
                tx.execute(
                    "INSERT OR IGNORE INTO pinned_conversations (conversation_id, pinned_at)
                     VALUES (?1, ?2)",
                    params![conversation_id, now_micros()],
                )?;
                conversation_pinned_at(tx, conversation_id)?
            } else {
                tx.execute(
                    "DELETE FROM pinned_conversations WHERE conversation_id = ?1",
                    [conversation_id],
                )?;
                None
            };
            if before != pinned_at {
                changes.conversation(conversation_id);
            }
            Ok(ConversationPinState {
                conversation_id: conversation_id.to_string(),
                pinned_at: pinned_at.map(timestamp),
            })
        })
    }

    /// The conversation's pinned messages, newest pin first.
    pub fn pins(&self, conversation_id: &str) -> Result<Vec<PinnedMessage>> {
        self.read(|db| {
            active_membership(db, conversation_id, LOCAL_OWNER_ID)?;
            let rows: Vec<(String, String, i64)> = db
                .prepare_cached(
                    "SELECT message_id, pinned_by, pinned_at FROM pinned_messages
                     WHERE conversation_id = ?1 ORDER BY pinned_at DESC, message_id DESC",
                )?
                .query_map([conversation_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows.into_iter()
                .map(|(message_id, pinned_by, pinned_at)| {
                    pinned_message(db, message_id, pinned_by, pinned_at)
                })
                .collect()
        })
    }

    /// Pins a live message for the conversation. Pinning a pinned message returns its pin; a
    /// conversation takes [`MAX_PINNED_MESSAGES`] at most.
    pub fn pin_message(&self, conversation_id: &str, message_id: &str) -> Result<PinnedMessage> {
        self.write(|tx, changes| {
            active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            live_message_author(tx, conversation_id, message_id)?;
            let pinned: i64 = tx.query_row(
                "SELECT COUNT(*) FROM pinned_messages WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO pinned_messages
                    (conversation_id, message_id, pinned_by, pinned_at)
                 SELECT ?1, ?2, ?3, ?4 WHERE ?5 < ?6",
                params![
                    conversation_id,
                    message_id,
                    LOCAL_OWNER_ID,
                    now_micros(),
                    pinned,
                    MAX_PINNED_MESSAGES
                ],
            )?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM pinned_messages
                                WHERE conversation_id = ?1 AND message_id = ?2)",
                params![conversation_id, message_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(pin_limit());
            }
            if inserted > 0 {
                changes.conversation(conversation_id);
            }
            pin_row(tx, conversation_id, message_id)
        })
    }

    pub fn unpin_message(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        self.write(|tx, changes| {
            active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            let removed = tx.execute(
                "DELETE FROM pinned_messages WHERE conversation_id = ?1 AND message_id = ?2",
                params![conversation_id, message_id],
            )?;
            if removed > 0 {
                changes.conversation(conversation_id);
            }
            Ok(())
        })
    }

    /// Stars a live message for the person. Starring a starred message keeps its star time.
    pub fn star_message(&self, conversation_id: &str, message_id: &str) -> Result<StarredMessage> {
        self.write(|tx, changes| {
            active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            live_message_author(tx, conversation_id, message_id)?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO starred_messages (message_id, conversation_id, starred_at)
                 VALUES (?1, ?2, ?3)",
                params![message_id, conversation_id, now_micros()],
            )?;
            if inserted > 0 {
                changes.conversation(conversation_id);
            }
            star_row(tx, conversation_id, message_id)
        })
    }

    pub fn unstar_message(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        self.write(|tx, changes| {
            active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            let removed = tx.execute(
                "DELETE FROM starred_messages WHERE conversation_id = ?1 AND message_id = ?2",
                params![conversation_id, message_id],
            )?;
            if removed > 0 {
                changes.conversation(conversation_id);
            }
            Ok(())
        })
    }

    /// The person's starred messages, newest star first, of one conversation or of all they are
    /// in. `next_before` is set when older stars remain.
    pub fn stars(
        &self,
        conversation_id: Option<&str>,
        before: Option<&str>,
        limit: Option<u64>,
    ) -> Result<StarredMessagesPage> {
        let before = position("before", before)?;
        let limit = limit.unwrap_or(DEFAULT_STARS_PAGE).clamp(1, MAX_STARS_PAGE);
        self.read(|db| {
            if let Some(conversation_id) = conversation_id {
                active_membership(db, conversation_id, LOCAL_OWNER_ID)?;
            }
            let (before_at, before_id) = before
                .map(|cursor| (cursor.created_at_micros, cursor.entry_id))
                .unzip();
            let mut rows: Vec<(String, String, i64)> = db
                .prepare_cached(
                    "SELECT s.conversation_id, s.message_id, s.starred_at FROM starred_messages s
                     JOIN conversation_members m ON m.conversation_id = s.conversation_id
                     JOIN entries e ON e.message_id = s.message_id
                     WHERE m.principal_id = ?1 AND m.left_at IS NULL AND m.status = 'ACTIVE'
                       AND (?2 IS NULL OR s.conversation_id = ?2)
                       AND (?3 IS NULL OR (s.starred_at, s.message_id) < (?3, ?4))
                       AND json_extract(e.envelope, '$.body.deleted') IS NOT 1
                     ORDER BY s.starred_at DESC, s.message_id DESC LIMIT ?5",
                )?
                .query_map(
                    params![
                        LOCAL_OWNER_ID,
                        conversation_id,
                        before_at,
                        before_id,
                        limit.saturating_add(1) as i64
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?
                .collect::<rusqlite::Result<_>>()?;
            let more = rows.len() as u64 > limit;
            rows.truncate(limit as usize);
            let stars = rows
                .into_iter()
                .map(|(conversation_id, message_id, starred_at)| {
                    starred_message(db, conversation_id, message_id, starred_at)
                })
                .collect::<Result<Vec<_>>>()?;
            let next_before = more
                .then(|| stars.last().map(|star| star.cursor.clone()))
                .flatten();
            Ok(StarredMessagesPage { stars, next_before })
        })
    }
}
