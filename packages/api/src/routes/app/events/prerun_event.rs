//! Pre-run analysis endpoint for events
//!
//! Returns information needed before executing an event. Frontends are
//! expected to cache responses and revalidate in the background using the
//! `signature` field to detect drift.

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{
        OAuthRequirement, PrerunPayload, RuntimeVariable, compute_payload, parse_version,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::{board::ExecutionMode, event::EventExecutionMode, node::NodePermission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use super::db::get_event_from_db;

/// Query parameters for pre-run analysis
#[derive(Debug, Deserialize, ToSchema)]
pub struct PrerunEventQuery {
    /// Board version as tuple (major, minor, patch) - defaults to latest
    pub version: Option<String>,
}

/// Response from pre-run analysis
#[derive(Debug, Serialize, ToSchema)]
pub struct PrerunEventResponse {
    /// ID of the board this event triggers
    pub board_id: String,
    /// Variables that are marked as runtime_configured (need user-provided values)
    pub runtime_variables: Vec<RuntimeVariable>,
    /// OAuth providers required by nodes in this board
    pub oauth_requirements: Vec<OAuthRequirement>,
    /// Whether the event can only run locally (has offline-only nodes)
    pub requires_local_execution: bool,
    /// Board's execution mode setting (Hybrid, Remote, Local)
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
    /// Event's execution mode — where this specific event runs.
    /// An event is always either Local or Remote (never Hybrid), and must
    /// match the board when the board is not Hybrid.
    #[schema(value_type = String)]
    pub event_execution_mode: EventExecutionMode,
    /// Whether the user can execute locally (has ReadBoards permission)
    /// If false, execution must happen on server
    pub can_execute_locally: bool,
    /// Whether the board contains any WASM (external) nodes
    pub has_wasm_nodes: bool,
    /// package_id values of all WASM nodes present in the board
    pub wasm_package_ids: Vec<String>,
    /// Per-package permissions declared by WASM nodes (package_id -> list of permissions)
    pub wasm_package_permissions: HashMap<String, Vec<NodePermission>>,
    /// Stable hash over the board-derived fields. Frontends may cache the
    /// response and revalidate in the background; a changed signature
    /// signals the underlying board has shifted.
    pub signature: String,
}

fn build_response(
    board_id: String,
    payload: &PrerunPayload,
    event_execution_mode: EventExecutionMode,
    can_execute_locally: bool,
) -> PrerunEventResponse {
    PrerunEventResponse {
        board_id,
        runtime_variables: payload.runtime_variables.clone(),
        oauth_requirements: payload.oauth_requirements.clone(),
        requires_local_execution: payload.requires_local_execution,
        execution_mode: payload.execution_mode.clone(),
        event_execution_mode,
        can_execute_locally,
        has_wasm_nodes: payload.has_wasm_nodes,
        wasm_package_ids: payload.wasm_package_ids.clone(),
        wasm_package_permissions: payload.wasm_package_permissions.clone(),
        signature: payload.signature.clone(),
    }
}

/// Analyze an event to determine what's needed before execution.
///
/// Returns runtime-configured variables and OAuth requirements from the event's board.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/prerun",
    tag = "events",
    description = "Get pre-run requirements for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Pre-run requirements", body = PrerunEventResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/prerun",
    skip(state, user)
)]
pub async fn prerun_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<PrerunEventQuery>,
) -> Result<Json<PrerunEventResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards);
    let version = query.version.as_ref().and_then(|v| parse_version(v));

    let event = get_event_from_db(&state.db, &event_id, &app_id).await?;
    // Prerun discloses the full board/flow definition; hold connected apps to
    // the same surface policy as invoke (only directly-callable events, never
    // the REST/MCP events they must reach through the proxy).
    super::ensure_connected_app_direct_event_allowed(&user, &event.event_type, event.active)?;
    let board_id = event.board_id.clone();
    let event_execution_mode = event.execution_mode;

    let board = state
        .master_board(&sub, &app_id, &board_id, &state, version)
        .await?;
    let payload = compute_payload(&board);

    Ok(Json(build_response(
        board_id,
        &payload,
        event_execution_mode,
        can_execute_locally,
    )))
}
