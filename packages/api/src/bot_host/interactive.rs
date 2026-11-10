//! A bot chat that answers the message it sent itself. The message carries `interactive` in its
//! body, so its turn is queued with a not-before ([`ledger::interactive_hold_until`]) and hosts
//! leave it alone. The chat refreshes that hold while it streams and ends the turn by storing its
//! reply under the same `re-` id a host would use, so a host that took over after the chat died
//! and a chat that finished late can not both answer: the completion is a compare-and-set on the
//! queued row and the message id repeats.

use super::ledger;
use crate::{
    entity::bot_turn,
    error::ApiError,
    routes::inbox::{
        BOT_TURN_NOT_CLAIMED,
        entries::{Sent, validate_send},
        model::{SendEntryRequest, TurnClaim},
        now_millis,
        turns::reply_interactive_as_bot,
        wake,
    },
    state::AppState,
};
use flow_like::flow::copilot::ToolTrail;
use flow_like_types::{
    bot_turn::{INTERACTIVE_HOST_PREFIX, TurnAudience, reply_message_id, turn_id},
    inbox::{ConversationKind, InboxEntryKind, plain_envelope},
    tokio::{self, task::JoinHandle},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use utoipa::ToSchema;

const HOLD_EVERY: Duration = Duration::from_secs(30);

/// Which message an interactive request answers: the one the chat just wrote to the bot's inbox.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InteractiveRef {
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub client_run_id: String,
}

/// The queued turn an interactive chat answers, once checked against the caller and the bot.
#[derive(Debug, Clone)]
pub(crate) struct InteractiveTurn {
    pub turn: bot_turn::Model,
    pub host_id: String,
    /// Set once [`Self::open_trail`] found the owner's direct conversation.
    pub trail: Option<ToolTrail>,
}

/// Facts of the stored trigger entry an interactive run acts on, never taken from the client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TriggerFacts {
    pub ephemeral_user_context: bool,
    /// The trigger is a spoken `transcript`: the run is a turn of a live call.
    pub live_call: bool,
}

impl InteractiveTurn {
    /// The turn's tool trail when it answers the owner in their direct conversation: the bot's
    /// earlier calls for the prompt, this turn's calls for the reply.
    pub(crate) async fn open_trail(&mut self, state: &AppState) -> Option<ToolTrail> {
        let turn = &self.turn;
        if TurnAudience::parse(&turn.audience) != Some(TurnAudience::Owner)
            || turn
                .requester_id
                .as_deref()
                .is_some_and(|requester| requester != turn.owner_id)
        {
            return None;
        }
        let section = super::history::transcript(
            &state.db,
            &turn.bot_id,
            &turn.conversation_id,
            &turn.trigger_message_id,
            ConversationKind::Direct,
            &turn.owner_id,
        )
        .await
        .ok()
        .and_then(|transcript| transcript.trail);
        let trail = ToolTrail::new(section);
        self.trail = Some(trail.clone());
        Some(trail)
    }

    /// What the turn's stored trigger entry says about the run answering it.
    pub(crate) async fn trigger_facts(&self, state: &AppState) -> Result<TriggerFacts, ApiError> {
        use crate::entity::inbox_entry;
        let trigger = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(&self.turn.bot_id))
            .filter(inbox_entry::Column::ConversationId.eq(&self.turn.conversation_id))
            .filter(inbox_entry::Column::MessageId.eq(&self.turn.trigger_message_id))
            .one(&state.db)
            .await?;
        Ok(
            trigger.map_or_else(TriggerFacts::default, |entry| TriggerFacts {
                ephemeral_user_context: entry.envelope.get("body").is_some_and(|body| {
                    flow_like_types::bot_user_context::response_for(
                        body,
                        &entry.sender_id,
                        &self.turn.bot_id,
                    )
                }),
                live_call: InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Transcript),
            }),
        )
    }

    pub(crate) async fn find(
        state: &AppState,
        owner_id: &str,
        bot_id: &str,
        reference: &InteractiveRef,
    ) -> Result<Option<Self>, ApiError> {
        let id = turn_id(bot_id, &reference.trigger_message_id);
        let turn = bot_turn::Entity::find_by_id(&id)
            .one(&state.db)
            .await?
            .filter(|turn| {
                turn.owner_id == owner_id
                    && turn.bot_id == bot_id
                    && turn.conversation_id == reference.conversation_id
            });
        Ok(turn.map(|turn| Self {
            turn,
            host_id: format!("{INTERACTIVE_HOST_PREFIX}{}", reference.client_run_id),
            trail: None,
        }))
    }

    /// Keeps hosts away until the returned task is aborted or the hold is lost.
    pub(crate) fn hold(&self, state: &AppState) -> JoinHandle<()> {
        let (state, id, owner) = (
            state.clone(),
            self.turn.id.clone(),
            self.turn.owner_id.clone(),
        );
        tokio::spawn(async move {
            loop {
                match ledger::hold_in(&state.db, &id, &owner, now_millis()).await {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        tracing::warn!(%error, turn_id = %id, "Interactive chat: hold failed")
                    }
                }
                tokio::time::sleep(HOLD_EVERY).await;
            }
        })
    }

    pub(crate) fn request(&self, text: &str, error_code: Option<&str>) -> SendEntryRequest {
        let mut body = json!({
            "text": text,
            "reply_to": self.turn.trigger_message_id,
            "hops": u32::try_from(self.turn.hops).unwrap_or(0).saturating_add(1),
        });
        if let Some(code) = error_code {
            body["error"] = json!({ "code": code });
        } else if let Some(trail) = &self.trail {
            trail.attach(&mut body);
        }
        SendEntryRequest {
            message_id: reply_message_id(&self.turn.bot_id, &self.turn.trigger_message_id),
            kind: InboxEntryKind::Message,
            envelope: Some(plain_envelope(body)),
            envelopes: None,
            turn: None,
        }
    }

    /// Stores the chat's answer as the bot and ends the turn. `None`: a host answers the turn
    /// (or already did), so the chat stores nothing.
    pub(crate) async fn store_reply(
        &self,
        state: &AppState,
        text: &str,
        error_code: Option<&str>,
    ) -> Result<Option<Sent>, ApiError> {
        let outgoing = Arc::new(validate_send(self.request(text, error_code))?);
        let claim = TurnClaim {
            id: self.turn.id.clone(),
            host_id: self.host_id.clone(),
        };
        let sent = state
            .transaction(|txn| {
                let (turn, claim, outgoing) = (self.turn.clone(), claim.clone(), outgoing.clone());
                Box::pin(async move {
                    reply_interactive_as_bot(
                        txn,
                        &turn.conversation_id,
                        &turn.bot_id,
                        &claim,
                        &outgoing,
                        now_millis(),
                    )
                    .await
                })
            })
            .await;
        match sent {
            Ok(sent) => {
                wake(state, sent.wake.clone()).await;
                crate::bot_host::dispatch(state, &sent.bots).await;
                Ok(Some(sent))
            }
            Err(error) if error.public_code() == BOT_TURN_NOT_CLAIMED => Ok(None),
            Err(error) => Err(error),
        }
    }
}
