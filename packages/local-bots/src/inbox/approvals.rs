//! Approval cards of parked local turns: the bot posts the card in its direct conversation with
//! the person and the turn waits in `parked_turns`; the person's answer is an entry that wakes
//! nobody, and it releases the turn.

use super::{
    InboxEntry, Json, LocalBotStore, Result, entry_by_message_id, insert_event, next_event_time,
    open_direct, repeated, validate_send,
};
use crate::{
    LOCAL_OWNER_ID, ParkedTurn,
    bots::{bot_exists, bot_not_found},
    model::SendEntryRequest,
    store::{now_micros, timestamp},
};
use flow_like_types::{
    bot_message::{ApprovalRequest, ApprovalResponse},
    inbox::{InboxEntryKind, plain_envelope},
    tool_grant::{APPROVAL_WAIT_DAYS, approval_response_message_id},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

const DAY_MICROS: i64 = 86_400_000_000;

fn message(message_id: String, body: Value) -> SendEntryRequest {
    SendEntryRequest {
        message_id,
        kind: InboxEntryKind::Message,
        envelope: Some(plain_envelope(body)),
        envelopes: None,
    }
}

impl LocalBotStore {
    /// Posts the card as the bot and parks the turn behind it. Parking again is a repeat.
    pub fn park_turn(
        &self,
        turn_id: &str,
        bot_id: &str,
        conversation_id: &str,
        trigger_message_id: &str,
        card: &ApprovalRequest,
    ) -> Result<InboxEntry> {
        let outgoing = validate_send(message(
            card.id.clone(),
            json!({
                "text": format!("I need your approval: {}", card.summary),
                "approval": card,
            }),
        ))?;
        self.write(|tx, changes| {
            if !bot_exists(tx, bot_id)? {
                return Err(bot_not_found(bot_id));
            }
            let direct = open_direct(tx, bot_id, changes)?;
            let entry = match entry_by_message_id(tx, &card.id)? {
                Some(existing) => repeated(existing, &direct, bot_id)?,
                None => {
                    let at = next_event_time(tx, &direct)?;
                    insert_event(
                        tx,
                        &direct,
                        &card.id,
                        bot_id,
                        InboxEntryKind::Message,
                        outgoing.envelope,
                        at,
                    )?
                }
            };
            tx.execute(
                "INSERT OR REPLACE INTO parked_turns
                 (turn_id, bot_id, conversation_id, trigger_message_id, approval_id, parked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    turn_id,
                    bot_id,
                    conversation_id,
                    trigger_message_id,
                    card.id,
                    now_micros()
                ],
            )?;
            changes.conversation(&direct);
            Ok(entry)
        })
    }

    /// The card with this id as the bot posted it.
    pub fn approval_card(&self, approval_id: &str) -> Result<Option<ApprovalRequest>> {
        self.read(|db| {
            let envelope: Option<Json<Value>> = db
                .query_row(
                    "SELECT envelope FROM entries WHERE message_id = ?1",
                    [approval_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(envelope
                .and_then(|Json(envelope)| envelope.get("body")?.get("approval").cloned())
                .and_then(|card| serde_json::from_value(card).ok()))
        })
    }

    /// Records the person's answer in the conversation and releases the parked turn, which the
    /// caller then runs again. `None` when no turn waits for this card.
    pub fn resolve_parked(
        &self,
        bot_id: &str,
        turn_id: &str,
        response: &ApprovalResponse,
    ) -> Result<Option<ParkedTurn>> {
        let message_id = approval_response_message_id(&response.approval_id);
        let outgoing = validate_send(message(
            message_id.clone(),
            json!({
                "text": "",
                "reply_to": response.approval_id,
                "approval_response": response,
            }),
        ))?;
        self.write(|tx, changes| {
            let parked: Option<(String, String, String, i64)> = tx
                .query_row(
                    "SELECT conversation_id, trigger_message_id, approval_id, parked_at
                     FROM parked_turns WHERE turn_id = ?1 AND bot_id = ?2 AND approval_id = ?3",
                    params![turn_id, bot_id, response.approval_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((conversation_id, trigger_message_id, approval_id, parked_at)) = parked else {
                return Ok(None);
            };
            let direct = open_direct(tx, bot_id, changes)?;
            if entry_by_message_id(tx, &message_id)?.is_none() {
                let at = next_event_time(tx, &direct)?;
                insert_event(
                    tx,
                    &direct,
                    &message_id,
                    LOCAL_OWNER_ID,
                    InboxEntryKind::Message,
                    outgoing.envelope,
                    at,
                )?;
            }
            tx.execute("DELETE FROM parked_turns WHERE turn_id = ?1", [turn_id])?;
            changes.conversation(&direct);
            changes.bot(bot_id);
            Ok(Some(ParkedTurn {
                turn_id: turn_id.to_string(),
                bot_id: bot_id.to_string(),
                conversation_id,
                trigger_message_id,
                approval_id,
                parked_at: timestamp(parked_at),
            }))
        })
    }

    /// Ends the waits older than seven days: the bot says it stopped waiting and the turns leave
    /// the table. Returns them.
    pub fn expire_parked(&self) -> Result<Vec<ParkedTurn>> {
        let cutoff = now_micros() - APPROVAL_WAIT_DAYS * DAY_MICROS;
        let stale: Vec<ParkedTurn> = self
            .parked_turns()?
            .into_iter()
            .filter(|parked| parked.parked_at.timestamp_micros() < cutoff)
            .collect();
        for parked in &stale {
            let notice_id = flow_like_types::bot_turn::failure_message_id(
                &parked.bot_id,
                &parked.trigger_message_id,
            );
            let outgoing = validate_send(message(
                notice_id.clone(),
                json!({ "text": "I waited seven days for your approval and stopped. Ask me again if you still want this done." }),
            ))?;
            self.write(|tx, changes| {
                if bot_exists(tx, &parked.bot_id)? {
                    let direct = open_direct(tx, &parked.bot_id, changes)?;
                    if entry_by_message_id(tx, &notice_id)?.is_none() {
                        let at = next_event_time(tx, &direct)?;
                        insert_event(
                            tx,
                            &direct,
                            &notice_id,
                            &parked.bot_id,
                            InboxEntryKind::Message,
                            outgoing.envelope,
                            at,
                        )?;
                    }
                    changes.conversation(&direct);
                }
                tx.execute(
                    "DELETE FROM parked_turns WHERE turn_id = ?1",
                    [&parked.turn_id],
                )?;
                Ok(())
            })?;
        }
        Ok(stale)
    }
}
