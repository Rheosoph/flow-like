//! Durable state is accessible only to the current desktop claim or foreground hold.

use crate::{
    bot_host::{ledger, recovery},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::bot_turn::{BotRunCheckpoint, TurnHostKind};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CheckpointQuery {
    pub host_id: String,
    pub attempt: u32,
    /// Read only the prior successful observation for this monitoring instruction.
    #[serde(default)]
    pub monitor_baseline: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct SaveCheckpoint {
    pub host_id: String,
    pub attempt: u32,
    #[schema(value_type = Object)]
    pub checkpoint: BotRunCheckpoint,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct CheckpointResponse {
    #[schema(value_type = Option<Object>)]
    pub checkpoint: Option<BotRunCheckpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor_result: Option<String>,
}

async fn claim(
    state: &AppState,
    owner_id: String,
    turn_id: String,
    host_id: String,
    attempt: u32,
) -> Result<recovery::Claim, ApiError> {
    let host_id = super::turns::checked_host_id(super::turns::TurnHostRequest { host_id })?;
    let turn = ledger::find(&state.db, &turn_id, Some(&owner_id))
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(&turn_id))?;
    let kind = TurnHostKind::parse(&turn.host_kind).unwrap_or(TurnHostKind::Server);
    let claim = recovery::Claim {
        turn_id,
        owner_id,
        host_id,
        attempt,
    };
    super::turns::user_claimable(kind, &claim.turn_id)?;
    Ok(claim)
}

#[utoipa::path(get, path = "/bots/turns/{turn_id}/checkpoint", tag = "Bots",
    params(("turn_id" = String, Path), CheckpointQuery),
    responses((status = 200, description = "The saved execution state for the current claim", body = CheckpointResponse),
        (status = 409, description = "The host lost its lease or foreground hold")),
    security(("bearer_auth" = [])))]
pub(crate) async fn load(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
    Query(query): Query<CheckpointQuery>,
) -> Result<Json<CheckpointResponse>, ApiError> {
    let claim = claim(&state, user.sub()?, turn_id, query.host_id, query.attempt).await?;
    if query.monitor_baseline {
        return Ok(Json(CheckpointResponse {
            checkpoint: None,
            monitor_result: recovery::previous_monitor_result_in(&state.db, &claim, now_millis())
                .await?,
        }));
    }
    Ok(Json(CheckpointResponse {
        checkpoint: recovery::load_in(&state.db, &claim, now_millis()).await?,
        monitor_result: None,
    }))
}

#[utoipa::path(put, path = "/bots/turns/{turn_id}/checkpoint", tag = "Bots",
    params(("turn_id" = String, Path)), request_body = SaveCheckpoint,
    responses((status = 200, description = "The checkpoint was saved"),
        (status = 409, description = "The claim or checkpoint revision changed")),
    security(("bearer_auth" = [])))]
pub(crate) async fn save(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(turn_id): Path<String>,
    Json(body): Json<SaveCheckpoint>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claim = claim(&state, user.sub()?, turn_id, body.host_id, body.attempt).await?;
    recovery::save(&state.db, state.db_dialect, &claim, &body.checkpoint).await?;
    Ok(Json(serde_json::json!({"saved": true})))
}
