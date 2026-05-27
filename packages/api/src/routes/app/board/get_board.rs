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
use flow_like_types::anyhow;

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
#[tracing::instrument(name = "GET /apps/{app_id}/board/{board_id}", skip(state, user))]
pub async fn get_board(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<Board>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let version_opt = if let Some(ver_str) = params.version {
        let parts = ver_str
            .split('_')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<u32>, _>>()?;
        match parts.as_slice() {
            [maj, min, pat] => Some((*maj, *min, *pat)),
            _ => {
                return Err(ApiError::internal_error(anyhow!(
                    "version must be in MAJOR_MINOR_PATCH format"
                )));
            }
        }
    } else {
        None
    };

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, version_opt)
        .await?;

    let builtin_nodes = state.registry.as_ref().get_nodes();
    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes);

    filter_board_secrets(&mut board);

    Ok(Json(board))
}
