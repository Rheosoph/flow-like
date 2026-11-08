//! Sending as a bot: a host answers the turn it claimed, and the reply and the turn's completion
//! commit together under the conversation's lock. A desktop host sends through
//! `POST …/entries?as=<bot>` with `turn { id, host_id }`; the server's worker calls
//! [`reply_as_bot`] in-process.

use super::{
    BOT_TURN_NOT_CLAIMED,
    entries::{Outgoing, Sent, fan_out, repeat_of},
    lock_conversation,
    model::TurnClaim,
};
use crate::{
    entity::{bot, bot_turn},
    error::ApiError,
    routes::bots::{model::hands_off_to_desktop, owned_bot},
};
use axum::http::StatusCode;
use chrono::{DateTime, FixedOffset};
use flow_like_types::{bot::BotLocation, inbox::InboxEntryKind};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, Statement};

pub(crate) const TURN_RUNNING: &str = "RUNNING";
pub(crate) const TURN_DONE: &str = "DONE";
pub(crate) const TURN_QUEUED: &str = "QUEUED";

const COMPLETE_INTERACTIVE_SQL: &str = r#"UPDATE "BotTurn" SET "status" = $1, "replyMessageId" = $2, "leaseUntil" = NULL, "updatedAt" = $3 WHERE "id" = $4 AND "botId" = $5 AND "conversationId" = $6 AND "status" = $7 AND ("hostId" IS NULL OR "hostId" = $8) RETURNING "hops""#;

const COMPLETE_TURN_SQL: &str = r#"UPDATE "BotTurn" SET "status" = $1, "replyMessageId" = $2, "leaseUntil" = NULL, "updatedAt" = $3 WHERE "id" = $4 AND "botId" = $5 AND "conversationId" = $6 AND "status" = $7 AND "hostId" = $8 AND "leaseUntil" > $9 RETURNING "hops""#;

fn clipped(value: &str) -> String {
    value.chars().take(80).collect()
}

fn not_claimed(bot_id: &str, claim: &TurnClaim, reason: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        BOT_TURN_NOT_CLAIMED,
        format!(
            "Bot '{bot_id}' can't answer turn '{}' as host '{}': {reason}",
            clipped(&claim.id),
            clipped(&claim.host_id)
        ),
    )
}

/// One of the caller's bots whose turns their desktop answers: only its desktop host sends as it
/// here. That is a desktop bot, or a server bot that hands off to the desktop; the fenced
/// completion still requires a turn this host claimed. Other server bots reply in-process and
/// device bots through their device.
pub(super) async fn desktop_bot<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    bot_id: &str,
) -> Result<bot::Model, ApiError> {
    let bot = owned_bot(db, sub, bot_id).await?;
    let host = match BotLocation::parse(&bot.location).unwrap_or_default() {
        BotLocation::Desktop => return Ok(bot),
        BotLocation::Server if hands_off_to_desktop(&bot) => return Ok(bot),
        BotLocation::Server => "the server, which answers its turns itself",
        BotLocation::Device => "a device, which answers its turns itself",
    };
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        BOT_TURN_NOT_CLAIMED,
        format!("Bot '{bot_id}' runs on {host}; only a desktop bot's host sends as it here"),
    ))
}

/// Completes the claimed turn with `reply_message_id` when it is RUNNING for this bot in this
/// conversation, held by `claim.host_id` with a lease that hasn't run out, and returns the turn's
/// hops; anything else is [`BOT_TURN_NOT_CLAIMED`]. Call it in the transaction that writes the
/// reply, under the conversation's lock.
pub(crate) async fn complete_turn_in<C: ConnectionTrait>(
    db: &C,
    claim: &TurnClaim,
    bot_id: &str,
    conversation_id: &str,
    reply_message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u32, ApiError> {
    let completed = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            COMPLETE_TURN_SQL,
            [
                TURN_DONE.into(),
                reply_message_id.into(),
                now.into(),
                claim.id.as_str().into(),
                bot_id.into(),
                conversation_id.into(),
                TURN_RUNNING.into(),
                claim.host_id.as_str().into(),
                now.into(),
            ],
        ))
        .await?;
    match completed {
        Some(row) => Ok(u32::try_from(row.try_get::<i32>("", "hops")?).unwrap_or(0)),
        None => Err(why_not_claimed(db, claim, bot_id, conversation_id).await?),
    }
}

