use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::app::prerun_shared::parse_version,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct GetExecutionElementsQuery {
    pub page_id: String,
    #[serde(default)]
    pub wildcard: bool,
    /// Board version in MAJOR_MINOR_PATCH format. Omitted means latest.
    pub version: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct GetExecutionElementsResponse {
    #[schema(value_type = Object)]
    pub elements: HashMap<String, Value>,
}

/// Gets the elements required for executing a workflow on a specific page.
///
/// This returns only the elements that are referenced by nodes in the board,
/// along with their children. Use `wildcard: true` to get all elements.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/elements",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        GetExecutionElementsQuery
    ),
    responses(
        (status = 200, description = "Execution elements for the page", body = GetExecutionElementsResponse),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/elements",
    skip(state, user, query)
)]
pub async fn get_execution_elements(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(query): Query<GetExecutionElementsQuery>,
) -> Result<Json<GetExecutionElementsResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteBoards);
    let sub = permission.sub()?;

    let version = query.version.as_deref().and_then(parse_version);
    let board = state
        .master_board(&sub, &app_id, &board_id, &state, version)
        .await?;

    let elements = board
        .get_execution_elements(&query.page_id, query.wildcard, None)
        .await?;

    Ok(Json(GetExecutionElementsResponse { elements }))
}
