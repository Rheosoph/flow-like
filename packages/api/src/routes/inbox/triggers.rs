//! Writing a `trigger` entry: a host, never a client, wakes a bot with work (a schedule firing,
//! a webhook, an email). The entry goes into the owner's direct conversation with the bot, from
//! the owner and with 0 hops, so the bot answers it like a message of its owner. A task scheduled
//! in a group or someone else's chat fires there instead, from the person who asked and naming
//! the bot (`bot_id`), so only that bot wakes.

use super::{
    approvals::open_conversation,
    entries::{Content, Links, Outgoing, Sent, fan_out, plain_links, repeat_of},
    lock_conversation,
};
use crate::error::ApiError;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{InboxEntryKind, TriggerBody};
use sea_orm::{ConnectionTrait, DatabaseTransaction};

pub(crate) fn trigger_outgoing(message_id: &str, body: &TriggerBody) -> Result<Outgoing, ApiError> {
    let envelope = body.envelope();
    let links: Links = plain_links(InboxEntryKind::Trigger, &envelope)?;
    Ok(Outgoing {
        message_id: message_id.to_string(),
        kind: InboxEntryKind::Trigger,
        content: Content::Plain(envelope),
        links,
    })
}

/// The owner's direct conversation with the bot, started when missing. `None` when it is
/// end-to-end encrypted: the server cannot write into it.
pub(crate) async fn trigger_conversation<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Option<String>, ApiError> {
    open_conversation(db, owner_id, bot_id, "", now).await
}

/// Takes the conversation's lock for the rest of the transaction.
pub(crate) async fn lock_trigger_conversation(
    txn: &DatabaseTransaction,
    conversation_id: &str,
) -> Result<(), ApiError> {
    Ok(lock_conversation(txn, conversation_id).await?)
}

/// Writes the trigger into `conversation_id` as `sender`, a person in it, with 0 hops. The
/// caller holds the conversation's lock. A repeat of `message_id` returns the first copy.
pub(crate) async fn write_trigger_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    message_id: &str,
    body: &TriggerBody,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let outgoing = trigger_outgoing(message_id, body)?;
    if let Some(sent) = repeat_of(txn, conversation_id, sender, message_id).await? {
        return Ok(sent);
    }
    fan_out(txn, conversation_id, sender, 0, &outgoing, now).await
}

/// Writes the trigger into the owner's direct conversation with the bot, under the
/// conversation's lock. `None` when that conversation is end-to-end encrypted.
pub(crate) async fn post_trigger_in(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
    message_id: &str,
    body: &TriggerBody,
    now: DateTime<FixedOffset>,
) -> Result<Option<Sent>, ApiError> {
    let Some(conversation_id) = trigger_conversation(txn, owner_id, bot_id, now).await? else {
        return Ok(None);
    };
    lock_conversation(txn, &conversation_id).await?;
    write_trigger_in(txn, &conversation_id, owner_id, message_id, body, now)
        .await
        .map(Some)
}
