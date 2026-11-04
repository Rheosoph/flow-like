//! Pins: a person pins conversations to the top of their own list, and any active person member
//! pins messages to a conversation for everyone in it. Pinning again changes nothing; at most
//! [`MAX_PINNED_MESSAGES`] messages stay pinned per conversation.

use super::{
    INBOX_PIN_LIMIT, Reader, active_members, active_membership, conversation_not_found,
    delivery::people_to_wake,
    marks::{copies_of, latest_edits, live_copy},
    members::Principal,
    model::{InboxEntry, ReaderQuery},
    now_millis, reader, reader_membership, wake,
};
use crate::{
    entity::{inbox_entry, pinned_conversation, pinned_message},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::is_tombstone;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The most messages one conversation keeps pinned.
pub(crate) const MAX_PINNED_MESSAGES: u64 = 50;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetPinRequest {
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ConversationPinState {
    pub conversation_id: String,
    /// When you pinned it; null when it isn't pinned.
    pub pinned_at: Option<DateTime<FixedOffset>>,
}

/// A message pinned to a conversation for every member.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct PinnedMessage {
    pub message_id: String,
    pub pinned_by: String,
    pub pinned_at: DateTime<FixedOffset>,
    /// Your copy of the message; null when you joined after it was sent.
    pub entry: Option<InboxEntry>,
    /// The sender's newest edit of it in your copy.
    pub latest_edit: Option<InboxEntry>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PinnedMessageList {
    /// Newest pin first, at most 50.
    pub pins: Vec<PinnedMessage>,
}

pub(crate) async fn set_pinned_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    pinned: bool,
    now: DateTime<FixedOffset>,
) -> Result<ConversationPinState, ApiError> {
    active_membership(db, conversation_id, sub).await?;
    let pinned_at = if pinned {
        pinned_conversation::Entity::insert(pinned_conversation::ActiveModel {
            principal_id: Set(sub.to_string()),
            conversation_id: Set(conversation_id.to_string()),
            pinned_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([
                pinned_conversation::Column::PrincipalId,
                pinned_conversation::Column::ConversationId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
        pinned_conversation::Entity::find_by_id((sub.to_string(), conversation_id.to_string()))
            .one(db)
            .await?
            .map(|row| row.pinned_at)
    } else {
        pinned_conversation::Entity::delete_by_id((sub.to_string(), conversation_id.to_string()))
            .exec(db)
            .await?;
        None
    };
    Ok(ConversationPinState {
        conversation_id: conversation_id.to_string(),
        pinned_at,
    })
}

/// The conversation's pins as `reader` sees them: their own copy of each message, deleted ones
/// left out.
pub(super) async fn pins_for<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &Reader,
) -> Result<Vec<PinnedMessage>, ApiError> {
    reader_membership(db, conversation_id, reader).await?;
    let pins = pinned_message::Entity::find()
        .filter(pinned_message::Column::ConversationId.eq(conversation_id))
        .order_by_desc(pinned_message::Column::PinnedAt)
        .order_by_desc(pinned_message::Column::MessageId)
        .all(db)
        .await?;
    present(db, &reader.inbox, conversation_id, pins)
        .await
        .map_err(Into::into)
}

async fn present<C: ConnectionTrait>(
    db: &C,
    inbox: &str,
    conversation_id: &str,
    pins: Vec<pinned_message::Model>,
) -> Result<Vec<PinnedMessage>, DbErr> {
    let ids: Vec<String> = pins.iter().map(|pin| pin.message_id.clone()).collect();
    let mut copies: HashMap<String, inbox_entry::Model> = copies_of(db, inbox, &ids)
        .await?
        .into_iter()
        .filter(|copy| copy.conversation_id == conversation_id)
        .map(|copy| (copy.message_id.clone(), copy))
        .collect();
    let pins: Vec<pinned_message::Model> = pins
        .into_iter()
        .filter(|pin| {
            copies
                .get(&pin.message_id)
                .is_none_or(|copy| !is_tombstone(&copy.envelope))
        })
        .collect();
    let shown: Vec<&inbox_entry::Model> = pins
        .iter()
        .filter_map(|pin| copies.get(&pin.message_id))
        .collect();
    let mut edits = latest_edits(db, inbox, &shown).await?;
    Ok(pins
        .into_iter()
        .map(|pin| PinnedMessage {
            entry: copies.remove(&pin.message_id).map(Into::into),
            latest_edit: edits.remove(&pin.message_id),
            message_id: pin.message_id,
            pinned_by: pin.pinned_by,
            pinned_at: pin.pinned_at,
        })
        .collect())
}

fn pin_limit() -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_PIN_LIMIT,
        format!(
            "This conversation already has {MAX_PINNED_MESSAGES} pinned messages, the most it keeps; unpin one first"
        ),
    )
}

async fn find_pin<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    message_id: &str,
) -> Result<Option<pinned_message::Model>, DbErr> {
    pinned_message::Entity::find_by_id((conversation_id.to_string(), message_id.to_string()))
        .one(db)
        .await
}

/// Adds the pin unless the conversation is full; a concurrent pin of the same message wins.
async fn insert_pin<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<pinned_message::Model, ApiError> {
    let pinned = pinned_message::Entity::find()
        .filter(pinned_message::Column::ConversationId.eq(conversation_id))
        .count(db)
        .await?;
    if pinned >= MAX_PINNED_MESSAGES {
        return Err(pin_limit());
    }
    pinned_message::Entity::insert(pinned_message::ActiveModel {
        conversation_id: Set(conversation_id.to_string()),
        message_id: Set(message_id.to_string()),
        pinned_by: Set(sub.to_string()),
        pinned_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            pinned_message::Column::ConversationId,
            pinned_message::Column::MessageId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    find_pin(db, conversation_id, message_id)
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))
}

pub(crate) async fn pin_message_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    message_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<PinnedMessage, ApiError> {
    active_membership(db, conversation_id, sub).await?;
    let copy = live_copy(db, conversation_id, sub, message_id).await?;
    let pin = match find_pin(db, conversation_id, message_id).await? {
        Some(pin) => pin,
        None => insert_pin(db, conversation_id, sub, message_id, now).await?,
    };
    let latest_edit = latest_edits(db, sub, &[&copy]).await?.remove(message_id);
    Ok(PinnedMessage {
        message_id: pin.message_id,
        pinned_by: pin.pinned_by,
        pinned_at: pin.pinned_at,
        entry: Some(copy.into()),
        latest_edit,
    })
}

pub(crate) async fn unpin_message_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    message_id: &str,
) -> Result<bool, ApiError> {
    active_membership(db, conversation_id, sub).await?;
    let removed =
        pinned_message::Entity::delete_by_id((conversation_id.to_string(), message_id.to_string()))
            .exec(db)
            .await?
            .rows_affected;
    Ok(removed > 0)
}

