use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_bot_tools::configuration::{merge_model_changes, validate_changes};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use super::{
    crud::{self, Patched},
    lock_bot,
    model::{Bot, UpdateBotRequest},
    owned_bot,
};
use crate::{audit, error::ApiError, middleware::jwt::AppUser, state::AppState};

/// Applies only the supplied configuration fields to the row read under the caller's bot lock.
pub(crate) async fn patch_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    mut changes: Value,
) -> Result<Patched, ApiError> {
    validate_changes(&changes).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let current = Bot::from(owned_bot(db, owner_id, bot_id).await?);
    merge_model_changes(&mut changes, &json!(current.model))
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let request: UpdateBotRequest = serde_json::from_value(changes)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    crud::patch_in(db, owner_id, bot_id, request).await
}

pub(crate) async fn apply(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    changes: Value,
) -> Result<Patched, ApiError> {
    validate_changes(&changes).map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .transaction(|txn| {
            let (owner, bot, changes) = (owner_id.to_owned(), bot_id.to_owned(), changes.clone());
            Box::pin(async move {
                lock_bot(txn, &bot).await?;
                patch_in(txn, &owner, &bot, changes).await
            })
        })
        .await
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/configuration",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body(content = Object, description = "Configuration fields to change: name, description, instructions, capabilities, model, embedding_model, stt_model, tts_model, image_model, vision_model or subtask_model. Omitted model.primary, model.fallbacks and model.decision keep their current values. Null clears a nullable field; capabilities replaces the full list."),
    responses(
        (status = 200, description = "The bot after applying only the supplied fields", body = Bot),
        (status = 400, description = "A configuration field is unknown or invalid"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only the owner can update configuration; bot runtime tokens cannot use this endpoint"),
        (status = 404, description = "You have no bot with this id"),
        (status = 422, description = "A capability is unavailable at the bot's location or its capabilities are fixed")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/configuration",
    skip(state, user, changes)
)]
pub async fn update_configuration(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(changes): Json<Value>,
) -> Result<Json<Bot>, ApiError> {
    let owner = user.sub()?;
    let patched = apply(&state, &owner, &bot_id, changes).await?;
    if !patched.changed.is_empty() {
        audit!(
            state,
            user,
            "bot.update",
            "Bot",
            bot_id,
            json!({"fields": patched.changed, "source": "configuration_patch"})
        );
    }
    Ok(Json(Bot::from(patched.bot)))
}
