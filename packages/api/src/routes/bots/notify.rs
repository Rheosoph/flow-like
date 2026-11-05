//! A bot messaging its owner from a desktop host: the same chat message, grouping, mute and hourly
//! limit as the server's `message_owner` tool.

use super::owned_bot;
use crate::{
    bot_notify::{ServerNotifier, count_tool_messages},
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
pub struct MessageOwnerRequest {
    /// At most 4000 characters.
    pub text: String,
    /// A headline above the text; at most 120 characters.
    pub title: Option<String>,
    /// The conversation the bot's turn runs in; the message is refused when that is the owner's
    /// direct chat with the bot.
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MessageOwnerResponse {
    /// The message id.
    pub id: String,
    /// The owner muted this bot, so no push was sent.
    pub muted: bool,
    /// The message is in the owner's chat with the bot.
    pub posted: bool,
    /// The turn already runs in the owner's chat; nothing was sent.
    pub in_owner_chat: bool,
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/notify",
    tag = "Bots",
    description = "Write a message to yourself in your chat with one of your bots, as the bot, with a push notification grouped under the bot's name. The bot needs the push_notifications capability; at most 10 per hour.",
    params(("bot_id" = String, Path, description = "One of your bots")),
    request_body = MessageOwnerRequest,
    responses(
        (status = 200, description = "Sent, refused because the turn runs in the owner chat, or sent without a push because the bot is muted", body = MessageOwnerResponse),
        (status = 400, description = "The notification is malformed, or the bot lacks the capability"),
        (status = 404, description = "Not your bot"),
        (status = 429, description = "The bot sent 10 messages in the last hour")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/notify", skip_all)]
pub async fn message_owner(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<MessageOwnerRequest>,
) -> Result<Json<MessageOwnerResponse>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
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
    let notice = notice_from(&request.text, request.title.as_deref())
        .map_err(|error| ApiError::bad_request(error.message))?;
    let since = chrono::Utc::now().timestamp_millis() - TOOL_WINDOW_SECS * 1000;
    let sent = count_tool_messages(&state.db, &bot.id, since)
        .await
        .map_err(ApiError::internal_error)?;
    if sent >= TOOL_MAX_PER_WINDOW {
        return Err(ApiError::coded(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            BOT_NOTIFY_RATE_LIMITED,
            format!("Bot '{bot_id}' already sent {sent} messages in the last hour"),
        ));
    }
    let receipt = ServerNotifier::new(
        &state,
        &bot.id,
        request
            .conversation_id
            .map(flow_like_bot_tools::TurnSite::Conversation),
    )
        .deliver(notice)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(MessageOwnerResponse {
        id: receipt.id,
        muted: receipt.muted,
        posted: receipt.posted,
        in_owner_chat: receipt.in_owner_chat,
    }))
}