/// Why the fenced completion matched no turn, without telling whether another bot's turn exists.
async fn why_not_claimed<C: ConnectionTrait>(
    db: &C,
    claim: &TurnClaim,
    bot_id: &str,
    conversation_id: &str,
) -> Result<ApiError, DbErr> {
    let turn = bot_turn::Entity::find_by_id(claim.id.as_str())
        .one(db)
        .await?
        .filter(|turn| turn.bot_id == bot_id && turn.conversation_id == conversation_id);
    let reason = match turn {
        None => "it is not this bot's turn in this conversation".to_string(),
        Some(turn) if turn.status != TURN_RUNNING => {
            format!("it is {}, not running", turn.status.to_ascii_lowercase())
        }
        Some(turn) if turn.host_id.as_deref() != Some(claim.host_id.as_str()) => {
            "another host claimed it".to_string()
        }
        Some(_) => "its lease ran out; claim it again".to_string(),
    };
    Ok(not_claimed(bot_id, claim, &reason))
}

/// The bot's reply for the turn its host claimed: takes the conversation's lock, then
/// [`reply_in`]. The entry point for hosts that reply in-process.
pub(crate) async fn reply_as_bot(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    reply_in(txn, conversation_id, bot_id, claim, outgoing, now).await
}

/// Completes the claimed turn and sends its reply, so both commit together; a repeat with the
/// same message id returns the first copy. The caller holds the conversation's lock and checked
/// that it may act for the bot.
pub(crate) async fn reply_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    if outgoing.kind != InboxEntryKind::Message {
        return Err(ApiError::bad_request(format!(
            "A bot answers its turn with a message, not a {}",
            outgoing.kind.as_str()
        )));
    }
    if let Some(sent) = repeat_of(txn, conversation_id, bot_id, &outgoing.message_id).await? {
        return Ok(sent);
    }
    let hops = complete_turn_in(
        txn,
        claim,
        bot_id,
        conversation_id,
        &outgoing.message_id,
        now,
    )
    .await?;
    fan_out(
        txn,
        conversation_id,
        bot_id,
        hops.saturating_add(1),
        outgoing,
        now,
    )
    .await
}

/// Completes the turn an interactive chat holds: it is still queued, so no host claimed it. Any
/// other state is [`BOT_TURN_NOT_CLAIMED`]; the chat then writes no reply of its own.
pub(crate) async fn complete_interactive_in<C: ConnectionTrait>(
    db: &C,
    claim: &TurnClaim,
    bot_id: &str,
    conversation_id: &str,
    reply_message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u32, ApiError> {
    let completed = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            COMPLETE_INTERACTIVE_SQL,
            [
                TURN_DONE.into(),
                reply_message_id.into(),
                now.into(),
                claim.id.as_str().into(),
                bot_id.into(),
                conversation_id.into(),
                TURN_QUEUED.into(),
                claim.host_id.as_str().into(),
            ],
        ))
        .await?;
    match completed {
        Some(row) => Ok(u32::try_from(row.try_get::<i32>("", "hops")?).unwrap_or(0)),
        None => Err(not_claimed(
            bot_id,
            claim,
            "a host answers it or it is finished; the chat's reply is not stored",
        )),
    }
}

/// An interactive chat's reply to the message it sent: [`reply_as_bot`] for a turn that was
/// held, not claimed. The bot's owner must have checked the caller.
pub(crate) async fn reply_interactive_as_bot(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    reply_interactive_in(txn, conversation_id, bot_id, claim, outgoing, now).await
}

/// [`reply_interactive_as_bot`] under the lock the caller holds.
pub(crate) async fn reply_interactive_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    if outgoing.kind != InboxEntryKind::Message {
        return Err(ApiError::bad_request(format!(
            "A bot answers its turn with a message, not a {}",
            outgoing.kind.as_str()
        )));
    }
    if let Some(sent) = repeat_of(txn, conversation_id, bot_id, &outgoing.message_id).await? {
        return Ok(sent);
    }
    let hops = complete_interactive_in(
        txn,
        claim,
        bot_id,
        conversation_id,
        &outgoing.message_id,
        now,
    )
    .await?;
    fan_out(
        txn,
        conversation_id,
        bot_id,
        hops.saturating_add(1),
        outgoing,
        now,
    )
    .await
}
