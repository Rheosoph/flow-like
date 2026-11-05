//! The owner's deletion boundary. Shared event records remain available to the bots.

use super::{
    active_members, conversation_not_found, end_membership, member_conversation, member_event,
    newest_event_time, next_event_time,
};
use crate::{
    LOCAL_OWNER_ID, LocalBotStore,
    error::{Error, Result, codes},
    model::OwnerConversationDeletion,
    store::{now_micros, timestamp},
};
use flow_like_types::inbox::{ConversationKind, MemberOp};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn entry_source(principal_id: &str) -> &'static str {
    if principal_id == LOCAL_OWNER_ID {
        "owner_entries"
    } else {
        "entries"
    }
}

pub(super) fn restore(db: &Connection, conversation_id: &str) -> Result<bool> {
    Ok(db.execute(
        "UPDATE conversations SET owner_hidden = 0 WHERE id = ?1 AND owner_hidden <> 0",
        [conversation_id],
    )? > 0)
}

pub(super) fn exported(
    db: &Connection,
    conversation_id: &str,
) -> Result<Option<OwnerConversationDeletion>> {
    Ok(db.query_row(
        "SELECT owner_cleared_seq, owner_cleared_at, owner_hidden FROM conversations WHERE id = ?1",
        [conversation_id],
        |row| {
            let seq: Option<i64> = row.get(0)?;
            let at: Option<i64> = row.get(1)?;
            let hidden = row.get(2)?;
            Ok(seq
                .zip(at)
                .map(|(cleared_through_seq, at)| OwnerConversationDeletion {
                    cleared_through_seq,
                    cleared_at: timestamp(at),
                    hidden,
                }))
        },
    )?)
}

pub(super) fn message_visible(
    db: &Connection,
    conversation_id: &str,
    message_id: &str,
) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS (SELECT 1 FROM owner_entries WHERE conversation_id = ?1 AND message_id = ?2)",
        params![conversation_id, message_id],
        |row| row.get(0),
    )?)
}

pub(super) fn require_message(
    db: &Connection,
    conversation_id: &str,
    message_id: &str,
) -> Result<()> {
    if message_visible(db, conversation_id, message_id)? {
        return Ok(());
    }
    Err(Error::new(
        codes::INBOX_TARGET_NOT_FOUND,
        "This message is not in your conversation history",
    ))
}

impl LocalBotStore {
    /// Saves the mapping before a later local deletion can remove the group from exports.
    pub(crate) fn record_conversation_transfer(
        &self,
        conversation_id: &str,
        bot_id: &str,
        owner_id: &str,
        account_bot_id: &str,
        account_conversation_id: &str,
    ) -> Result<()> {
        self.write(|tx, _| {
            tx.execute("INSERT INTO conversation_transfers (conversation_id, bot_id, owner_id, account_bot_id, account_conversation_id)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT (conversation_id, bot_id, owner_id, account_bot_id)
                DO UPDATE SET account_conversation_id = excluded.account_conversation_id",
                params![conversation_id, bot_id, owner_id, account_bot_id, account_conversation_id])?;
            Ok(())
        })
    }

    pub(crate) fn pending_transferred_deletions(
        &self,
        bot_id: &str,
        owner_id: &str,
        account_bot_id: &str,
    ) -> Result<Vec<(String, String)>> {
        self.read(|db| {
            let mut rows = db.prepare_cached(
                "SELECT t.conversation_id, t.account_conversation_id FROM conversation_transfers t
                JOIN conversations c ON c.id = t.conversation_id
                WHERE t.bot_id = ?1 AND t.owner_id = ?2 AND t.account_bot_id = ?3
                  AND c.kind = 'group' AND c.owner_hidden = 1 AND c.owner_cleared_seq IS NOT NULL",
            )?;
            let pending = rows
                .query_map(params![bot_id, owner_id, account_bot_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(pending)
        })
    }

    pub(crate) fn acknowledge_transferred_deletion(
        &self,
        conversation_id: &str,
        bot_id: &str,
        owner_id: &str,
        account_bot_id: &str,
    ) -> Result<()> {
        self.write(|tx, _| {
            tx.execute("DELETE FROM conversation_transfers WHERE conversation_id = ?1 AND bot_id = ?2 AND owner_id = ?3 AND account_bot_id = ?4", params![conversation_id, bot_id, owner_id, account_bot_id])?;
            Ok(())
        })
    }

    /// Deletes the owner's view. Groups also end the owner's and their bots' memberships.
    /// A new direct message or explicitly starting that chat again restores it with empty past history.
    pub fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        self.write(|tx, changes| {
            let hidden: bool = tx.query_row(
                "SELECT owner_hidden FROM conversations WHERE id = ?1",
                [conversation_id],
                |row| row.get(0),
            ).optional()?.ok_or_else(|| conversation_not_found(conversation_id))?;
            if hidden {
                return Ok(());
            }
            let current = member_conversation(tx, conversation_id, LOCAL_OWNER_ID)?;
            if current.kind == ConversationKind::Group {
                let members = active_members(tx, conversation_id)?;
                let at = next_event_time(tx, conversation_id)?;
                for member in members {
                    end_membership(tx, conversation_id, &member.principal_id, at)?;
                    member_event(tx, conversation_id, MemberOp::Leave, &member.principal_id, at)?;
                }
            }
            let previous = exported(tx, conversation_id)?.map(|deletion| deletion.cleared_at.timestamp_micros());
            let at = now_micros()
                .max(newest_event_time(tx, conversation_id)?)
                .max(previous.map_or(i64::MIN, |at| at.saturating_add(1)));
            tx.execute(
                "UPDATE conversations SET owner_cleared_seq = last_seq, owner_cleared_at = ?2, owner_hidden = 1 WHERE id = ?1",
                params![conversation_id, at],
            )?;
            tx.execute("DELETE FROM pinned_conversations WHERE conversation_id = ?1", [conversation_id])?;
            tx.execute("DELETE FROM starred_messages WHERE conversation_id = ?1", [conversation_id])?;
            changes.conversation(conversation_id);
            Ok(())
        })
    }
}
