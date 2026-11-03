//! The bot-turn ledger over HTTP, for hosts that answer for their owner (the desktop app, and
//! devices through their own routes): list the queued turns, claim one with a lease, keep the
//! lease with heartbeats, then answer with `POST /inbox/conversations/{id}/entries?as=<bot>` and
//! `turn { id, host_id }`. Owners retry failed turns and anyone in the conversation reads a
//! turn's state. Every query is scoped to the caller in SQL.

use super::owned_bot;
use crate::{
    audit,
    bot_host::ledger::{self, BOT_TURN_HOST_KIND, ClaimRequest},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{now_millis, wake},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use flow_like_types::bot_turn::{self as contract, TurnHostKind};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

const MAX_HOST_ID_CHARS: usize = 128;

/// A bot's turn: one message that woke the bot and what became of it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotTurn {
    /// `bt_` and a hash of the bot and the message that woke it.
    #[schema(example = "bt_0c3f7a9e1d5b48a2f6e0b9d1c4a7e382")]
    pub id: String,
    pub bot_id: String,
    pub conversation_id: String,
    /// The message that woke the bot: what the turn answers.
    pub trigger_message_id: String,
    /// Where the turn runs: `server`, `desktop` or `device`.
    #[schema(value_type = String, example = "desktop")]
    pub host_kind: TurnHostKind,
    /// `queued`, `running`, `done`, `failed` or `cancelled`.
    #[schema(value_type = String, example = "running")]
    pub status: contract::TurnStatus,
    /// Claims so far, takeovers of an expired lease included.
    pub attempts: u32,
    /// The host holding the lease.
    pub host_id: Option<String>,
    /// A running turn belongs to its host until then.
    pub lease_until: Option<String>,
    /// The bot's answer, once there is one.
    pub reply_message_id: Option<String>,
    /// Why the turn failed or was dropped, e.g. `BOT_TURN_TIMEOUT`; `BATCHED` marks a turn a
    /// later one answered together with it (`reply_message_id` names the answer).
    pub error_code: Option<String>,
    /// Bot turns that led to the trigger message; at most 3 queue another.
    pub hops: u32,
    pub created_at: String,
    pub updated_at: String,
    /// Older messages whose turns this one answers too. Set on a claim and on a listing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub batched: Vec<String>,
}

