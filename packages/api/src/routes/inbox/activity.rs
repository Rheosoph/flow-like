//! Writing an `activity` entry: a line in a bot's chat about something it did elsewhere, e.g. a
//! question to another bot. The host writes it as the bot; it wakes no bot, moves no unread
//! count and stays out of transcripts.

use super::{
    approvals::lock as lock_conversation,
    entries::{Content, Links, Outgoing, Sent, fan_out, plain_links, repeat_of},
    now_millis, wake,
};
use crate::{entity::conversation, error::ApiError, state::AppState};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{
    ActivityBody, ConversationEncryption, InboxEntryKind, direct_pair_key,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter};

pub(crate) fn activity_outgoing(
    message_id: &str,
    body: &ActivityBody,
) -> Result<Outgoing, ApiError> {
    let envelope = body.envelope();
    let links: Links = plain_links(InboxEntryKind::Activity, &envelope)?;
    Ok(Outgoing {
        message_id: message_id.to_string(),
        kind: InboxEntryKind::Activity,
        content: Content::Plain(envelope),
        links,
    })
}

/// The id of the activity an ask leaves in `conversation_id`: the same ask writes it once.
pub(crate) fn activity_message_id(ask_id: &str, conversation_id: &str) -> String {
    let digest = blake3::hash(format!("{ask_id}\0{conversation_id}").as_bytes()).to_hex();
    format!("act-{}", &digest[..32])
}

/// Writes the activity into `conversation_id` as `sender_id`. The caller holds the
/// conversation's lock. A repeat of `message_id` returns the first copy.
pub(crate) async fn write_activity_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender_id: &str,
    message_id: &str,
    body: &ActivityBody,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let outgoing = activity_outgoing(message_id, body)?;
    if let Some(sent) = repeat_of(txn, conversation_id, sender_id, message_id).await? {
        return Ok(sent);
    }
    fan_out(txn, conversation_id, sender_id, 0, &outgoing, now).await
}

fn is_plain(row: &conversation::Model) -> bool {
    ConversationEncryption::parse(&row.encryption) != Some(ConversationEncryption::E2e)
}

/// The owner's existing, readable direct conversation with the bot; none is started.
async fn existing_chat<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Option<String>, ApiError> {
    let row = conversation::Entity::find()
        .filter(conversation::Column::DirectKey.eq(direct_pair_key(owner_id, bot_id)))
        .one(db)
        .await?;
    Ok(row.filter(is_plain).map(|row| row.id))
}

async fn is_plain_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<bool, ApiError> {
    let row = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?;
    Ok(row.as_ref().is_some_and(is_plain))
}

async fn post_locked(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    sender_id: &str,
    body: &ActivityBody,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    let message_id = match body {
        ActivityBody::BotAsk { ask_id, .. } => activity_message_id(ask_id, conversation_id),
    };
    write_activity_in(
        txn,
        conversation_id,
        sender_id,
        &message_id,
        body,
        now_millis(),
    )
    .await
}

async fn post_activity(
    state: &AppState,
    conversation_id: &str,
    sender_id: &str,
    body: &ActivityBody,
) -> Result<(), ApiError> {
    let sent = state
        .transaction(|txn| {
            let (conversation_id, sender_id, body) = (
                conversation_id.to_string(),
                sender_id.to_string(),
                body.clone(),
            );
            Box::pin(async move { post_locked(txn, &conversation_id, &sender_id, &body).await })
        })
        .await?;
    wake(state, sent.wake).await;
    Ok(())
}

/// Where a question leaves its activity, each with the bot that writes it: the asker's chat it
/// was asked from and the callee's chat with the owner, when that exists. A chat the server
/// cannot read is left out.
pub(crate) async fn bot_ask_targets<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    origin_conversation: &str,
    body: &ActivityBody,
) -> Result<Vec<(String, String)>, ApiError> {
    let ActivityBody::BotAsk {
        asker_id,
        callee_id,
        ..
    } = body;
    let mut targets = Vec::new();
    if is_plain_conversation(db, origin_conversation).await? {
        targets.push((origin_conversation.to_string(), asker_id.clone()));
    }
    if let Some(chat) = existing_chat(db, owner_id, callee_id)
        .await?
        .filter(|chat| chat != origin_conversation)
    {
        targets.push((chat, callee_id.clone()));
    }
    Ok(targets)
}

/// Records the question in the chats [`bot_ask_targets`] names. Never fails the ask: a write that
/// fails is logged.
pub(crate) async fn record_bot_ask(
    state: &AppState,
    owner_id: &str,
    origin_conversation: &str,
    body: &ActivityBody,
) {
    let targets = match bot_ask_targets(&state.db, owner_id, origin_conversation, body).await {
        Ok(targets) => targets,
        Err(error) => {
            tracing::warn!(%error, %origin_conversation, "Bot activity: the ask's chats were not looked up");
            return;
        }
    };
    for (conversation_id, sender_id) in targets {
        if let Err(error) = post_activity(state, &conversation_id, &sender_id, body).await {
            tracing::warn!(%error, %conversation_id, %sender_id, "Bot activity: the ask was not recorded");
        }
    }
}
