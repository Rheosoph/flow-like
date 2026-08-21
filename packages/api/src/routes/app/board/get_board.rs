use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        board::secrets::filter_board_secrets,
        template::get_template::VersionQuery,
        wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::board::Board;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format (e.g., 1_0_3)")
    ),
    responses(
        (status = 200, description = "Board details", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}",
    skip(state, user, params)
)]
pub async fn get_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<Board>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let version_opt = super::sync_board::parse_version(params.version.as_deref())?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, version_opt)
        .await?;

    let builtin_nodes = state.registry.as_ref().get_nodes_shared();
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes);

    filter_board_secrets(&mut board);

    Ok(Json(board))
}
