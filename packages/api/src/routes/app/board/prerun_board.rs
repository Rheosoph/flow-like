//! Pre-run analysis endpoint for boards
//!
//! Returns information needed before executing a board:
//! - Runtime-configured variables that need values
//! - Required OAuth providers and scopes
//!
//! Frontends are expected to cache responses and revalidate in the
//! background using the `signature` field to detect drift.

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::prerun_shared::{
        OAuthRequirement, PrerunPayload, RuntimeVariable, load_prerun_manifest, parse_version,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::{board::ExecutionMode, node::NodePermission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for pre-run analysis
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct PrerunBoardQuery {
    /// Board version as tuple (major, minor, patch) - defaults to latest
    pub version: Option<String>,
}

/// Response from pre-run analysis
#[derive(Debug, Serialize, ToSchema)]
pub struct PrerunBoardResponse {
    /// Variables that are marked as runtime_configured (need user-provided values)
    pub runtime_variables: Vec<RuntimeVariable>,
    /// OAuth providers required by nodes in this board
    pub oauth_requirements: Vec<OAuthRequirement>,
    /// Whether the board can only run locally (has offline-only nodes)
    pub requires_local_execution: bool,
    /// Board's execution mode setting (Hybrid, Remote, Local)
    #[schema(value_type = String)]
    pub execution_mode: ExecutionMode,
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

fn build_response(payload: PrerunPayload, can_execute_locally: bool) -> PrerunBoardResponse {
    PrerunBoardResponse {
        runtime_variables: payload.runtime_variables,
        oauth_requirements: payload.oauth_requirements,
        requires_local_execution: payload.requires_local_execution,
        execution_mode: payload.execution_mode,
        can_execute_locally,
        has_wasm_nodes: payload.has_wasm_nodes,
        wasm_package_ids: payload.wasm_package_ids,
        wasm_package_permissions: payload.wasm_package_permissions,
        signature: payload.signature,
    }
}

/// Analyze a board to determine what's needed before execution.
///
/// Returns runtime-configured variables and OAuth requirements.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/prerun",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        PrerunBoardQuery
    ),
    responses(
        (status = 200, description = "Pre-run analysis results", body = PrerunBoardResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/prerun",
    skip(state, user, query)
)]
pub async fn prerun_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<PrerunBoardQuery>,
) -> Result<Json<PrerunBoardResponse>, ApiError> {
    super::ensure_connected_app_board_invoke_denied(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteBoards);
    // Loading the board used to require a caller identity; principals without
    // one (API keys) stay rejected until that is decided on purpose.
    permission.sub()?;

    let can_execute_locally = permission.has_permission(RolePermissions::ReadBoards);
    let version = query.version.as_ref().and_then(|v| parse_version(v));

    let manifest = load_prerun_manifest(&state, &app_id, &board_id, version).await?;

    Ok(Json(build_response(
        PrerunPayload::from(&*manifest),
        can_execute_locally,
    )))
}
