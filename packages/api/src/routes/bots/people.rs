//! A bot on a desktop host reaching the people its owner chats with: the same contacts, room,
//! push and hourly limit as the server's `list_people` and `message_person` tools.

use super::owned_bot;
use crate::{
    bot_notify::StateSink,
    bot_people::{StateRooms, contacts, count_person_messages, deliver_person_message},
    entity::bot,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use flow_like_bot_tools::{
    Delivery,
    people::{MAX_PER_WINDOW, TEXT_MAX_CHARS, WINDOW_SECS, person_id},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub(crate) const BOT_PEOPLE_RATE_LIMITED: &str = "BOT_PEOPLE_RATE_LIMITED";
pub(crate) const BOT_PERSON_NOT_A_CONTACT: &str = "BOT_PERSON_NOT_A_CONTACT";
pub(crate) const BOT_PERSON_LEFT: &str = "BOT_PERSON_LEFT";
const CHANNELS_CAPABILITY: &str = "channels";

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct PeopleQuery {
    /// Part of a name, ignoring case.
    pub q: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotContact {
    /// The person's user id; `message_person` takes it.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotContactList {
    pub people: Vec<BotContact>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MessagePersonRequest {
    /// The person's user id, one of `GET /bots/{bot_id}/people`.
    pub person: String,
    /// At most 4000 characters.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MessagePersonResponse {
    /// The bot's room with the person.
    pub conversation_id: String,
    pub message_id: String,
    /// The person's name.
    pub name: String,
    /// The person got a push; `false` when they muted the room.
    pub notified: bool,
}

async fn talking_bot(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
) -> Result<bot::Model, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, bot_id).await?;
    if !bot
        .capabilities
        .iter()
        .any(|id| id.as_str() == CHANNELS_CAPABILITY)
    {
        return Err(ApiError::bad_request(format!(
            "Bot '{bot_id}' does not have the {CHANNELS_CAPABILITY} capability"
        )));
    }
    Ok(bot)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/people",
    tag = "Bots",
    description = "The people you chat with directly whom one of your bots may write to, newest conversation first, at most 50. The bot needs the channels capability.",
    params(("bot_id" = String, Path, description = "One of your bots"), PeopleQuery),
    responses(
        (status = 200, description = "Your contacts with their ids", body = BotContactList),
        (status = 400, description = "The bot lacks the channels capability"),
        (status = 404, description = "Not your bot")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/people", skip_all)]
pub async fn list_bot_people(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<PeopleQuery>,
) -> Result<Json<BotContactList>, ApiError> {
    let bot = talking_bot(&state, &user, &bot_id).await?;
    let people = contacts(&state.db, &bot.owner_id, query.q.as_deref())
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(BotContactList {
        people: people
            .into_iter()
            .map(|contact| BotContact {
                id: contact.id,
                name: contact.name,
            })
            .collect(),
    }))
}

/// The request's person id and trimmed text, or why they can't be sent.
fn checked(request: &MessagePersonRequest) -> Result<(&str, &str), ApiError> {
    let person = person_id(&request.person)
        .ok_or_else(|| ApiError::bad_request("'person' must be a user id"))?;
    let text = request.text.trim();
    let chars = text.chars().count();
    if chars == 0 || chars > TEXT_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "'text' must have 1 to {TEXT_MAX_CHARS} characters, not {chars}"
        )));
    }
    Ok((person, text))
}

async fn within_hourly_limit(state: &AppState, bot_id: &str) -> Result<(), ApiError> {
    let since = chrono::Utc::now().timestamp_millis() - WINDOW_SECS * 1000;
    let sent = count_person_messages(&state.db, bot_id, since)
        .await
        .map_err(ApiError::internal_error)?;
    if sent < MAX_PER_WINDOW {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::TOO_MANY_REQUESTS,
        BOT_PEOPLE_RATE_LIMITED,
        format!("Bot '{bot_id}' already sent {sent} messages to people in the last hour"),
    ))
}

fn response_of(delivery: Delivery, person: &str) -> Result<Json<MessagePersonResponse>, ApiError> {
    match delivery {
        Delivery::Sent(message) => Ok(Json(MessagePersonResponse {
            conversation_id: message.conversation_id,
            message_id: message.message_id,
            name: message.name,
            notified: message.notified,
        })),
        Delivery::NotAContact => Err(ApiError::coded(
            StatusCode::NOT_FOUND,
            BOT_PERSON_NOT_A_CONTACT,
            format!("You have no direct chat with '{person}'"),
        )),
        Delivery::Left { name } => {
            Err(ApiError::coded(StatusCode::CONFLICT, BOT_PERSON_LEFT, name))
        }
    }
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/people/message",
    tag = "Bots",
    description = "Write as one of your bots to a person you chat with directly. The message goes into the bot's room with that person and you, where their answer reaches the bot, and the person gets a push. The bot needs the channels capability; at most 20 messages per hour.",
    params(("bot_id" = String, Path, description = "One of your bots")),
    request_body = MessagePersonRequest,
    responses(
        (status = 200, description = "Sent", body = MessagePersonResponse),
        (status = 400, description = "The message is malformed, or the bot lacks the channels capability"),
        (status = 404, description = "Not your bot, or not a person you chat with directly"),
        (status = 409, description = "The person left the bot's room with them"),
        (status = 429, description = "The bot sent 20 messages to people in the last hour")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/people/message", skip_all)]
pub async fn message_person(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(request): Json<MessagePersonRequest>,
) -> Result<Json<MessagePersonResponse>, ApiError> {
    let bot = talking_bot(&state, &user, &bot_id).await?;
    let (person, text) = checked(&request)?;
    within_hourly_limit(&state, &bot.id).await?;
    let delivery = deliver_person_message(
        &state.db,
        &StateRooms(&state),
        &StateSink(&state),
        &bot.id,
        person,
        text,
    )
    .await
    .map_err(ApiError::internal_error)?;
    response_of(delivery, person)
}
