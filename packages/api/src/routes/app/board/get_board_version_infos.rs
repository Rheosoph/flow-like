use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::BoardVersionInfo;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/version/info",
    tag = "boards",
    description = "Lists the published versions of a board, newest first, with when each was published and by whom when that was recorded.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "Published versions, newest first", body = Vec<BoardVersionInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/version/info",
    skip(state, user)
)]
pub async fn get_board_version_infos(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<Vec<BoardVersionInfo>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    let versions = board.get_version_infos(None).await?;

    Ok(Json(versions))
}
