use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::VersionType;
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Clone, Deserialize, IntoParams)]
pub struct CreateVersionQuery {
    #[param(value_type = Option<String>)]
    pub version_type: Option<VersionType>,
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        CreateVersionQuery
    ),
    responses(
        (status = 200, description = "New version created as (major, minor, patch) tuple", body = (u32, u32, u32)),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn version_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<CreateVersionQuery>,
) -> Result<Json<(u32, u32, u32)>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    let version = board
        .create_version(params.version_type.unwrap_or(VersionType::Patch), None)
        .await?;

    audit_branch!(state, user, app_id, "board.version", "board", board_id, format!("Board versioned to {}.{}.{}", version.0, version.1, version.2));
    Ok(Json(version))
}
