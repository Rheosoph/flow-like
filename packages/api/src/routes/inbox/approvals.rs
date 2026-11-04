//! The inbox side of parked turns: where an approval card goes, and posting the card, the
//! owner's answer and the notice of an abandoned wait. The card goes into the conversation the
//! owner and the bot share directly (the turn's own when that is it, else the pair's direct
//! conversation, started when missing). An answer never wakes the bot as a new turn: the parked
//! turn itself is queued again.

use super::{
    conversations::start_direct,
    entries::{Sent, fan_out, repeat_of, validate_send},
    lock_conversation,
    members::Principal,
    model::SendEntryRequest,
};
use crate::{
    entity::{conversation, inbox_entry},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_message::{ApprovalRequest, ApprovalResponse},
    inbox::{
        ConversationEncryption, InboxEntryKind, PrincipalKind, direct_pair_key, plain_envelope,
    },
    tool_grant::approval_response_message_id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

pub(crate) fn is_approval_response(envelope: &Value) -> bool {
    envelope
        .get("body")
        .and_then(|body| body.get("approval_response"))
        .is_some_and(|response| !response.is_null())
}

fn pair_key(owner_id: &str, bot_id: &str) -> String {
    direct_pair_key(owner_id, bot_id)
}

/// The conversation an approval card of this bot goes to, started when missing. `None` when the
/// pair's conversation is end-to-end encrypted: the server cannot write into it.
pub(crate) async fn open_conversation<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_conversation_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Option<String>, ApiError> {
    let key = pair_key(owner_id, bot_id);
    let turn_conversation = conversation::Entity::find_by_id(turn_conversation_id)
        .one(db)
        .await?;
    let id = match turn_conversation {
        Some(row) if row.direct_key.as_deref() == Some(key.as_str()) => row.id,
        _ => {
            let bot = Principal {
                id: bot_id.to_string(),
                kind: PrincipalKind::Bot,
            };
            start_direct(db, owner_id, &bot, ConversationEncryption::Plain, now).await?
        }
    };
    let row = conversation::Entity::find_by_id(&id).one(db).await?;
    let plain = row.is_some_and(|row| {
        ConversationEncryption::parse(&row.encryption) != Some(ConversationEncryption::E2e)
    });
    Ok(plain.then_some(id))
}

/// Whether `conversation_id` is the direct conversation of `owner_id` and `bot_id`.
pub(crate) async fn is_owner_chat<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    conversation_id: &str,
) -> Result<bool, ApiError> {
    let key = pair_key(owner_id, bot_id);
    let row = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?;
    Ok(row.is_some_and(|row| row.direct_key.as_deref() == Some(key.as_str())))
}

fn message(message_id: String, body: Value) -> SendEntryRequest {
    SendEntryRequest {
        message_id,
        kind: InboxEntryKind::Message,
        envelope: Some(plain_envelope(body)),
        envelopes: None,
        turn: None,
    }
}

/// Posts the card as the bot, in the transaction that parks the turn. A repeat returns the first
/// copy.
pub(crate) async fn post_card_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    hops: u32,
    card: &ApprovalRequest,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    if let Some(sent) = repeat_of(txn, conversation_id, bot_id, &card.id).await? {
        return Ok(sent);
    }
    let body = json!({
        "text": format!("I need your approval: {}", card.summary),
        "hops": hops,
        "approval": card,
    });
    let outgoing = validate_send(message(card.id.clone(), body))?;
    fan_out(txn, conversation_id, bot_id, hops, &outgoing, now).await
}

/// Posts the owner's answer to a card; it replies to the card and wakes nobody.
pub(crate) async fn post_answer_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    owner_id: &str,
    response: &ApprovalResponse,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let message_id = approval_response_message_id(&response.approval_id);
    if let Some(sent) = repeat_of(txn, conversation_id, owner_id, &message_id).await? {
        return Ok(sent);
    }
    let body = json!({
        "text": "",
        "reply_to": response.approval_id,
        "approval_response": response,
    });
    let outgoing = validate_send(message(message_id, body))?;
    fan_out(txn, conversation_id, owner_id, 0, &outgoing, now).await
}

/// Posts a plain note as the bot, e.g. that a wait ended unanswered.
pub(crate) async fn post_notice_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    message_id: &str,
    text: &str,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    if let Some(sent) = repeat_of(txn, conversation_id, bot_id, message_id).await? {
        return Ok(sent);
    }
    let outgoing = validate_send(message(
        message_id.to_string(),
        json!({ "text": text, "hops": 0 }),
    ))?;
    fan_out(txn, conversation_id, bot_id, 0, &outgoing, now).await
}

/// Serializes the writes of one conversation; call first in the transaction.
pub(crate) async fn lock(
    txn: &sea_orm::DatabaseTransaction,
    conversation_id: &str,
) -> Result<(), sea_orm::DbErr> {
    lock_conversation(txn, conversation_id).await
}

/// The card with this id in the owner's copy of the conversation.
pub(crate) async fn card_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    conversation_id: &str,
    approval_id: &str,
) -> Result<Option<ApprovalRequest>, ApiError> {
    let row = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(owner_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(approval_id))
        .one(db)
        .await?;
    Ok(row
        .and_then(|row| row.envelope.get("body")?.get("approval").cloned())
        .and_then(|card| serde_json::from_value(card).ok()))
}
