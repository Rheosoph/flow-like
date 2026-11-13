//! The person's answer to the approval card of a parked local turn, and the grants the local
//! store keeps. The answer stores what it says per item (a grant for "always", a token for the
//! resumed run), records itself in the conversation and releases the turn, which the rule then
//! runs again.

use super::store::{HostStore, StoreGrants};
use flow_like::flow::copilot::tool_grants::record_batch_answer;
use flow_like_local_bots::codes;
use flow_like_types::{
    bot_message::ItemDecision,
    tool_grant::{ApprovalAnswer, ApprovalDecision, ERROR_APPROVAL_ITEM_UNKNOWN},
};
use serde::Serialize;
use std::sync::Arc;

/// The codes the account API answers with for the same refusals.
const APPROVAL_UNKNOWN: &str = "BOT_APPROVAL_UNKNOWN";
const TURN_NOT_WAITING: &str = "BOT_TURN_NOT_WAITING";

/// Why an answer was not taken, serialized as `{ code, message }`. A card reads
/// `BOT_TURN_NOT_WAITING` as "answered on another device".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnswerRefused {
    pub code: &'static str,
    pub message: String,
}

impl AnswerRefused {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn storage(message: String) -> Self {
        Self::new(codes::STORAGE, message)
    }
}

/// `decision` answers every item of the card that `items` does not list. Nothing is stored when
/// the turn no longer waits for this card or an item is not on it.
pub async fn answer_local(
    store: &Arc<dyn HostStore>,
    bot_id: &str,
    turn_id: &str,
    approval_id: &str,
    decision: ApprovalDecision,
    items: &[ItemDecision],
) -> Result<(), AnswerRefused> {
    let card = store
        .approval_card(approval_id)
        .await
        .map_err(AnswerRefused::storage)?
        .filter(|card| card.turn_id == turn_id)
        .ok_or_else(|| {
            AnswerRefused::new(
                APPROVAL_UNKNOWN,
                format!("Turn '{turn_id}' waits for no approval '{approval_id}'"),
            )
        })?;
    let not_waiting = || {
        AnswerRefused::new(
            TURN_NOT_WAITING,
            format!("Turn '{turn_id}' no longer waits for approval '{approval_id}'"),
        )
    };
    if !store
        .waits_for(turn_id, approval_id)
        .await
        .map_err(AnswerRefused::storage)?
    {
        return Err(not_waiting());
    }
    card.decisions(decision, items)
        .map_err(|error| AnswerRefused::new(ERROR_APPROVAL_ITEM_UNKNOWN, error))?;
    let grants = StoreGrants {
        store: store.clone(),
        bot_id: bot_id.to_owned(),
    };
    let answer = ApprovalAnswer {
        approval_id: approval_id.to_owned(),
        decision,
        items: items.to_vec(),
    };
    let response = record_batch_answer(&grants, &card, &answer)
        .await
        .map_err(|error| {
            AnswerRefused::storage(format!(
                "The answer to approval '{approval_id}' could not be stored: {error}"
            ))
        })?;
    if store
        .resolve_parked(bot_id, turn_id, &response)
        .await
        .map_err(AnswerRefused::storage)?
    {
        Ok(())
    } else {
        Err(not_waiting())
    }
}
