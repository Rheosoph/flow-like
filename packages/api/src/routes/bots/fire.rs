//! Waking a bot with a `trigger` entry from outside a chat (webhooks, flows) and waiting for the
//! turn it queues. The entry goes into the owner's direct conversation with the bot, which queues
//! the turn in the same transaction.

use crate::{
    bot_host,
    entity::{bot_turn, inbox_entry},
    error::ApiError,
    routes::inbox::{now_millis, triggers::post_trigger_in, wake},
    state::AppState,
};
use axum::http::StatusCode;
use flow_like_types::{
    bot_turn::{
        ERROR_BATCHED, STORED_CANCELLED, STORED_DONE, STORED_FAILED, STORED_QUEUED, STORED_RUNNING,
        STORED_WAITING, turn_id,
    },
    inbox::TriggerBody,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

pub(crate) const BOT_TURN_NOT_QUEUED: &str = "BOT_TURN_NOT_QUEUED";
pub(crate) const BOT_CONVERSATION_ENCRYPTED: &str = "BOT_CONVERSATION_ENCRYPTED";
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// The bot's answer to a turn, as plain text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TurnReply {
    pub message_id: String,
    pub text: String,
}

/// Where a woken bot's turn stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct FiredTurn {
    pub turn_id: String,
    /// `queued`, `running`, `waiting` (for the owner's approval), `done`, `failed` or `cancelled`.
    pub status: String,
    /// The bot's reply once the turn is done and the caller waited for it.
    pub reply: Option<TurnReply>,
    /// Why the turn failed or was cancelled.
    pub error_code: Option<String>,
}

fn wire_status(stored: &str) -> &'static str {
    match stored {
        STORED_QUEUED => "queued",
        STORED_RUNNING => "running",
        STORED_WAITING => "waiting",
        STORED_DONE => "done",
        STORED_FAILED => "failed",
        STORED_CANCELLED => "cancelled",
        _ => "queued",
    }
}

/// Writes `body` as a trigger with `message_id` (a repeat returns the first turn) and returns the
/// id of the turn it queued.
pub(crate) async fn fire_trigger(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    message_id: &str,
    body: &TriggerBody,
) -> Result<String, ApiError> {
    let sent = state
        .transaction(|txn| {
            let (owner, bot, message, body) = (
                owner_id.to_string(),
                bot_id.to_string(),
                message_id.to_string(),
                body.clone(),
            );
            Box::pin(async move {
                post_trigger_in(txn, &owner, &bot, &message, &body, now_millis()).await
            })
        })
        .await?;
    let Some(sent) = sent else {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_CONVERSATION_ENCRYPTED,
            "This bot's conversation with you is end-to-end encrypted, so the server can't write into it",
        ));
    };
    wake(state, sent.wake).await;
    bot_host::dispatch(state, &sent.bots).await;
    let id = turn_id(bot_id, message_id);
    if turn_row(&state.db, &id).await?.is_none() {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_NOT_QUEUED,
            format!("Bot '{bot_id}' took no turn for this call; it may be at its turn limit"),
        ));
    }
    Ok(id)
}

async fn turn_row<C: ConnectionTrait>(
    db: &C,
    id: &str,
) -> Result<Option<bot_turn::Model>, ApiError> {
    Ok(bot_turn::Entity::find_by_id(id).one(db).await?)
}

async fn reply_text<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    message_id: &str,
) -> Result<Option<TurnReply>, ApiError> {
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(owner_id))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .one(db)
        .await?;
    Ok(entry.map(|entry| TurnReply {
        message_id: entry.message_id,
        text: entry
            .envelope
            .pointer("/body/text")
            .and_then(|text| text.as_str())
            .unwrap_or_default()
            .to_string(),
    }))
}

/// The turn's state now, with the reply when it has one.
pub(crate) async fn turn_state<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    id: &str,
) -> Result<FiredTurn, ApiError> {
    let turn = turn_row(db, id)
        .await?
        .filter(|turn| turn.owner_id == owner_id);
    let Some(turn) = turn else {
        return Ok(FiredTurn {
            turn_id: id.to_string(),
            status: "queued".into(),
            reply: None,
            error_code: None,
        });
    };
    let reply = match (&turn.reply_message_id, turn.status.as_str()) {
        (Some(reply_id), STORED_DONE | STORED_FAILED | STORED_CANCELLED) => {
            reply_text(db, owner_id, reply_id).await?
        }
        _ => None,
    };
    Ok(FiredTurn {
        turn_id: turn.id,
        status: wire_status(&turn.status).into(),
        reply,
        error_code: turn.error_code,
    })
}

fn settled(state: &FiredTurn) -> bool {
    match state.status.as_str() {
        "done" => state.reply.is_some(),
        "failed" => true,
        "cancelled" => state.error_code.as_deref() != Some(ERROR_BATCHED) || state.reply.is_some(),
        _ => false,
    }
}

/// Polls the turn until its reply is in or `timeout` passes, and returns the last state.
pub(crate) async fn wait_for_turn(
    db: &impl ConnectionTrait,
    owner_id: &str,
    id: &str,
    timeout: Duration,
) -> Result<FiredTurn, ApiError> {
    let started = Instant::now();
    loop {
        let state = turn_state(db, owner_id, id).await?;
        if settled(&state) || started.elapsed() >= timeout {
            return Ok(state);
        }
        flow_like_types::tokio::time::sleep(POLL_INTERVAL).await;
    }
}
