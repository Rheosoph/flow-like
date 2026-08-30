use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};

/// Available board versions, each as (major, minor, patch).
pub type BoardVersionList = Vec<(u32, u32, u32)>;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/version",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "List of board versions as (major, minor, patch) tuples", body = Vec<(u32, u32, u32)>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/version",
    skip(state, user)
)]
pub async fn get_board_versions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<BoardVersionList>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    let versions = board.get_versions(None).await?;

    Ok(Json(versions))
}
