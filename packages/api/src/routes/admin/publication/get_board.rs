use crate::{
    error::ApiError, middleware::jwt::AppUser, permission::global_permission::GlobalPermission,
    routes::app::board::secrets::filter_board_secrets, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::Board;

#[utoipa::path(
    get,
    path = "/admin/publication/apps/{app_id}/board/{board_id}",
    tag = "admin",
    description = "Get full board data for admin review. Board is returned read-only with secrets stripped.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "Full board data", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /admin/publication/apps/{app_id}/board/{board_id}",
    skip(state, user)
)]
pub async fn get_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<Board>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let mut board = state
        .master_board("admin", &app_id, &board_id, &state, None)
        .await?;

    filter_board_secrets(&mut board);

    Ok(Json(board))
}
