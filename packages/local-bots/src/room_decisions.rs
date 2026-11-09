//! Durable group participation checkpoints and dependencies on another participant's reply.

use crate::{Error, LocalBotStore, Result, store::now_micros};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

const WAIT_MICROS: i64 = 24 * 60 * 60 * 1_000_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomDecisionState {
    pub handled_through: i64,
    pub participant_id: Option<String>,
    /// The participant answered, left the room, or the wait expired.
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomWaitOutcome {
    Parked,
    AlreadyAnswered,
    Unavailable,
    Superseded,
}

fn active(db: &Connection, conversation: &str, participant: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_members
         WHERE conversation_id = ?1 AND principal_id = ?2 AND status = 'ACTIVE' AND left_at IS NULL)",
        params![conversation, participant],
        |row| row.get(0),
    )?)
}

fn answered(db: &Connection, conversation: &str, participant: &str, after: i64) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM entries WHERE conversation_id = ?1 AND sender_id = ?2
         AND seq > ?3 AND kind IN ('message', 'transcript')
         AND COALESCE(json_extract(envelope, '$.body.deleted'), 0) != 1
         AND json_extract(envelope, '$.body.approval') IS NULL
         AND json_extract(envelope, '$.body.approval_response') IS NULL)",
        params![conversation, participant, after],
        |row| row.get(0),
    )?)
}

impl LocalBotStore {
    pub fn room_decision_state(&self, bot: &str, conversation: &str) -> Result<RoomDecisionState> {
        self.write(|db, _changes| {
            let row: Option<(i64, Option<String>, Option<i64>)> = db
                .query_row(
                    "SELECT handled_through, participant_id, expires_at FROM room_decisions
                 WHERE bot_id = ?1 AND conversation_id = ?2",
                    params![bot, conversation],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((handled_through, participant_id, expires)) = row else {
                return Ok(RoomDecisionState::default());
            };
            let topic: Option<i64> = db.query_row(
                "SELECT MAX(seq) FROM entries WHERE conversation_id = ?1 AND kind = 'topic'
                 AND seq > ?2 AND COALESCE(json_extract(envelope, '$.body.deleted'), 0) != 1",
                params![conversation, handled_through],
                |row| row.get(0),
            )?;
            if let Some(topic) = topic {
                db.execute(
                    "UPDATE room_decisions SET handled_through = ?3,
                     participant_id = NULL, expires_at = NULL
                     WHERE bot_id = ?1 AND conversation_id = ?2",
                    params![bot, conversation, topic],
                )?;
                return Ok(RoomDecisionState {
                    handled_through: topic,
                    participant_id: None,
                    ready: true,
                });
            }
            let ready = match participant_id.as_deref() {
                Some(participant) => {
                    expires.is_some_and(|until| until <= now_micros())
                        || !active(db, conversation, participant)?
                        || answered(db, conversation, participant, handled_through)?
                }
                None => true,
            };
            Ok(RoomDecisionState {
                handled_through,
                participant_id,
                ready,
            })
        })
    }

    /// With no participant, records that the bot chose not to answer this message.
    pub fn settle_room_decision(
        &self,
        bot: &str,
        conversation: &str,
        trigger: &str,
        participant: Option<&str>,
    ) -> Result<RoomWaitOutcome> {
        self.write(|db, _changes| {
            let group: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1 AND kind = 'group')",
                [conversation], |row| row.get(0),
            )?;
            if (!group && participant.is_some()) || !active(db, conversation, bot)? {
                return Ok(RoomWaitOutcome::Unavailable);
            }
            let seq: Option<i64> = db.query_row(
                "SELECT seq FROM entries WHERE conversation_id = ?1 AND message_id = ?2",
                params![conversation, trigger], |row| row.get(0),
            ).optional()?.flatten();
            let seq = seq.ok_or_else(|| Error::invalid("The decision's message no longer exists"))?;
            let topic: Option<i64> = db.query_row(
                "SELECT MAX(seq) FROM entries WHERE conversation_id = ?1 AND kind = 'topic'
                 AND seq > ?2 AND COALESCE(json_extract(envelope, '$.body.deleted'), 0) != 1",
                params![conversation, seq], |row| row.get(0),
            )?;
            if let Some(topic) = topic {
                db.execute(
                    "INSERT INTO room_decisions (bot_id, conversation_id, handled_through, participant_id, expires_at)
                     VALUES (?1, ?2, ?3, NULL, NULL) ON CONFLICT(bot_id, conversation_id) DO UPDATE SET
                     handled_through = excluded.handled_through, participant_id = NULL, expires_at = NULL
                     WHERE room_decisions.handled_through < excluded.handled_through",
                    params![bot, conversation, topic],
                )?;
                return Ok(RoomWaitOutcome::Superseded);
            }
            if let Some(participant) = participant {
                if participant == bot || !active(db, conversation, participant)? {
                    return Ok(RoomWaitOutcome::Unavailable);
                }
                if answered(db, conversation, participant, seq)? {
                    return Ok(RoomWaitOutcome::AlreadyAnswered);
                }
                let mut next = Some(participant.to_owned());
                let mut seen = HashSet::from([bot.to_owned()]);
                while let Some(id) = next {
                    if !seen.insert(id.clone()) {
                        return Ok(RoomWaitOutcome::Unavailable);
                    }
                    next = db.query_row(
                        "SELECT participant_id FROM room_decisions WHERE bot_id = ?1
                         AND conversation_id = ?2 AND expires_at > ?3",
                        params![id, conversation, now_micros()], |row| row.get(0),
                    ).optional()?.flatten();
                }
            }
            let written = db.execute(
                "INSERT INTO room_decisions (bot_id, conversation_id, handled_through, participant_id, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(bot_id, conversation_id) DO UPDATE SET
                 handled_through = excluded.handled_through,
                 participant_id = excluded.participant_id, expires_at = excluded.expires_at
                 WHERE room_decisions.handled_through <= excluded.handled_through",
                params![bot, conversation, seq, participant, participant.map(|_| now_micros() + WAIT_MICROS)],
            )?;
            if written == 0 {
                return Ok(RoomWaitOutcome::Superseded);
            }
            Ok(RoomWaitOutcome::Parked)
        })
    }

    /// A new addressed turn or a resumed reply replaces the old dependency.
    pub fn clear_room_wait(&self, bot: &str, conversation: &str) -> Result<()> {
        self.write(|db, _changes| {
            db.execute(
                "UPDATE room_decisions SET participant_id = NULL, expires_at = NULL
                 WHERE bot_id = ?1 AND conversation_id = ?2",
                params![bot, conversation],
            )?;
            Ok(())
        })
    }
}
