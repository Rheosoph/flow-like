use super::{
    acting_reader,
    model::{BotMemoryPolicy, ReaderQuery},
    reader_membership,
};
use crate::{
    bot_host::memory::memory_policy, error::ApiError, middleware::jwt::AppUser, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::ConnectionTrait;

pub(super) async fn policy_for_caller<C: ConnectionTrait>(
    db: &C,
    user: &AppUser,
    conversation_id: &str,
    as_bot: Option<&str>,
) -> Result<BotMemoryPolicy, ApiError> {
    let reader = acting_reader(db, user, as_bot, conversation_id).await?;
    reader_membership(db, conversation_id, &reader).await?;
    let bot_id = reader
        .bot()
        .ok_or_else(|| ApiError::bad_request("Choose the bot with as"))?;
    let owner = reader
        .owner
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("The bot has no owner"))?;
    memory_policy(db, conversation_id, owner, bot_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Bot memory is unavailable in this conversation"))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/memory",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The current conversation"), ReaderQuery),
    responses(
        (status = 200, description = "Current memory permissions and shared fact-store keys for this bot", body = BotMemoryPolicy),
        (status = 403, description = "The caller cannot act as this bot in this conversation"),
        (status = 404, description = "The bot has no memory access in this conversation")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_memory_policy(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ReaderQuery>,
) -> Result<Json<BotMemoryPolicy>, ApiError> {
    let policy =
        policy_for_caller(&state.db, &user, &conversation_id, query.as_bot.as_deref()).await?;
    Ok(Json(policy))
}
