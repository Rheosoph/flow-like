//! The picture of a bot's look. Looks are drawn by clients only, so a client that shows a bot
//! uploads its picture once per look; push notifications show it as the bot's avatar.

use super::owned_bot;
use crate::{
    entity::bot,
    error::ApiError,
    middleware::jwt::AppUser,
    notification_images::{bot_avatar_url, decode_inline_icon, store_bot_avatar},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{Value, bot_notify::MUTE_KEY};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PutBotAvatarRequest {
    /// The appearance the picture shows, as the bot was read. Refused when the bot looks different now.
    #[schema(value_type = Object)]
    pub appearance: Value,
    /// A PNG, JPEG, GIF or WebP data URL of at most 512 KiB, stored as a PNG of at most 256 px.
    pub image: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotAvatar {
    /// A link to the picture of the bot's current look, valid for a day; empty until one is uploaded.
    pub url: Option<String>,
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().filter(|(_, value)| !value.is_null()).collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Names what a bot looks like: its appearance without the settings kept beside it, whatever
/// order its keys were written in.
pub(crate) fn look_digest(appearance: &Value) -> String {
    let mut look = appearance.clone();
    if let Some(map) = look.as_object_mut() {
        map.remove(MUTE_KEY);
    }
    blake3::hash(canonical(&look).to_string().as_bytes()).to_hex()[..32].to_string()
}

/// The link to the picture of the bot's current look, if a client uploaded one.
pub(crate) async fn current_avatar_url(
    state: &AppState,
    bot: &bot::Model,
) -> flow_like_types::Result<Option<String>> {
    let store = state.master_credentials().await?.to_store(false).await?;
    bot_avatar_url(
        &store,
        &bot.owner_id,
        &bot.id,
        &look_digest(&bot.appearance),
    )
    .await
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/avatar",
    tag = "Bots",
    description = "The picture of your bot's current look that push notifications show as its avatar.",
    params(("bot_id" = String, Path, description = "One of your bots")),
    responses(
        (status = 200, description = "The picture, or none when no picture of the current look was uploaded", body = BotAvatar),
        (status = 404, description = "Not your bot")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/avatar", skip_all)]
pub async fn get_bot_avatar(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotAvatar>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let url = current_avatar_url(&state, &bot)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(BotAvatar { url }))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/avatar",
    tag = "Bots",
    description = "Upload the picture of your bot's current look, which push notifications show as its avatar. Pictures of its earlier looks are deleted.",
    params(("bot_id" = String, Path, description = "One of your bots")),
    request_body = PutBotAvatarRequest,
    responses(
        (status = 200, description = "The stored picture", body = BotAvatar),
        (status = 400, description = "The image is not a PNG, JPEG, GIF or WebP data URL of at most 512 KiB"),
        (status = 404, description = "Not your bot"),
        (status = 409, description = "The bot's look changed since it was read")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/avatar", skip_all)]
pub async fn put_bot_avatar(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<PutBotAvatarRequest>,
) -> Result<Json<BotAvatar>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let look = look_digest(&bot.appearance);
    if look_digest(&request.appearance) != look {
        return Err(ApiError::conflict(format!(
            "Bot '{bot_id}' looks different now; draw its current look and upload that"
        )));
    }
    let png = decode_avatar(&bot_id, request.image).await?;
    let url = save_avatar(&state, &bot, &look, png)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(BotAvatar { url }))
}

async fn decode_avatar(bot_id: &str, image: String) -> Result<Vec<u8>, ApiError> {
    flow_like_types::tokio::task::spawn_blocking(move || decode_inline_icon(&image))
        .await
        .map_err(|error| ApiError::internal_error(error.into()))?
        .map_err(|error| {
            ApiError::bad_request(format!(
                "The picture of bot '{bot_id}' is unusable: {error}"
            ))
        })
}

async fn save_avatar(
    state: &AppState,
    bot: &bot::Model,
    look: &str,
    png: Vec<u8>,
) -> flow_like_types::Result<Option<String>> {
    let store = state.master_credentials().await?.to_store(false).await?;
    store_bot_avatar(&store, &bot.owner_id, &bot.id, look, png).await?;
    bot_avatar_url(&store, &bot.owner_id, &bot.id, look).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn a_look_ignores_key_order_nulls_and_the_mute_switch() {
        let look = look_digest(&json!({
            "kind": "pip",
            "colors": { "body": "a", "ink": "b" },
            "skills": ["web"],
            "variant": "gold",
        }));
        assert_eq!(
            look,
            look_digest(&json!({
                "mute_notifications": true,
                "variant": "gold",
                "colors": { "ink": "b", "body": "a" },
                "halo": null,
                "skills": ["web"],
                "kind": "pip",
            }))
        );
        assert_eq!(
            look,
            look_digest(&json!({
                "colors": { "body": "a", "ink": "b" },
                "kind": "pip",
                "mute_notifications": false,
                "skills": ["web"],
                "variant": "gold",
            }))
        );
        assert_ne!(look, look_digest(&json!({ "kind": "pip" })));
        assert_ne!(
            look,
            look_digest(&json!({
                "kind": "pip",
                "colors": { "body": "b", "ink": "a" },
                "skills": ["web"],
                "variant": "gold",
            }))
        );
    }
}