impl From<contract::BotTurn> for BotTurn {
    fn from(turn: contract::BotTurn) -> Self {
        Self {
            id: turn.id,
            bot_id: turn.bot_id,
            conversation_id: turn.conversation_id,
            trigger_message_id: turn.trigger_message_id,
            host_kind: turn.host_kind,
            status: turn.status,
            attempts: turn.attempts,
            host_id: turn.host_id,
            lease_until: turn.lease_until,
            reply_message_id: turn.reply_message_id,
            error_code: turn.error_code,
            hops: turn.hops,
            created_at: turn.created_at,
            updated_at: turn.updated_at,
            batched: turn.batched,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotTurnList {
    /// Oldest first; one per bot and conversation, the newest message's turn.
    pub turns: Vec<BotTurn>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTurnsQuery {
    /// Which host asks: `desktop` or `device` (`server` turns are the server's own).
    #[param(example = "desktop")]
    pub host_kind: String,
}

/// The host that claims or keeps a turn.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TurnHostRequest {
    /// Names this host in `turn { id, host_id }` when it answers. 1 to 128 characters without
    /// spaces; one per running app instance.
    #[schema(example = "desktop:7f3c1a")]
    pub host_id: String,
}

pub(super) fn checked_host_id(request: TurnHostRequest) -> Result<String, ApiError> {
    let host_id = request.host_id.trim().to_string();
    let valid = (1..=MAX_HOST_ID_CHARS).contains(&host_id.chars().count())
        && !host_id
            .chars()
            .any(|char| char.is_whitespace() || char.is_control());
    if valid {
        Ok(host_id)
    } else {
        Err(ApiError::bad_request(format!(
            "host_id is 1 to {MAX_HOST_ID_CHARS} characters without spaces"
        )))
    }
}

fn turn_response(turn: &crate::entity::bot_turn::Model, batched: Vec<String>) -> Json<BotTurn> {
    Json(ledger::turn_json(turn, batched).into())
}

#[utoipa::path(
    get,
    path = "/bots/turns",
    tag = "Bots",
    params(ListTurnsQuery),
    responses(
        (status = 200, description = "Your bots' queued turns for this kind of host, oldest first. When several messages are waiting for one bot in one conversation, only the newest one's turn is listed and `batched` names the older messages it answers too. Turns of deleted bots, turns still retrying and conversations whose bot already has a turn running are left out.", body = BotTurnList),
        (status = 400, description = "host_kind is not server, desktop or device"),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/turns", skip(state, user))]
pub async fn list_turns(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListTurnsQuery>,
) -> Result<Json<BotTurnList>, ApiError> {
    let sub = user.sub()?;
    let kind = TurnHostKind::parse(&query.host_kind)
        .ok_or_else(|| ApiError::bad_request("host_kind is one of server, desktop or device"))?;
    let heads = ledger::queued_heads(
        &state.db,
        Some(&sub),
        kind,
        now_millis(),
        ledger::LIST_LIMIT,
    )
    .await?;
    Ok(Json(BotTurnList {
        turns: heads
            .iter()
            .map(|(turn, batched)| ledger::turn_json(turn, batched.clone()).into())
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/bots/turns/{turn_id}/claim",
    tag = "Bots",
    params(("turn_id" = String, Path, description = "The turn from your listing")),
    request_body = TurnHostRequest,
    responses(
        (status = 200, description = "The turn is yours for 60 seconds: answer it before then and send a heartbeat every 15 seconds meanwhile. It is the newest queued turn of the bot in this conversation, which may be a newer one than you asked for when more messages arrived; the older ones are listed in `batched` and are answered by your reply. Claiming again with the same host_id returns the turn as it is.", body = BotTurn),
        (status = 400, description = "host_id is empty, too long or has spaces"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such turn, or its bot was deleted (the turn is cancelled)"),
        (status = 409, description = "BOT_TURN_NOT_QUEUED: another host holds it or it is finished; BOT_TURN_BUSY: another turn of this bot is running in this conversation; BOT_TURN_OWNER_BUSY: you already have 3 turns running; BOT_TURN_HOST_KIND: the turn belongs to the server, or its bot moved to another kind of host")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/turns/{turn_id}/claim", skip(state, user, body))]
pub async fn claim_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
    Json(body): Json<TurnHostRequest>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let host_id = checked_host_id(body)?;
    let turn = ledger::find(&state.db, &turn_id, Some(&sub))
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    let kind = TurnHostKind::parse(&turn.host_kind).unwrap_or(TurnHostKind::Server);
    if kind == TurnHostKind::Server {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_TURN_HOST_KIND,
            format!("Turn '{turn_id}' belongs to the server, which answers it itself"),
        ));
    }
    let claimed = ledger::claim(
        &state,
        ClaimRequest {
            turn_id: turn_id.clone(),
            owner_id: Some(sub.clone()),
            host_kind: kind,
            host_id,
            now: now_millis(),
        },
    )
    .await?
    .map_err(|refusal| refusal.into_api(&turn_id))?;
    wake(&state, [sub]).await;
    Ok(turn_response(&claimed.turn, claimed.batched))
}

#[utoipa::path(
    post,
    path = "/bots/turns/{turn_id}/heartbeat",
    tag = "Bots",
    params(("turn_id" = String, Path, description = "The turn you claimed")),
    request_body = TurnHostRequest,
    responses(
        (status = 200, description = "The lease runs 60 seconds from now.", body = BotTurn),
        (status = 400, description = "host_id is empty, too long or has spaces"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such turn"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the turn isn't running under this host any more (it finished, was cancelled, or its lease ran out); stop working on it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/turns/{turn_id}/heartbeat", skip(state, user, body))]
pub async fn heartbeat_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
    Json(body): Json<TurnHostRequest>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let host_id = checked_host_id(body)?;
    let turn =
        ledger::heartbeat_in(&state.db, &turn_id, Some(&sub), &host_id, now_millis()).await?;
    Ok(turn_response(&turn, Vec::new()))
}

/// Whether the interactive chat still holds the turn of a message it sent.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TurnHold {
    /// `false`: a host claimed the turn or it finished; the chat must not write a reply.
    pub held: bool,
}

#[utoipa::path(
    post,
    path = "/bots/turns/{turn_id}/hold",
    tag = "Bots",
    params(("turn_id" = String, Path, description = "The turn of a message your chat sent with an interactive mark")),
    responses(
        (status = 200, description = "Hosts leave the message to your chat for another two minutes.", body = TurnHold),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such turn")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/turns/{turn_id}/hold", skip(state, user))]
pub async fn hold_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
) -> Result<Json<TurnHold>, ApiError> {
    let sub = user.sub()?;
    let held = ledger::hold_in(&state.db, &turn_id, &sub, now_millis()).await?;
    Ok(Json(TurnHold { held }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/turns/{turn_id}/retry",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("turn_id" = String, Path, description = "A failed turn of this bot")
    ),
    responses(
        (status = 200, description = "The turn is queued again with a fresh set of attempts; its host picks it up.", body = BotTurn),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or turn"),
        (status = 409, description = "BOT_TURN_NOT_RETRYABLE: only a failed turn can be retried")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/turns/{turn_id}/retry", skip(state, user))]
pub async fn retry_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let turn = ledger::retry_in(&state.db, &sub, &bot.id, &turn_id, now_millis())
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    crate::bot_host::dispatch_turns(&state, vec![turn.id.clone()]).await;
    wake(&state, [sub]).await;
    Ok(turn_response(&turn, Vec::new()))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/turns/{turn_id}/cancel",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("turn_id" = String, Path, description = "A queued or running turn of this bot")
    ),
    responses(
        (status = 200, description = "The turn is cancelled. If a host was working on it, its next heartbeat gets BOT_TURN_LEASE_LOST and it stops.", body = BotTurn),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or turn"),
        (status = 409, description = "BOT_TURN_NOT_CANCELLABLE: the turn already finished")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/turns/{turn_id}/cancel", skip(state, user))]
pub async fn cancel_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let turn = ledger::cancel_owned_in(&state.db, &sub, &bot.id, &turn_id, now_millis()).await?;
    audit!(
        state,
        user,
        "bot.turn.cancel",
        "Bot",
        bot.id,
        serde_json::json!({ "turn_id": turn.id })
    );
    wake(&state, [sub]).await;
    Ok(turn_response(&turn, Vec::new()))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/turns/{turn_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ("turn_id" = String, Path, description = "The turn")
    ),
    responses(
        (status = 200, description = "Where the turn stands: queued, running, done (with `reply_message_id`), failed (with `error_code`) or cancelled. Readable by the bot's owner and by the people in the conversation.", body = BotTurn),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No such turn in a conversation you are in, or one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/turns/{turn_id}",
    skip(state, user)
)]
pub async fn get_conversation_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, turn_id)): Path<(String, String)>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let turn = ledger::watched_turn(&state.db, &sub, &conversation_id, &turn_id)
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    Ok(turn_response(&turn, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_id_is_one_word_of_bounded_length() {
        let host = |id: &str| checked_host_id(TurnHostRequest { host_id: id.into() });
        assert_eq!(host(" desktop:1 ").unwrap(), "desktop:1");
        assert!(host("").is_err());
        assert!(host("two words").is_err());
        assert!(host(&"x".repeat(MAX_HOST_ID_CHARS + 1)).is_err());
        assert!(host(&"x".repeat(MAX_HOST_ID_CHARS)).is_ok());
    }

    #[test]
    fn the_response_serializes_like_the_contract_hosts_read() {
        let turn = contract::BotTurn {
            id: "bt_1".into(),
            bot_id: "bot_a".into(),
            conversation_id: "c1".into(),
            trigger_message_id: "m2".into(),
            host_kind: TurnHostKind::Desktop,
            status: contract::TurnStatus::Running,
            attempts: 1,
            host_id: Some("desktop:1".into()),
            lease_until: Some("2026-11-02T10:00:00+00:00".into()),
            reply_message_id: None,
            error_code: None,
            hops: 1,
            created_at: "2026-11-02T09:59:00+00:00".into(),
            updated_at: "2026-11-02T09:59:10+00:00".into(),
            batched: vec!["m1".into()],
        };
        let wire = serde_json::to_value(BotTurn::from(turn.clone())).unwrap();
        assert_eq!(
            serde_json::from_value::<contract::BotTurn>(wire).unwrap(),
            turn
        );
    }
}
