//! The person's answer to the approval card of a parked local turn, and the grants the local
//! store keeps. The answer stores what it says (a grant for "always", a token for the resumed
//! run), records itself in the conversation and releases the turn, which the rule then runs again.

use super::store::{HostStore, StoreGrants};
use flow_like::flow::copilot::tool_grants::record_card_answer;
use flow_like_types::{bot_message::ApprovalResponse, tool_grant::ApprovalDecision};
use std::sync::Arc;

pub async fn answer_local(
    store: &Arc<dyn HostStore>,
    bot_id: &str,
    turn_id: &str,
    approval_id: &str,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let card = store
        .approval_card(approval_id)
        .await?
        .filter(|card| card.turn_id == turn_id)
        .ok_or_else(|| format!("Turn '{turn_id}' waits for no approval '{approval_id}'"))?;
    let grants = StoreGrants {
        store: store.clone(),
        bot_id: bot_id.to_owned(),
    };
    record_card_answer(&grants, &card, decision)
        .await
        .map_err(|error| error.to_string())?;
    let response = ApprovalResponse {
        approval_id: approval_id.to_owned(),
        decision,
    };
    if store.resolve_parked(bot_id, turn_id, &response).await? {
        Ok(())
    } else {
        Err(format!("Turn '{turn_id}' is not waiting for an approval"))
    }
}
