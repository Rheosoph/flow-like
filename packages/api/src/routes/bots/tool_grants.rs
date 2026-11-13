//! A bot's standing tool approvals and the owner's answers to the approval cards of parked
//! turns. A grant says the bot may run a tool for a scope without asking again, on every host;
//! an answer is stored with the turn and queues it again. Only the owner reads or changes any of
//! it, and every query is scoped to the owner in SQL.

use super::owned_bot;
use crate::{
    audit, bot_caller,
    bot_host::{
        approvals::{self, GrantRows},
        ledger,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        bots::turns::{BotTurn, TurnHostRequest, checked_host_id},
        inbox::{now_millis, wake},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use flow_like::flow::copilot::tool_grants::GrantStore;
use flow_like_types::{
    bot_message::{ApprovalRequest, ItemDecision},
    tool_grant::{ApprovalAnswer, ApprovalDecision, ToolGrantKey},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

const MAX_TOUCHED_GRANTS: usize = 50;

/// One standing approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotToolGrant {
    /// The tool, e.g. `code_run`.
    pub tool: String,
    /// `*`, `documents.expensive`, or `app:<id>`, `host:<h>`, `connection:<key>`, `table:<name>`,
    /// `target:<app>:event|page:<id>`.
    pub scope: String,
    pub granted_at: String,
    /// The person who granted it.
    pub granted_by: String,
    /// When the grant lapses: 90 days after the bot last used it. Each use moves it forward,
    /// at most once a day; a lapsed grant is ignored and removed.
    pub expires_at: Option<String>,
}

impl From<flow_like_types::tool_grant::ToolGrant> for BotToolGrant {
    fn from(grant: flow_like_types::tool_grant::ToolGrant) -> Self {
        Self {
            tool: grant.tool,
            scope: grant.scope,
            granted_at: grant.granted_at,
            granted_by: grant.granted_by,
            expires_at: grant.expires_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotToolGrantList {
    /// Oldest first.
    pub grants: Vec<BotToolGrant>,
}

/// A grant to store or take back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct BotToolGrantKey {
    #[schema(example = "code_run")]
    pub tool: String,
    #[schema(example = "host:api.example.com")]
    pub scope: String,
}

impl From<BotToolGrantKey> for ToolGrantKey {
    fn from(key: BotToolGrantKey) -> Self {
        Self {
            tool: key.tool,
            scope: key.scope,
        }
    }
}

/// The grants a host's bot just used.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct BotToolGrantTouch {
    /// At most 50 grants; ones the bot doesn't hold (or that lapsed) are ignored.
    pub grants: Vec<BotToolGrantKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotToolGrantRemoved {
    /// False when the bot had no such grant.
    pub deleted: bool,
}

/// The owner's answer to an approval card.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, ToSchema)]
pub struct BotApprovalAnswer {
    /// The card's id (`ap-…`), also its message id and the turn's `reply_message_id` while waiting.
    pub approval_id: String,
    /// `once` runs this call, `always` also stores a grant, `deny` refuses it. On a card with
    /// several calls, the answer for every call not listed in `items`.
    #[schema(value_type = String, example = "always")]
    pub decision: ApprovalDecision,
    /// Calls of the card answered differently: `{ "args_hash": "…", "decision": "deny" }` each.
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub items: Vec<ItemDecision>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct GrantsQuery {
    /// Hosts only: include the short-lived `once:` and `deny:` answer tokens of parked turns.
    #[serde(default)]
    pub tokens: bool,
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/tool-grants",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id"), GrantsQuery),
    responses(
        (status = 200, description = "What the bot may run without asking you again, oldest first. Each grant carries `expires_at`: it lapses 90 days after the bot last used it, and lapsed grants are left out.", body = BotToolGrantList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/tool-grants", skip(state, user))]
pub async fn list_tool_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Query(query): Query<GrantsQuery>,
) -> Result<Json<BotToolGrantList>, ApiError> {
    let sub = user.sub()?;
    let grants = approvals::list_grants(&state.db, &sub, &bot_id, query.tokens).await?;
    Ok(Json(BotToolGrantList {
        grants: grants.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/bots/{bot_id}/tool-grants",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotToolGrantKey,
    responses(
        (status = 200, description = "From now on the bot runs this tool for this scope without asking you, on every device. Granting again changes nothing.", body = BotToolGrant),
        (status = 400, description = "The tool or scope is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PUT /bots/{bot_id}/tool-grants", skip(state, user, body))]
pub async fn put_tool_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotToolGrantKey>,
) -> Result<Json<BotToolGrant>, ApiError> {
    let sub = user.sub()?;
    let key = ToolGrantKey::from(body);
    let now = now_millis();
    let held = approvals::list_grants(&state.db, &sub, &bot_id, true)
        .await?
        .iter()
        .any(|grant| grant.tool == key.tool && grant.scope == key.scope);
    let grant = approvals::grant_in(&state.db, &sub, &bot_id, &key, now).await?;
    if !held {
        audit!(
            state,
            user,
            "bot.tool_grant.put",
            "Bot",
            bot_id,
            serde_json::json!({ "tool": key.tool, "scope": key.scope })
        );
    }
    Ok(Json(grant.into()))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/tool-grants",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotToolGrantTouch,
    responses(
        (status = 204, description = "A host reports that the bot used these standing grants: their expiry moves 90 days past now, at most once a day each. Grants that don't exist or already lapsed stay as they are."),
        (status = 400, description = "More than 50 grants"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PATCH /bots/{bot_id}/tool-grants", skip(state, user, body))]
pub async fn touch_tool_grants(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotToolGrantTouch>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    if body.grants.len() > MAX_TOUCHED_GRANTS {
        return Err(ApiError::bad_request(format!(
            "At most {MAX_TOUCHED_GRANTS} grants per call; this one has {}",
            body.grants.len()
        )));
    }
    owned_bot(&state.db, &sub, &bot_id).await?;
    let keys: Vec<(String, String)> = body
        .grants
        .into_iter()
        .map(|key| (key.tool, key.scope))
        .collect();
    approvals::touch_in(&state.db, &bot_id, &keys, now_millis()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/tool-grants",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = BotToolGrantKey,
    responses(
        (status = 200, description = "The bot has to ask you again for this tool and scope (or never had permission).", body = BotToolGrantRemoved),
        (status = 400, description = "The tool or scope is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/tool-grants", skip(state, user, body))]
pub async fn delete_tool_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<BotToolGrantKey>,
) -> Result<Json<BotToolGrantRemoved>, ApiError> {
    let sub = user.sub()?;
    let key = ToolGrantKey::from(body);
    let deleted = approvals::revoke_in(&state.db, &sub, &bot_id, &key).await?;
    if deleted {
        audit!(
            state,
            user,
            "bot.tool_grant.revoke",
            "Bot",
            bot_id,
            serde_json::json!({ "tool": key.tool, "scope": key.scope })
        );
    }
    Ok(Json(BotToolGrantRemoved { deleted }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/turns/{turn_id}/approval",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("turn_id" = String, Path, description = "The waiting turn")
    ),
    request_body = BotApprovalAnswer,
    responses(
        (status = 200, description = "Your answer is recorded in the conversation and the turn is queued again: `once` lets this one call run, `always` also stores a grant, `deny` tells the bot you refused.", body = BotTurn),
        (status = 400, description = "BOT_APPROVAL_ITEM_UNKNOWN: `items` names a call that is not on the card, or one call twice"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or turn"),
        (status = 409, description = "BOT_TURN_NOT_WAITING: the turn is not waiting for an approval (answered already, cancelled or finished); BOT_APPROVAL_UNKNOWN: the turn waits for another card")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/turns/{turn_id}/approval",
    skip(state, user, body)
)]
pub async fn answer_approval(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, turn_id)): Path<(String, String)>,
    Json(body): Json<BotApprovalAnswer>,
) -> Result<Json<BotTurn>, ApiError> {
    let sub = user.sub()?;
    let answer = ApprovalAnswer {
        approval_id: body.approval_id,
        decision: body.decision,
        items: body.items,
    };
    let turn = approvals::answer(&state, &sub, &bot_id, &turn_id, &answer).await?;
    audit!(
        state,
        user,
        "bot.turn.approval",
        "Bot",
        bot_id,
        serde_json::json!({ "turn_id": turn.id, "decision": answer.decision.as_str() })
    );
    Ok(Json(ledger::turn_json(&turn, Vec::new()).into()))
}

/// A desktop host parks the turn it holds behind an approval card.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ParkTurnRequest {
    /// The host holding the turn's lease.
    pub host_id: String,
    /// The card to post; `id` is `ap-` and a hash of the turn and the call.
    #[schema(value_type = Object)]
    pub approval: ApprovalRequest,
}

#[utoipa::path(
    post,
    path = "/bots/turns/{turn_id}/park",
    tag = "Bots",
    params(("turn_id" = String, Path, description = "The turn you claimed")),
    request_body = ParkTurnRequest,
    responses(
        (status = 200, description = "The bot posted the approval card in its conversation with you and the turn waits (`waiting`, no lease) until you answer. Stop working on it.", body = BotTurn),
        (status = 400, description = "The card or host_id is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such turn"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the turn isn't running under this host; BOT_APPROVAL_UNREACHABLE: your conversation with the bot is end-to-end encrypted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/turns/{turn_id}/park", skip(state, user, body))]
pub async fn park_turn(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
    Json(body): Json<ParkTurnRequest>,
) -> Result<Json<BotTurn>, ApiError> {
    let names = bot_caller::Names {
        turn: Some(&turn_id),
        ..Default::default()
    };
    let sub = bot_caller::acting_owner(&state.db, &user, &names).await?;
    let host_id = checked_host_id(TurnHostRequest {
        host_id: body.host_id,
    })?;
    let turn = ledger::find(&state.db, &turn_id, Some(&sub))
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    owned_bot(&state.db, &sub, &turn.bot_id).await?;
    let card = body.approval;
    flow_like_types::bot_message::MessageBody {
        approval: Some(card.clone()),
        ..Default::default()
    }
    .check()
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if card.turn_id != turn.id {
        return Err(ApiError::bad_request("approval.turn_id must be this turn"));
    }
    approvals::park_held(&state, &turn, &host_id, &card).await?;
    wake(&state, [sub.clone()]).await;
    let parked = ledger::find(&state.db, &turn_id, Some(&sub))
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    Ok(Json(ledger::turn_json(&parked, Vec::new()).into()))
}

pub(crate) fn rows_for_owner(
    state: &AppState,
    bot_id: &str,
    owner_id: &str,
) -> Arc<dyn GrantStore> {
    Arc::new(GrantRows::new(Arc::new(state.db.clone()), bot_id, owner_id))
}
