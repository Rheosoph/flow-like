//! Scoped execution checkpoints and cancellation fences for local bot turns.

use crate::{Error, InboxEntry, LocalBotStore, Result, store::now_micros};
use flow_like_types::bot_turn::{BotRunCheckpoint, failure_message_id, reply_message_id, turn_id};
use flow_like_types::inbox::{InboxEntryKind, TriggerBody, TriggerSource};
use rusqlite::{Connection, OptionalExtension, params};

fn scope(db: &Connection, bot: &str, room: &str, trigger: &str) -> Result<()> {
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_members m JOIN bots b ON b.id = m.principal_id JOIN entries e
         ON e.conversation_id = m.conversation_id
         WHERE m.conversation_id = ?1 AND m.principal_id = ?2
         AND m.status = 'ACTIVE' AND m.left_at IS NULL AND e.message_id = ?3
         AND COALESCE(json_extract(e.envelope, '$.body.deleted'), 0) != 1)",
        params![room, bot, trigger], |row| row.get(0),
    )?;
    if !valid {
        return Err(Error::not_found(
            "The bot no longer holds this turn's conversation or message",
        ));
    }
    Ok(())
}

impl LocalBotStore {
    /// Reuse the latest completed observation for this exact monitoring instruction.
    pub fn previous_monitor_result(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
    ) -> Result<Option<String>> {
        self.read(|db| {
            scope(db, bot, room, trigger)?;
            let Some(current) = crate::inbox::entry_by_message_id(db, trigger)? else { return Ok(None); };
            let Some(seq) = current.seq else { return Ok(None); };
            if current.kind != InboxEntryKind::Trigger || current.sender_id != crate::LOCAL_OWNER_ID {
                return Ok(None);
            }
            let Ok(body) = TriggerBody::parse(&current.envelope["body"]) else { return Ok(None); };
            if body.source != TriggerSource::Schedule || !body.monitoring { return Ok(None); }
            let rows = db.prepare(
                "SELECT c.checkpoint FROM bot_run_checkpoints c
                 JOIN entries e ON e.message_id = c.trigger_message_id AND e.conversation_id = c.conversation_id
                 WHERE c.bot_id = ?1 AND c.conversation_id = ?2 AND c.cancelled = 0 AND c.checkpoint IS NOT NULL
                 AND e.seq < ?3 AND e.kind = 'trigger' AND e.sender_id = ?4
                 AND json_extract(e.envelope, '$.body.source') = 'schedule'
                 AND json_extract(e.envelope, '$.body.monitoring') = 1
                 AND json_extract(e.envelope, '$.body.ref_id') = ?5
                 AND json_extract(e.envelope, '$.body.text') = ?6
                 AND COALESCE(json_extract(e.envelope, '$.body.deleted'), 0) != 1
                 ORDER BY e.seq DESC LIMIT 16",
            )?.query_map(params![bot, room, seq, crate::LOCAL_OWNER_ID, body.ref_id, body.text], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for text in rows {
                let Ok(saved) = serde_json::from_str::<BotRunCheckpoint>(&text) else { continue; };
                if saved.validate().is_err() || saved.state["phase"] != "finished"
                    || !saved.state["answer"]["stop_reason"].is_null() { continue; }
                if let Some(result) = saved.state["answer"]["output"].as_str().filter(|text| !text.trim().is_empty()) {
                    return Ok(Some(result.chars().take(4_000).collect()));
                }
            }
            Ok(None)
        })
    }

    pub fn run_checkpoint(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
    ) -> Result<Option<BotRunCheckpoint>> {
        self.read(|db| {
            scope(db, bot, room, trigger)?;
            let row: Option<(Option<String>, bool)> = db
                .query_row(
                    "SELECT checkpoint, cancelled FROM bot_run_checkpoints
                 WHERE turn_id = ?1 AND bot_id = ?2 AND conversation_id = ?3",
                    params![turn_id(bot, trigger), bot, room],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            match row {
                Some((_, true)) => Err(Error::local_changed("This turn was cancelled")),
                Some((Some(text), false)) => {
                    let checkpoint: BotRunCheckpoint = serde_json::from_str(&text)?;
                    checkpoint.validate().map_err(Error::invalid)?;
                    Ok(Some(checkpoint))
                }
                _ => Ok(None),
            }
        })
    }

    pub fn save_run_checkpoint(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
        checkpoint: &BotRunCheckpoint,
    ) -> Result<()> {
        checkpoint.validate().map_err(Error::invalid)?;
        let revision = i64::try_from(checkpoint.revision)
            .map_err(|_| Error::invalid("The checkpoint revision is too large"))?;
        let text = serde_json::to_string(checkpoint)?;
        self.write(|db, _| {
            scope(db, bot, room, trigger)?;
            let old: Option<(i64, Option<String>, bool)> = db.query_row(
                "SELECT revision, checkpoint, cancelled FROM bot_run_checkpoints WHERE turn_id = ?1",
                [turn_id(bot, trigger)], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?;
            if let Some((previous, prior, cancelled)) = old {
                if !cancelled && previous == revision && prior.as_deref() == Some(text.as_str()) { return Ok(()); }
                if cancelled || previous != revision - 1 {
                    return Err(Error::local_changed("This turn was cancelled or its checkpoint changed"));
                }
                if let Some(prior) = prior {
                    let prior: BotRunCheckpoint = serde_json::from_str(&prior)?;
                    if prior.started_at_ms != checkpoint.started_at_ms || prior.deadline_at_ms != checkpoint.deadline_at_ms {
                        return Err(Error::invalid("A resumed turn keeps its original start and deadline"));
                    }
                }
            } else if revision != 1 {
                return Err(Error::local_changed("The first checkpoint revision must be 1"));
            }
            db.execute(
                "INSERT INTO bot_run_checkpoints
                 (turn_id, bot_id, conversation_id, trigger_message_id, revision, checkpoint, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(turn_id) DO UPDATE SET revision = excluded.revision,
                 checkpoint = excluded.checkpoint, updated_at = excluded.updated_at",
                params![turn_id(bot, trigger), bot, room, trigger, revision, text, now_micros()],
            )?;
            Ok(())
        })
    }

    pub fn cancel_run(&self, bot: &str, room: &str, trigger: &str) -> Result<()> {
        self.write(|db, _| {
            scope(db, bot, room, trigger)?;
            db.execute(
                "INSERT INTO bot_run_checkpoints
                 (turn_id, bot_id, conversation_id, trigger_message_id, cancelled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT(turn_id) DO UPDATE SET cancelled = 1, checkpoint = NULL,
                 updated_at = excluded.updated_at",
                params![turn_id(bot, trigger), bot, room, trigger, now_micros()],
            )?;
            Ok(())
        })
    }

    pub fn run_cancelled(&self, bot: &str, trigger: &str) -> Result<bool> {
        self.read(|db| {
            Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM bot_run_checkpoints WHERE turn_id = ?1 AND cancelled = 1)",
            [turn_id(bot, trigger)], |row| row.get(0),
        )?)
        })
    }

    /// Pin unfinished turns before newer messages. Published replies and new topics retire them.
    pub fn recoverable_run_triggers(&self, room: &str) -> Result<Vec<(String, InboxEntry)>> {
        self.read(|db| {
            let rows = db.prepare(
                "SELECT c.bot_id, c.trigger_message_id FROM bot_run_checkpoints c
                 JOIN entries trigger ON trigger.message_id = c.trigger_message_id
                 JOIN conversation_members m ON m.conversation_id = c.conversation_id AND m.principal_id = c.bot_id
                 WHERE c.conversation_id = ?1 AND c.cancelled = 0 AND c.checkpoint IS NOT NULL
                 AND m.status = 'ACTIVE' AND m.left_at IS NULL
                 AND COALESCE(json_extract(trigger.envelope, '$.body.deleted'), 0) != 1
                 AND NOT EXISTS(SELECT 1 FROM room_decisions d WHERE d.bot_id = c.bot_id
                     AND d.conversation_id = c.conversation_id AND d.handled_through >= trigger.seq
                     AND json_extract(c.checkpoint, '$.state.phase') = 'finished')
                 AND NOT EXISTS(SELECT 1 FROM entries topic WHERE topic.conversation_id = c.conversation_id
                     AND topic.kind = 'topic' AND topic.seq > trigger.seq)
                 ORDER BY trigger.seq",
            )?.query_map([room], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut result = Vec::new();
            for (bot, trigger) in rows {
                let replied: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM entries WHERE message_id IN (?1, ?2))",
                    params![reply_message_id(&bot, &trigger), failure_message_id(&bot, &trigger)], |row| row.get(0),
                )?;
                if !replied && let Some(entry) = crate::inbox::entry_by_message_id(db, &trigger)? {
                    result.push((bot, entry));
                }
            }
            Ok(result)
        })
    }
}
