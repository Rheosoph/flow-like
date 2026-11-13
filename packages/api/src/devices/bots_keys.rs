//! Inbox keys of the bots bound to a device. The device makes each encryption bot's key pair
//! itself and publishes only the public half here, before any turn exists, so members can seal
//! to the bot; the private half never leaves the device. The hub hands the device a turn of an
//! end-to-end encrypted conversation only once the bot has a published key.

use super::{bots::owner_of, bots_grant::is_end_to_end};
use crate::{
    bot_host::ledger,
    db::coordination::coordinate,
    entity::{bot, bot_turn, conversation, inbox_key},
    error::ApiError,
    routes::{
        bots::model::BotSettings,
        inbox::{
            keys::{NewKey, register_in, validate_key},
            model::{InboxKey, RegisterInboxKeyRequest},
            now_millis,
        },
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use flow_like_types::{bot::BotCapability, bot_turn::TurnHostKind};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};

pub(crate) const BOT_DEVICE_E2E_KEY_MISSING: &str = "BOT_DEVICE_E2E_KEY_MISSING";

/// The live bot `bot_id` of `owner` bound to `device_id`; any other bot reads as missing.
pub(crate) async fn bound_bot<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    bot_id: &str,
) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::OwnerId.eq(owner))
        .filter(bot::Column::DeviceId.eq(device_id))
        .filter(bot::Column::Location.eq(TurnHostKind::Device.stored()))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Bot '{bot_id}' is not a live bot bound to device '{device_id}'"
            ))
        })
}

/// The bot a device publishes a key for, which must be bound to it, and the checked key.
pub(crate) async fn checked_key<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    request: &RegisterInboxKeyRequest,
) -> Result<(String, NewKey), ApiError> {
    let key = validate_key(request)?;
    let bot_id = request
        .principal_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ApiError::bad_request("principal_id must name a bot bound to this device")
        })?;
    Ok((bound_bot(db, owner, device_id, bot_id).await?.id, key))
}

/// Whether the device can open the bot's encrypted conversations: the bot holds `encryption`
/// and has an active published key.
pub(crate) async fn can_open_sealed<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<bool, ApiError> {
    let Some(row) = bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    if !BotSettings::from_row(&row)
        .capabilities
        .contains(&BotCapability::Encryption)
    {
        return Ok(false);
    }
    let keys = inbox_key::Entity::find()
        .filter(inbox_key::Column::PrincipalId.eq(bot_id))
        .filter(inbox_key::Column::RevokedAt.is_null())
        .count(db)
        .await?;
    Ok(keys > 0)
}

/// A device gets the claimed turn of an end-to-end encrypted conversation only when its bot can
/// open it there; otherwise the turn fails with [`BOT_DEVICE_E2E_KEY_MISSING`] and nothing is
/// posted, since the hub can't write into the conversation.
pub(crate) async fn ensure_openable<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    host_id: &str,
    conversation: &conversation::Model,
) -> Result<(), ApiError> {
    if !is_end_to_end(conversation) || can_open_sealed(db, &turn.bot_id).await? {
        return Ok(());
    }
    ledger::fail_held_in(
        db,
        &turn.id,
        host_id,
        BOT_DEVICE_E2E_KEY_MISSING,
        now_millis(),
    )
    .await?;
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        BOT_DEVICE_E2E_KEY_MISSING,
        format!(
            "Conversation '{}' is end-to-end encrypted and bot '{}' has no encryption ability or no inbox key published from its device",
            conversation.id, turn.bot_id
        ),
    ))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/inbox-keys",
    tag = "devices",
    description = "A device publishes the public inbox key of a bot bound to it, so people can seal messages to the bot. The private key stays on the device.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = RegisterInboxKeyRequest,
    responses(
        (status = 200, description = "The registered key; publishing the same key again returns it", body = InboxKey),
        (status = 400, description = "principal_id is missing, or the key id, algorithm or public key is malformed"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "principal_id is not a live bot bound to this device"),
        (status = 409, description = "INBOX_KEY_EXISTS: the key id is taken by other material; INBOX_KEY_LIMIT: the bot has 20 active keys")
    )
)]
pub(crate) async fn publish_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RegisterInboxKeyRequest>,
) -> Result<Json<InboxKey>, ApiError> {
    let owner = owner_of(&state, &id, &headers, "/bots/inbox-keys").await?;
    let (bot_id, key) = checked_key(&state.db, &owner, &id, &body).await?;
    let stored = state
        .transaction(|txn| {
            let (bot_id, key) = (bot_id.clone(), key.clone());
            Box::pin(async move {
                coordinate(txn, "inbox-keys", &[&bot_id]).await?;
                register_in(txn, &bot_id, &key, now_millis()).await
            })
        })
        .await?;
    tracing::info!(
        device_id = %id,
        bot_id = %stored.principal_id,
        key_id = %stored.key_id,
        "A device published a bot's inbox key"
    );
    Ok(Json(InboxKey::from(stored)))
}
