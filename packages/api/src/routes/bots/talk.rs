//! Bots asking each other for callers that are not the server: a bot running on a desktop reaches
//! its crew through the hub. The exchange, the grants and the wait live here; the desktop only
//! asks and shows the answer.

use super::owned_bot;
use crate::{
    bot_caller,
    bot_host::talk::{AnsweredAsk, ServerTalkHost, answered_ask, bot_name, crew_member, roster_in},
    entity::bot_turn,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::copilot::bot_talk::{BotPresence, BotTalkHost, CrewMember, PendingAnswer};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_QUESTION_CHARS: usize = 8_000;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TalkRoster {
    /// The caller's other bots with where they run and whether they are online.
    #[schema(value_type = Vec<Object>)]
    pub members: Vec<CrewMember>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TalkRequest {
    /// The bot to ask; one of the caller's other bots.
    pub callee_id: String,
    /// The question, at most 8,000 characters.
    pub question: String,
    /// The asking run; asking the same question in the same run again continues the same exchange.
    pub run_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TalkStatus {
    /// `text` is the callee's answer.
    Answered,
    /// The callee has not answered yet or is offline; `text` says so. The answer wakes the asker
    /// in its conversation with the owner.
    Pending,
    /// The callee could not answer; `text` says why.
    Failed,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TalkExchangeRequest {
    /// The bot to ask; one of the caller's other bots.
    pub callee_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TalkExchange {
    /// The conversation between the two bots, created on their first exchange.
    pub exchange_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TalkAnswer {
    pub status: TalkStatus,
    pub text: String,
    /// The conversation between the two bots.
    pub exchange_id: String,
    /// The question's message in the exchange.
    pub ask_id: String,
    /// The callee's answer message in the exchange when it answered.
    pub reply_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AskResult {
    /// The conversation the question was asked from; the turn's reply belongs there.
    pub origin_conversation: String,
    pub callee_id: String,
    pub callee_name: String,
    pub question: String,
    pub answer: String,
    pub failed: bool,
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/talk/roster",
    tag = "Bots",
    description = "The bots this bot may ask: your other bots with where they run and whether they are online.",
    params(("bot_id" = String, Path, description = "The asking bot; yours")),
    responses(
        (status = 200, description = "The crew", body = TalkRoster),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/talk/roster", skip(state, user))]
pub async fn talk_roster(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<TalkRoster>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let members = roster_in(&state.db, &sub, &bot_id, now_millis()).await?;
    Ok(Json(TalkRoster { members }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/talk/exchange",
    tag = "Bots",
    description = "The conversation between this bot and another of your bots, created on their first exchange. One pair has one conversation.",
    params(("bot_id" = String, Path, description = "The asking bot; yours")),
    request_body = TalkExchangeRequest,
    responses(
        (status = 200, description = "The exchange", body = TalkExchange),
        (status = 400, description = "The bot asks itself"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "One of the bots isn't yours or was deleted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/talk/exchange", skip(state, user, body))]
pub async fn talk_exchange(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<TalkExchangeRequest>,
) -> Result<Json<TalkExchange>, ApiError> {
    let sub = user.sub()?;
    if body.callee_id == bot_id {
        return Err(ApiError::bad_request("A bot can't ask itself"));
    }
    let caller = crew_member(
        &owned_bot(&state.db, &sub, &bot_id).await?,
        BotPresence::Online,
    );
    let callee = crew_member(
        &owned_bot(&state.db, &sub, &body.callee_id).await?,
        BotPresence::Unknown,
    );
    let host = ServerTalkHost::for_chat(state.clone(), &sub, "");
    let exchange_id = host
        .exchange_conversation(&caller, &callee)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(TalkExchange { exchange_id }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/talk",
    tag = "Bots",
    description = "A bot of yours asks another of your bots and waits for the answer. The exchange is recorded as a conversation between the two bots. Permission is the caller's to check: this endpoint only runs asks between bots you own.",
    params(("bot_id" = String, Path, description = "The asking bot; yours")),
    request_body = TalkRequest,
    responses(
        (status = 200, description = "The answer, or why there is none yet", body = TalkAnswer),
        (status = 400, description = "The question is empty or too long, or the bot asks itself"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "One of the bots isn't yours or was deleted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/talk", skip(state, user, body))]
pub async fn talk(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<TalkRequest>,
) -> Result<Json<TalkAnswer>, ApiError> {
    let sub = user.sub()?;
    let question = body.question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ApiError::bad_request(format!(
            "A question to a bot has 1 to {MAX_QUESTION_CHARS} characters; this one has {}",
            question.chars().count()
        )));
    }
    if body.run_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "Asking a bot needs the run_id of the asking run",
        ));
    }
    if body.callee_id == bot_id {
        return Err(ApiError::bad_request("A bot can't ask itself"));
    }
    let caller = crew_member(
        &owned_bot(&state.db, &sub, &bot_id).await?,
        BotPresence::Online,
    );
    let callee_row = owned_bot(&state.db, &sub, &body.callee_id).await?;
    let presence = roster_in(&state.db, &sub, &bot_id, now_millis())
        .await?
        .into_iter()
        .find(|member| member.id == callee_row.id)
        .map_or(BotPresence::Unknown, |member| member.presence);
    let callee = crew_member(&callee_row, presence);
    let host = ServerTalkHost::for_chat(state.clone(), &sub, body.run_id.trim());
    let asked = host
        .ask_for_desktop(&caller, &callee, question)
        .await
        .map_err(ApiError::internal_error)?;
    let (status, text) = match asked.outcome {
        Ok(text) => (TalkStatus::Answered, text),
        Err(error) => match error.downcast_ref::<PendingAnswer>() {
            Some(pending) => (TalkStatus::Pending, pending.0.clone()),
            None => (TalkStatus::Failed, format!("{error:#}")),
        },
    };
    Ok(Json(TalkAnswer {
        status,
        text,
        exchange_id: asked.exchange_id,
        ask_id: asked.ask_id,
        reply_id: asked.reply_id,
    }))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/turns/{turn_id}/ask-result",
    tag = "Bots",
    description = "For a turn that was woken by the late answer to a question its bot asked: the question and the answer. The turn's host replies in `origin_conversation`. 404 when the turn is not such a turn.",
    params(
        ("bot_id" = String, Path, description = "The bot; yours"),
        ("turn_id" = String, Path, description = "The turn")
    ),
    responses(
        (status = 200, description = "The answered question", body = AskResult),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The turn isn't yours or doesn't wait on an answer")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /bots/{bot_id}/turns/{turn_id}/ask-result",
    skip(state, user)
)]
pub async fn turn_ask_result(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
) -> Result<Json<AskResult>, ApiError> {
    let names = bot_caller::Names {
        as_bot: Some(&bot_id),
        turn: Some(&turn_id),
        ..Default::default()
    };
    let sub = bot_caller::acting_owner(&state.db, &user, &names).await?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let turn = bot_turn::Entity::find_by_id(&turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id.as_str()))
        .filter(bot_turn::Column::OwnerId.eq(sub.as_str()))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Turn '{turn_id}' not found")))?;
    let AnsweredAsk {
        origin_conversation,
        question,
        callee_id,
        answer,
        failed,
    } = answered_ask(&state.db, &turn)
        .await?
        .ok_or_else(|| ApiError::not_found("This turn does not wait on an answer"))?;
    let callee_name = bot_name(&state.db, &callee_id).await?;
    Ok(Json(AskResult {
        origin_conversation,
        callee_id,
        callee_name,
        question,
        answer,
        failed,
    }))
}
