//! The owner's current instructions for a running turn, read by hosts that infer per-call
//! permission. The host classifies with its own decision model; no route runs inference.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::bot_turn::STORED_RUNNING;
use serde_json::Value;

use crate::{
    bot_caller::{self, Names},
    bot_host::{authorization, ledger, talk},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};

async fn current_turn(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
    turn_id: &str,
) -> Result<crate::entity::bot_turn::Model, ApiError> {
    let owner = bot_caller::acting_owner(
        &state.db,
        user,
        &Names {
            as_bot: Some(bot_id),
            turn: Some(turn_id),
            ..Default::default()
        },
    )
    .await?;
    let turn = ledger::find(&state.db, turn_id, Some(&owner))
        .await?
        .filter(|turn| turn.bot_id == bot_id)
        .ok_or_else(|| ApiError::not_found("This bot turn was not found"))?;
    if turn.status != STORED_RUNNING || turn.lease_until.is_none_or(|until| until <= now_millis()) {
        return Err(ApiError::conflict("This bot turn is no longer running"));
    }
    if let AppUser::Bot(bot) = user {
        let origin = talk::turn_origin_conversation(&state.db, &turn).await?;
        bot_caller::ensure_room(bot, &origin)?;
    }
    Ok(turn)
}

#[utoipa::path(get, path = "/bots/{bot_id}/turns/{turn_id}/authorization-context", tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id"), ("turn_id" = String, Path, description = "The running turn's id")),
    responses(
        (status = 200, description = "Your current instructions to the bot, used to decide whether a background action needs your approval", body = Object),
        (status = 404, description = "The bot or turn was not found"),
        (status = 409, description = "The turn is no longer running")
    ),
    security(("bearer_auth" = [])))]
pub(crate) async fn context(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let turn = current_turn(&state, &user, &bot_id, &turn_id).await?;
    let include_direct = !matches!(&user, AppUser::Bot(bot) if bot.conv_id.is_some());
    Ok(Json(
        authorization::context(&state.db, &turn, include_direct).await?,
    ))
}
