//! A desktop host signs in as the owner and obtains a token for its bot. A supplied turn pins
//! its group. Direct app tokens carry no group and cannot access inbox conversations.

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
    /// A turn of this bot that you own. Omit for an owner-initiated app call.
    pub turn_id: Option<String>,
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
    let bot = bot_user_for_request(&state, &owner_id, bot_id, body.turn_id.as_deref()).await?;
    let SignedBotToken { token, expires_at } = sign_bot_token(&bot, None)
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))?;
    Ok(Json(BotRuntimeToken { token, expires_at }))
}

/// A model request may start a direct chat, or answer a validated inbox turn.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BotModelTokenRequest {
    pub turn_id: Option<String>,
    pub app_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/model-token",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot")),
    request_body = BotModelTokenRequest,
    responses((status = 200, description = "A model token attributed to this bot and its turn's group", body = BotRuntimeToken)),
    security(("bearer_auth" = []))
)]
pub async fn mint_model_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotModelTokenRequest>,
) -> Result<Json<BotRuntimeToken>, ApiError> {
    let owner_id = user.sub()?;
    let bot = bot_user_for_request(&state, &owner_id, bot_id, body.turn_id.as_deref()).await?;
    if let Some(app_id) = &body.app_id {
        crate::bot_permission::bot_app_permission(&bot, app_id, &state).await?;
    }
    const TTL_SECONDS: i64 = 900;
    // A project is attached only after checking the bot's current app permission.
    let token = crate::bot_host::worker::executor_token(
        &owner_id,
        &bot.bot_id,
        bot.conv_id.as_deref(),
        &bot.turn_id,
        body.app_id.as_deref(),
        TTL_SECONDS,
    )
    .ok_or_else(|| ApiError::internal("The bot model token could not be signed"))?;
    Ok(Json(BotRuntimeToken {
        token,
        expires_at: chrono::Utc::now().timestamp() + TTL_SECONDS,
    }))
}

async fn bot_user_for_request(
    state: &AppState,
    owner_id: &str,
    bot_id: String,
    turn_id: Option<&str>,
) -> Result<BotUser, ApiError> {
    let bot_id = match bot_id.as_str() {
        "general" | "flux" => {
            let rows = super::provision::bots_with_system(state, owner_id).await?;
            rows.into_iter()
                .find(|row| row.system_key.as_deref() == Some(bot_id.as_str()))
                .map(|row| row.id)
                .ok_or_else(|| ApiError::not_found("The account's built-in bot is unavailable"))?
        }
        _ => bot_id,
    };
    let bot = match turn_id {
        Some(turn_id) => bot_user_for_turn(&state.db, owner_id, &bot_id, turn_id.trim()).await?,
        None => bot_user_for_direct(&state.db, owner_id, &bot_id).await?,
    };
    crate::bot_caller::usage_identity(&state.db, &bot).await?;
    Ok(bot)
}

/// Direct app calls carry no room and cannot use inbox conversation endpoints.
pub(crate) async fn bot_user_for_direct<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<BotUser, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    Ok(BotUser {
        bot_id: bot_id.to_owned(),
        turn_id: format!("direct:{}", flow_like_types::create_id()),
        conv_id: None,
    })
}
