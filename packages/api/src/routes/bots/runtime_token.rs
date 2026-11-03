//! A bot token for a desktop host: the host signs in as the owner, asks for a token bound to one
//! of the bot's turns and calls the hub with it, so the hub enforces the bot's rights, not the
//! owner's.

use super::owned_bot;
use crate::{
    bot_permission::{SignedBotToken, sign_bot_token},
    entity::bot_turn,
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::bot_turn::TurnAudience;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BotRuntimeTokenRequest {
    /// A turn of this bot that you own.
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotRuntimeToken {
    /// Send as `Authorization: Bearer <token>` on app calls the bot makes during the turn.
    pub token: String,
    /// Unix seconds; ask again before it passes.
    pub expires_at: i64,
}

/// The identity a token for `turn_id` carries: the turn must be the bot's and the caller's.
/// A room turn pins the room so its invitation limits apply.
pub(crate) async fn bot_user_for_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<BotUser, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let turn = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    let in_room = TurnAudience::parse(&turn.audience) == Some(TurnAudience::Room);
    Ok(BotUser {
        bot_id: bot_id.to_string(),
        turn_id: turn.id,
        conv_id: in_room.then_some(turn.conversation_id),
    })
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/runtime-token",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot")),
    request_body = BotRuntimeTokenRequest,
    responses(
        (status = 200, description = "A short-lived token that acts as the bot for this turn.", body = BotRuntimeToken),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or turn")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/runtime-token", skip(state, user, body))]
pub async fn mint_runtime_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotRuntimeTokenRequest>,
) -> Result<Json<BotRuntimeToken>, ApiError> {
    let owner_id = user.sub()?;
    let bot = bot_user_for_turn(&state.db, &owner_id, &bot_id, body.turn_id.trim()).await?;
    let SignedBotToken { token, expires_at } = sign_bot_token(&bot, None)
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))?;
    Ok(Json(BotRuntimeToken { token, expires_at }))
}