/// Everyone whose view of the conversation's pins changed: its people and the owners of its bots.
async fn wake_members(state: &AppState, conversation_id: &str) {
    let woken = async {
        let members: Vec<Principal> = active_members(&state.db, &[conversation_id.to_string()])
            .await?
            .iter()
            .map(Principal::from)
            .collect();
        people_to_wake(&state.db, &members).await
    };
    match woken.await {
        Ok(people) => wake(state, people).await,
        Err(error) => {
            tracing::warn!(%error, %conversation_id, "Failed to find who to tell about a pin change");
        }
    }
}

#[utoipa::path(
    patch,
    path = "/inbox/conversations/{conversation_id}/pin",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation")),
    request_body = SetPinRequest,
    responses(
        (status = 200, description = "Whether the conversation is pinned for you: pinned conversations list first, newest pin first. Only you see your pins.", body = ConversationPinState),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people pin conversations, not bots, API keys or running flows"),
        (status = 404, description = "You are not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /inbox/conversations/{conversation_id}/pin",
    skip(state, user, body)
)]
pub async fn set_pin(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<SetPinRequest>,
) -> Result<Json<ConversationPinState>, ApiError> {
    let sub = user.sub()?;
    let pin = set_pinned_in(&state.db, &conversation_id, &sub, body.pinned, now_millis()).await?;
    wake(&state, [sub]).await;
    Ok(Json(pin))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/pins",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ReaderQuery
    ),
    responses(
        (status = 200, description = "The messages pinned for everyone in this conversation, newest pin first, each with your copy of it", body = PinnedMessageList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read pins, not API keys or running flows"),
        (status = 404, description = "You (or, reading as a bot, the bot) are not in this conversation, or `as` is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/pins",
    skip(state, user)
)]
pub async fn list_pins(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ReaderQuery>,
) -> Result<Json<PinnedMessageList>, ApiError> {
    let sub = user.sub()?;
    let reader = reader(&state.db, &sub, query.as_bot.as_deref()).await?;
    Ok(Json(PinnedMessageList {
        pins: pins_for(&state.db, &conversation_id, &reader).await?,
    }))
}

#[utoipa::path(
    put,
    path = "/inbox/conversations/{conversation_id}/pins/{message_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("message_id" = String, Path, description = "The message to pin")
    ),
    responses(
        (status = 200, description = "The message is pinned for everyone in the conversation; pinning it again changes nothing", body = PinnedMessage),
        (status = 400, description = "The message id is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people pin messages, not bots, API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "The conversation already has 50 pinned messages (`INBOX_PIN_LIMIT`)"),
        (status = 422, description = "The message isn't in your copy of the conversation, or was deleted (`INBOX_TARGET_NOT_FOUND`)")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /inbox/conversations/{conversation_id}/pins/{message_id}",
    skip(state, user)
)]
pub async fn pin_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<Json<PinnedMessage>, ApiError> {
    let sub = user.sub()?;
    let pin = pin_message_in(&state.db, &conversation_id, &sub, &message_id, now_millis()).await?;
    wake_members(&state, &conversation_id).await;
    Ok(Json(pin))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/pins/{message_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("message_id" = String, Path, description = "The pinned message")
    ),
    responses(
        (status = 204, description = "The message is no longer pinned for anyone; unpinning a message that isn't pinned changes nothing"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people unpin messages, not bots, API keys or running flows"),
        (status = 404, description = "You are not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/pins/{message_id}",
    skip(state, user)
)]
pub async fn unpin_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    if unpin_message_in(&state.db, &conversation_id, &sub, &message_id).await? {
        wake_members(&state, &conversation_id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
