//! Bots talking to each other: a bot posts into the exchange it shares with another bot as
//! itself, without a claimed turn of its own (the asker is mid-turn or in a chat with its owner),
//! and a late answer is posted into the asker's own conversation while its answered turn
//! completes.

use super::{
    conversations::start_exchange,
    entries::{Outgoing, Sent, fan_out, repeat_of, validate_send},
    lock_conversation,
    model::SendEntryRequest,
    now_millis, wake,
};
use crate::{
    db::coordination::coordinate,
    error::ApiError,
    state::AppState,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{InboxEntryKind, plain_envelope};
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use serde_json::Value;

pub(crate) fn message_outgoing(message_id: &str, body: Value) -> Result<Outgoing, ApiError> {
    validate_send(SendEntryRequest {
        message_id: message_id.to_string(),
        kind: InboxEntryKind::Message,
        envelope: Some(plain_envelope(body)),
        envelopes: None,
        turn: None,
    })
}

/// The pair's exchange, started on first use.
pub(crate) async fn open_exchange(
    state: &AppState,
    owner_id: &str,
    first_bot: &str,
    second_bot: &str,
    title: Option<String>,
) -> Result<String, ApiError> {
    state
        .transaction(|txn| {
            let (owner, first, second, title) = (
                owner_id.to_string(),
                first_bot.to_string(),
                second_bot.to_string(),
                title.clone(),
            );
            Box::pin(async move {
                let (low, high) = if first <= second {
                    (first.as_str(), second.as_str())
                } else {
                    (second.as_str(), first.as_str())
                };
                coordinate(txn, "bot-exchange", &[low, high]).await?;
                start_exchange(txn, &owner, &first, &second, title, now_millis()).await
            })
        })
        .await
}

/// `bot_id`'s message in the conversation it belongs to; a repeat of `message_id` returns the
/// first copy. The caller holds the conversation's lock. `hops` counts the bot turns behind it.
pub(crate) async fn post_as_bot_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    bot_id: &str,
    hops: u32,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    match repeat_of(db, conversation_id, bot_id, &outgoing.message_id).await? {
        Some(sent) => Ok(sent),
        None => fan_out(db, conversation_id, bot_id, hops, outgoing, now).await,
    }
}

async fn post_locked(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    bot_id: &str,
    hops: u32,
    outgoing: &Outgoing,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    post_as_bot_in(txn, conversation_id, bot_id, hops, outgoing, now_millis()).await
}

/// Posts as the bot under the conversation's lock, wakes the people it reaches and starts the
/// turns it queued.
pub(crate) async fn post_as_bot(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    hops: u32,
    outgoing: Outgoing,
) -> Result<Sent, ApiError> {
    let outgoing = std::sync::Arc::new(outgoing);
    let sent = state
        .transaction(|txn| {
            let (conversation_id, bot_id, outgoing) = (
                conversation_id.to_string(),
                bot_id.to_string(),
                outgoing.clone(),
            );
            Box::pin(async move {
                post_locked(txn, &conversation_id, &bot_id, hops, &outgoing).await
            })
        })
        .await?;
    wake(state, sent.wake.clone()).await;
    crate::bot_host::dispatch(state, &sent.bots).await;
    Ok(sent)
}

/// Completes the asker's turn that was woken by a late answer and posts its reply into
/// `origin_conversation` instead of the exchange the turn lives in. Both conversations are
/// locked in id order.
pub(crate) async fn reply_in_origin(
    state: &AppState,
    turn: &crate::entity::bot_turn::Model,
    host_id: &str,
    origin_conversation: &str,
    outgoing: Outgoing,
) -> Result<Sent, ApiError> {
    let outgoing = std::sync::Arc::new(outgoing);
    let claim = super::model::TurnClaim {
        id: turn.id.clone(),
        host_id: host_id.to_string(),
    };
    let sent = state
        .transaction(|txn| {
            let (turn, claim, origin, outgoing) = (
                turn.clone(),
                claim.clone(),
                origin_conversation.to_string(),
                outgoing.clone(),
            );
            Box::pin(async move {
                let mut locks = [turn.conversation_id.as_str(), origin.as_str()];
                locks.sort_unstable();
                for conversation_id in locks {
                    lock_conversation(txn, conversation_id).await?;
                }
                let now = now_millis();
                if let Some(sent) =
                    repeat_of(txn, &origin, &turn.bot_id, &outgoing.message_id).await?
                {
                    return Ok(sent);
                }
                let hops = super::turns::complete_turn_in(
                    txn,
                    &claim,
                    &turn.bot_id,
                    &turn.conversation_id,
                    &outgoing.message_id,
                    now,
                )
                .await?;
                fan_out(txn, &origin, &turn.bot_id, hops.saturating_add(1), &outgoing, now).await
            })
        })
        .await?;
    wake(state, sent.wake.clone()).await;
    crate::bot_host::dispatch(state, &sent.bots).await;
    Ok(sent)
}
