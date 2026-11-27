//! The owner's routes for a bot's outside chats: the Discord messages their desktop relays, the
//! turns those messages queued, and the list of linked chats.

use super::{
    ChannelProvider, OutsideAttachment, OutsideMessage, OutsideSender, Posted,
    link_of_conversation, post_outside_message,
};
use crate::{
    entity::{bot_channel_link, bot_turn},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::{
        fire::{FiredTurn, wait_for_turn},
        owned_bot,
    },
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use utoipa::{IntoParams, ToSchema};

const MAX_TURN_WAIT_SECS: u64 = 25;
const MAX_LISTED_LINKS: u64 = 200;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{bot_id}/channels", get(list_channel_links))
        .route("/{bot_id}/channels/{link_id}", delete(delete_channel_link))
        .route(
            "/{bot_id}/channels/discord/messages",
            post(post_discord_message).layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES)),
        )
        .route("/{bot_id}/channels/turns/{turn_id}", get(get_channel_turn))
}

/// Who wrote the Discord message.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DiscordSender {
    /// The Discord user id.
    pub id: String,
    /// The name Discord shows for them in that chat.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DiscordAttachment {
    pub name: String,
    /// An https link to the file.
    pub url: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// One Discord message addressed to the bot, as the owner's desktop relays it.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DiscordMessageRequest {
    /// The channel (or thread) id.
    pub chat_id: String,
    /// The server id; none for a direct message.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// How the chat reads, e.g. "#general (Acme)".
    pub chat_title: String,
    pub message_id: String,
    pub sender: DiscordSender,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<DiscordAttachment>,
}

impl DiscordMessageRequest {
    fn chat_key(&self) -> String {
        match self
            .guild_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(guild) => format!("{guild}:{}", self.chat_id.trim()),
            None => format!("dm:{}", self.chat_id.trim()),
        }
    }

    pub(super) fn into_message(self) -> OutsideMessage {
        OutsideMessage {
            provider: ChannelProvider::Discord,
            chat_key: self.chat_key(),
            meta: json!({
                "channel_id": self.chat_id.trim(),
                "guild_id": self.guild_id.as_deref().map(str::trim),
            }),
            chat_title: self.chat_title,
            message_id: self.message_id,
            sender: OutsideSender {
                id: self.sender.id,
                name: self.sender.name,
            },
            text: self.text,
            attachments: self
                .attachments
                .into_iter()
                .map(|attachment| OutsideAttachment {
                    name: attachment.name,
                    url: attachment.url,
                    content_type: attachment.content_type,
                })
                .collect(),
        }
    }
}

/// What became of a relayed chat message.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChannelPostedView {
    pub link_id: String,
    /// The chat's guest room.
    pub conversation_id: String,
    pub trigger_message_id: String,
    /// The bot's turn; none when the message was refused for a limit.
    pub turn_id: Option<String>,
    /// `queued`, `limited` (refused; send `notice` back) or `duplicate` (already relayed).
    pub status: String,
    pub notice: Option<String>,
}

