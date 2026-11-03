//! A bot sending its owner a notification from a desktop host: the same grouping, mute and hourly
//! limit as the server's `notify_owner` tool.

use super::owned_bot;
use crate::{
    bot_notify::{ServerNotifier, count_tool_notices},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_bot_tools::{OwnerNotifier, notify::notice_from};
use flow_like_types::bot_notify::{TOOL_MAX_PER_WINDOW, TOOL_WINDOW_SECS};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub(crate) const BOT_NOTIFY_RATE_LIMITED: &str = "BOT_NOTIFY_RATE_LIMITED";
const PUSH_CAPABILITY: &str = "push_notifications";

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NotifyOwnerRequest {
    /// At most 120 characters.
    pub title: String,
    /// At most 500 characters.
    pub body: String,
    /// An in-app path (starts with `/`); omitted opens the bot's chat.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct NotifyOwnerResponse {
    /// The notification; empty when the bot is muted.
    pub id: String,
    /// The owner muted this bot, so nothing was sent.
    pub muted: bool,
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/notify",
    tag = "Bots",
    description = "Send your own notification as one of your bots, grouped under the bot's name. The bot needs the push_notifications capability; at most 10 per hour.",
    params(("bot_id" = String, Path, description = "One of your bots")),
    request_body = NotifyOwnerRequest,
    responses(
        (status = 200, description = "Sent, or skipped because the bot is muted", body = NotifyOwnerResponse),
        (status = 400, description = "The notification is malformed, or the bot lacks the capability"),
        (status = 404, description = "Not your bot"),
        (status = 429, description = "The bot sent 10 notifications in the last hour")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/notify", skip_all)]
pub async fn notify_owner(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<NotifyOwnerRequest>,
) -> Result<Json<NotifyOwnerResponse>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    if !bot
        .capabilities
        .iter()
        .any(|id| id.as_str() == PUSH_CAPABILITY)
    {
        return Err(ApiError::bad_request(format!(
            "Bot '{bot_id}' does not have the {PUSH_CAPABILITY} capability"
        )));
    }
    let notice = notice_from(&request.title, &request.body, request.link)
        .map_err(|error| ApiError::bad_request(error.message))?;
    let since = chrono::Utc::now().timestamp_millis() - TOOL_WINDOW_SECS * 1000;
    let sent = count_tool_notices(&state.db, &sub, &bot.id, since)
        .await
        .map_err(ApiError::internal_error)?;
    if sent >= TOOL_MAX_PER_WINDOW {
        return Err(ApiError::coded(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            BOT_NOTIFY_RATE_LIMITED,
            format!("Bot '{bot_id}' already sent {sent} notifications in the last hour"),
        ));
    }
    let receipt = ServerNotifier::new(&state, &bot.id)
        .deliver(notice)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(NotifyOwnerResponse {
        id: receipt.id,
        muted: receipt.muted,
    }))
}
