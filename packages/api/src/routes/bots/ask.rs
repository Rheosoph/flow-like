//! Waking one of the caller's bots from a flow: the "Ask Bot" node posts here. The message
//! becomes a `trigger` entry (`source: event`) in the bot's conversation with its owner and the
//! call optionally waits for the reply. Only the bot's owner may ask: a person for themselves, a
//! flow run only for the person it runs as.

use super::{
    fire::{FiredTurn, fire_trigger, wait_for_turn},
    owned_bot,
};
use crate::{
    error::ApiError,
    middleware::jwt::{AppUser, ExecutorUser},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::inbox::{TRIGGER_TEXT_MAX_CHARS, TriggerBody, TriggerSource};
use serde::Deserialize;
use std::time::Duration;
use utoipa::ToSchema;

pub const MAX_ASK_WAIT_SECS: u64 = 100;
const FROM_A_FLOW: &str = "From a flow";

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AskBotRequest {
    /// What the bot is asked, at most 8,000 characters.
    pub message: String,
    /// Seconds to wait for the reply, at most 100. 0 returns as soon as the turn is queued.
    #[serde(default)]
    pub wait_seconds: u64,
    /// Asking again with the same id returns the first turn instead of waking the bot twice.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExecutorAskBotRequest {
    /// One of the bots of the person the run acts as.
    pub bot_id: String,
    #[serde(flatten)]
    pub ask: AskBotRequest,
}

pub(crate) fn ask_message_id(owner_id: &str, bot_id: &str, request_id: Option<&str>) -> String {
    let unique = match request_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let digest = blake3::hash(format!("{owner_id}\0{bot_id}\0{unique}").as_bytes()).to_hex();
    format!("ask-{}", &digest[..32])
}

pub(crate) fn ask_trigger(message_id: &str, message: &str) -> Result<TriggerBody, ApiError> {
    let text = message.trim();
    if text.is_empty() {
        return Err(ApiError::bad_request("Ask Bot needs a message"));
    }
    if text.chars().count() > TRIGGER_TEXT_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "A message to a bot has at most {TRIGGER_TEXT_MAX_CHARS} characters; this one has {}",
            text.chars().count()
        )));
    }
    Ok(TriggerBody {
        monitoring: false,
        source: TriggerSource::Event,
        ref_id: message_id.to_string(),
        title: FROM_A_FLOW.to_string(),
        text: text.to_string(),
        payload: None,
        payload_handle: None,
        bot_id: None,
    })
}

pub(crate) async fn ask_in(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    request: &AskBotRequest,
) -> Result<FiredTurn, ApiError> {
    owned_bot(&state.db, owner_id, bot_id).await?;
    let message_id = ask_message_id(owner_id, bot_id, request.request_id.as_deref());
    let trigger = ask_trigger(&message_id, &request.message)?;
    let turn = fire_trigger(state, owner_id, bot_id, &message_id, &trigger).await?;
    let wait = request.wait_seconds.min(MAX_ASK_WAIT_SECS);
    Ok(wait_for_turn(&state.db, owner_id, &turn, Duration::from_secs(wait)).await?)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/ask",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = AskBotRequest,
    responses(
        (status = 200, description = "The turn the message queued, with the bot's reply when you waited and it answered in time.", body = FiredTurn),
        (status = 400, description = "The message is empty or too long"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_TURN_NOT_QUEUED or BOT_CONVERSATION_ENCRYPTED")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/ask", skip(state, user, body))]
pub async fn ask_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<AskBotRequest>,
) -> Result<Json<FiredTurn>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(ask_in(&state, &sub, &bot_id, &body).await?))
}

/// The person a run acts as, if the run may ask bots for them.
pub(crate) fn run_owner(executor: &ExecutorUser) -> Result<&str, ApiError> {
    if executor.shadow {
        return Err(crate::middleware::jwt::shadow_side_effect_refused(
            "ask a bot",
        ));
    }
    if executor.sub.starts_with("sink:") || executor.hosted_frontend {
        return Err(ApiError::forbidden(
            "Only a run that acts as a person can ask that person's bots",
        ));
    }
    Ok(&executor.sub)
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/bots/ask",
    tag = "execution",
    description = "Ask one of the bots of the person this run acts as. Executor only; same contract as /bots/{bot_id}/ask. Shadow and regression runs are refused.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = ExecutorAskBotRequest,
    responses(
        (status = 200, description = "The turn the message queued, with the reply when waited for", body = FiredTurn),
        (status = 403, description = "The caller is not a run of this app that acts as a person, or SHADOW_SIDE_EFFECT_REFUSED"),
        (status = 404, description = "The person has no bot with this id")
    ),
    security(("executor_jwt" = []))
)]
pub async fn executor_ask_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<ExecutorAskBotRequest>,
) -> Result<Json<FiredTurn>, ApiError> {
    let AppUser::Executor(executor) = &user else {
        return Err(ApiError::FORBIDDEN);
    };
    if executor.app_id != app_id {
        return Err(ApiError::forbidden(
            "The executor token belongs to a different app",
        ));
    }
    let owner = run_owner(executor)?.to_string();
    Ok(Json(ask_in(&state, &owner, &body.bot_id, &body.ask).await?))
}