impl From<Posted> for ChannelPostedView {
    fn from(posted: Posted) -> Self {
        Self {
            link_id: posted.link_id,
            conversation_id: posted.conversation_id,
            trigger_message_id: posted.trigger_message_id,
            turn_id: posted.turn_id,
            status: posted.status.wire().to_string(),
            notice: posted.notice,
        }
    }
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/channels/discord/messages",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = DiscordMessageRequest,
    responses(
        (status = 200, description = "The message went into the chat's guest room and queued the bot's turn, or was refused for a limit (status `limited`, with the notice to send back). A message relayed twice queues one turn.", body = ChannelPostedView),
        (status = 400, description = "The message has no text and no attachment, or a field is too long"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people relay chats, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_CHANNEL_NOT_ALLOWED: Flux can't join chats")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/channels/discord/messages",
    skip(state, user, body)
)]
pub async fn post_discord_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<DiscordMessageRequest>,
) -> Result<Json<ChannelPostedView>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let posted = post_outside_message(&state, &bot, &body.into_message()).await?;
    Ok(Json(posted.into()))
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ChannelTurnQuery {
    /// Seconds to wait for the reply, at most 25.
    #[serde(default)]
    pub wait: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/channels/turns/{turn_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("turn_id" = String, Path, description = "The turn a chat message queued"),
        ChannelTurnQuery
    ),
    responses(
        (status = 200, description = "Where the turn stands, with the reply once the bot answered. Waits up to `wait` seconds for it.", body = FiredTurn),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No turn of this bot answers a chat message under this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /bots/{bot_id}/channels/turns/{turn_id}",
    skip(state, user)
)]
pub async fn get_channel_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
    Query(query): Query<ChannelTurnQuery>,
) -> Result<Json<FiredTurn>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let turn = bot_turn::Entity::find_by_id(&turn_id)
        .filter(bot_turn::Column::BotId.eq(&bot_id))
        .filter(bot_turn::Column::OwnerId.eq(&sub))
        .one(&state.db)
        .await?
        .filter(|turn| {
            turn.trigger_message_id
                .starts_with(super::CHANNEL_MESSAGE_PREFIX)
        })
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    if link_of_conversation(&state.db, &turn.conversation_id)
        .await?
        .is_none()
    {
        return Err(ApiError::not_found(format!("Turn '{turn_id}' not found")));
    }
    let wait = Duration::from_secs(query.wait.unwrap_or(0).min(MAX_TURN_WAIT_SECS));
    Ok(Json(wait_for_turn(&state.db, &sub, &turn_id, wait).await?))
}

/// An outside chat the bot is part of.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChannelLinkView {
    pub id: String,
    /// `teams` or `discord`.
    pub provider: String,
    pub chat_title: String,
    /// The chat's guest room.
    pub conversation_id: String,
    pub created_at: String,
    pub last_seen_at: String,
}

impl From<bot_channel_link::Model> for ChannelLinkView {
    fn from(link: bot_channel_link::Model) -> Self {
        Self {
            id: link.id,
            provider: link.provider,
            chat_title: link.chat_title,
            conversation_id: link.conversation_id,
            created_at: link.created_at.to_rfc3339(),
            last_seen_at: link.last_seen_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChannelLinkList {
    pub links: Vec<ChannelLinkView>,
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/channels",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The Microsoft Teams and Discord chats the bot is part of, most recently active first (at most 200), each with its guest room.", body = ChannelLinkList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/channels", skip(state, user))]
pub async fn list_channel_links(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<ChannelLinkList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let links = bot_channel_link::Entity::find()
        .filter(bot_channel_link::Column::BotId.eq(&bot_id))
        .filter(bot_channel_link::Column::OwnerId.eq(&sub))
        .order_by_desc(bot_channel_link::Column::LastSeenAt)
        .limit(MAX_LISTED_LINKS)
        .all(&state.db)
        .await?
        .into_iter()
        .map(ChannelLinkView::from)
        .collect();
    Ok(Json(ChannelLinkList { links }))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/channels/{link_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("link_id" = String, Path, description = "The linked chat")
    ),
    responses(
        (status = 204, description = "The chat is no longer linked to its guest room. The room stays with its history; the chat's next message to the bot opens a new room."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id, or it isn't part of this chat")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/channels/{link_id}", skip(state, user))]
pub async fn delete_channel_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, link_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let removed = bot_channel_link::Entity::delete_many()
        .filter(bot_channel_link::Column::Id.eq(&link_id))
        .filter(bot_channel_link::Column::BotId.eq(&bot_id))
        .filter(bot_channel_link::Column::OwnerId.eq(&sub))
        .exec(&state.db)
        .await?;
    if removed.rows_affected == 0 {
        return Err(ApiError::not_found(format!("Chat '{link_id}' not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}
